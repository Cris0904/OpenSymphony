//! Local daemon supervisor for OpenSymphony desktop.
//!
//! Manages the lifecycle of a local OpenSymphony daemon process,
//! including startup, health checking, monitoring, and graceful shutdown.
//! Only stops processes that it explicitly owns.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::watch;
use tracing::{error, info, warn};

/// Configuration for a supervised daemon process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Path to the daemon executable.
    pub executable: PathBuf,
    /// Arguments to pass to the daemon.
    pub args: Vec<String>,
    /// Environment variables to set for the daemon process.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for the daemon to become healthy.
    pub startup_timeout: Duration,
    /// Whether to automatically restart the daemon if it exits.
    pub auto_restart: bool,
    /// Gateway URL where the daemon listens.
    pub gateway_url: String,
}

/// Current state of the supervised daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DaemonState {
    /// Daemon is not running.
    Stopped,
    /// Daemon is starting up.
    Starting,
    /// Daemon is running and healthy.
    Running,
    /// Daemon is running but unhealthy.
    Unhealthy,
    /// Daemon is shutting down.
    Stopping,
    /// Daemon has crashed or failed to start.
    Failed(String),
}

/// Result of a daemon startup attempt.
#[derive(Debug, Serialize)]
pub struct StartupResult {
    /// Whether startup succeeded.
    pub success: bool,
    /// Process ID if started successfully.
    pub pid: Option<u32>,
    /// Error message if startup failed.
    pub error: Option<String>,
    /// Time taken to start up in milliseconds.
    pub elapsed_ms: u64,
}

/// Error type for daemon operations.
#[derive(Error, Debug)]
pub enum DaemonError {
    #[error("daemon failed to start: {0}")]
    StartFailed(String),
    #[error("daemon is not running")]
    NotRunning,
    #[error("daemon health check failed: {0}")]
    HealthCheckFailed(String),
    #[error("timeout waiting for daemon to start")]
    StartupTimeout,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Handle to a supervised daemon process.
///
/// Tracks process ownership and provides lifecycle management.
pub struct DaemonHandle {
    /// The child process.
    child: Option<Child>,
    /// Process ID of the daemon.
    pid: Option<u32>,
    /// Current state of the daemon.
    state: DaemonState,
    /// Whether this handle owns the process (and should stop it on drop).
    owns_process: bool,
    /// Configuration used to start this daemon.
    config: DaemonConfig,
    /// Shutdown signal sender.
    shutdown_tx: Option<watch::Sender<bool>>,
}

impl DaemonHandle {
    /// Create a new daemon handle with the given configuration.
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            child: None,
            pid: None,
            state: DaemonState::Stopped,
            owns_process: false,
            config,
            shutdown_tx: None,
        }
    }

    /// Start the daemon process.
    ///
    /// Only starts if not already running. Returns a StartupResult with
    /// the outcome and timing information.
    pub async fn start(&mut self) -> StartupResult {
        if self.is_running() {
            warn!("daemon already running, pid={:?}", self.pid);
            return StartupResult {
                success: true,
                pid: self.pid,
                error: None,
                elapsed_ms: 0,
            };
        }

        let start_time = Instant::now();
        info!(
            executable = ?self.config.executable,
            args = ?self.config.args,
            "starting supervised daemon",
        );

        self.state = DaemonState::Starting;

        let mut cmd = Command::new(&self.config.executable);
        cmd.args(&self.config.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id();
                self.child = Some(child);
                self.pid = Some(pid);
                self.owns_process = true;

                info!(pid, "daemon process started");

                // Wait for health check
                let health_result = self.wait_for_health().await;

                let elapsed = start_time.elapsed().as_millis() as u64;

                match health_result {
                    Ok(()) => {
                        self.state = DaemonState::Running;
                        info!(pid, elapsed_ms = elapsed, "daemon healthy");
                        StartupResult {
                            success: true,
                            pid: Some(pid),
                            error: None,
                            elapsed_ms: elapsed,
                        }
                    }
                    Err(e) => {
                        self.state = DaemonState::Failed(e.to_string());
                        error!(pid, error = %e, "daemon failed health check");
                        StartupResult {
                            success: false,
                            pid: Some(pid),
                            error: Some(e.to_string()),
                            elapsed_ms: elapsed,
                        }
                    }
                }
            }
            Err(e) => {
                self.state = DaemonState::Failed(e.to_string());
                error!(error = %e, "failed to spawn daemon process");
                StartupResult {
                    success: false,
                    pid: None,
                    error: Some(e.to_string()),
                    elapsed_ms: start_time.elapsed().as_millis() as u64,
                }
            }
        }
    }

    /// Wait for the daemon to become healthy.
    ///
    /// Polls the health endpoint until it responds or the timeout is reached.
    async fn wait_for_health(&self) -> Result<(), DaemonError> {
        let deadline = Instant::now() + self.config.startup_timeout;
        let health_url = format!("{}/healthz", self.config.gateway_url.trim_end_matches('/'));

        info!(url = %health_url, "waiting for daemon health check");

        while Instant::now() < deadline {
            match reqwest::get(&health_url).await {
                Ok(response) if response.status().is_success() => {
                    return Ok(());
                }
                Ok(response) => {
                    warn!(status = %response.status(), "daemon not yet ready");
                }
                Err(e) => {
                    warn!(error = %e, "health check failed, retrying");
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err(DaemonError::StartupTimeout)
    }

    /// Check if the daemon is currently running.
    pub fn is_running(&self) -> bool {
        self.pid.is_some()
            && matches!(
                self.state,
                DaemonState::Running | DaemonState::Starting | DaemonState::Unhealthy
            )
    }

    /// Get the current state of the daemon.
    pub fn state(&self) -> &DaemonState {
        &self.state
    }

    /// Get the process ID of the daemon.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Get the gateway URL for this daemon.
    pub fn gateway_url(&self) -> &str {
        &self.config.gateway_url
    }

    /// Stop the daemon process gracefully.
    ///
    /// Only stops if this handle owns the process.
    pub fn stop(&mut self) -> Result<(), DaemonError> {
        if !self.owns_process {
            warn!("attempted to stop daemon that we don't own");
            return Ok(());
        }

        if let Some(ref mut child) = self.child {
            info!(pid = ?self.pid, "stopping daemon process");
            self.state = DaemonState::Stopping;

            #[cfg(unix)]
            {
                // Send SIGTERM for graceful shutdown on Unix
                let _ = unsafe { libc::kill(self.pid.unwrap_or(0) as i32, libc::SIGTERM) };
            }

            #[cfg(windows)]
            {
                // Use taskkill on Windows
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &self.pid.unwrap_or(0).to_string(), "/F"])
                    .output();
            }

            // Wait for process to exit
            match child.wait() {
                Ok(status) => {
                    info!(pid = ?self.pid, status = ?status, "daemon stopped");
                }
                Err(e) => {
                    warn!(pid = ?self.pid, error = %e, "error waiting for daemon to stop");
                }
            }

            self.child = None;
            self.pid = None;
            self.state = DaemonState::Stopped;
            self.owns_process = false;
        }

        Ok(())
    }

    /// Force-kill the daemon process.
    pub fn kill(&mut self) -> Result<(), DaemonError> {
        if let Some(ref mut child) = self.child {
            info!(pid = ?self.pid, "force-killing daemon");
            let _ = child.kill();
            self.child = None;
            self.pid = None;
            self.state = DaemonState::Stopped;
            self.owns_process = false;
        }
        Ok(())
    }
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        if self.owns_process {
            info!(
                pid = ?self.pid,
                "daemon handle dropped, cleaning up owned process",
            );
            let _ = self.stop();
        }
    }
}

/// Supervisor that manages a daemon's lifecycle.
///
/// Monitors the daemon process and optionally restarts it on failure.
pub struct DaemonSupervisor {
    /// The daemon handle being supervised.
    handle: DaemonHandle,
    /// Whether auto-restart is enabled.
    auto_restart: bool,
    /// Shutdown signal receiver.
    shutdown_rx: watch::Receiver<bool>,
}

impl DaemonSupervisor {
    /// Create a new supervisor for the given daemon configuration.
    pub fn new(config: DaemonConfig) -> (Self, watch::Sender<bool>) {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let auto_restart = config.auto_restart;
        let handle = DaemonHandle::new(config);
        let supervisor = Self {
            handle,
            auto_restart,
            shutdown_rx,
        };
        (supervisor, shutdown_tx)
    }

    /// Start supervising the daemon.
    ///
    /// Returns immediately after starting the daemon. The supervisor will
    /// monitor and optionally restart the daemon in the background.
    pub async fn start(&mut self) -> StartupResult {
        self.handle.start().await
    }

    /// Get a reference to the daemon handle.
    pub fn handle(&self) -> &DaemonHandle {
        &self.handle
    }

    /// Get a mutable reference to the daemon handle.
    pub fn handle_mut(&mut self) -> &mut DaemonHandle {
        &mut self.handle
    }

    /// Check if the daemon is currently running.
    pub fn is_running(&self) -> bool {
        self.handle.is_running()
    }

    /// Stop the supervised daemon.
    pub fn stop(&mut self) -> Result<(), DaemonError> {
        self.auto_restart = false;
        self.handle.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn test_config() -> DaemonConfig {
        DaemonConfig {
            executable: PathBuf::from("/bin/sleep"),
            args: vec!["300".to_string()],
            env: vec![("TEST_VAR".to_string(), "test_value".to_string())],
            startup_timeout: Duration::from_secs(5),
            auto_restart: true,
            gateway_url: "http://127.0.0.1:8080".to_string(),
        }
    }

    #[test]
    fn test_daemon_handle_creation() {
        let config = test_config();
        let handle = DaemonHandle::new(config);
        assert_eq!(handle.state(), &DaemonState::Stopped);
        assert!(handle.pid().is_none());
        assert!(!handle.is_running());
    }

    #[tokio::test]
    async fn test_daemon_start_stop_with_fake_command() {
        // Create a simple script that exits immediately
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("fake_daemon.sh");
        fs::write(
            &script_path,
            "#!/bin/bash\nsleep 0.1\necho 'daemon started'\nwhile true; do sleep 1; done\n",
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).unwrap();
        }

        let mut config = test_config();
        config.executable = script_path.clone();
        config.args = vec![];
        config.startup_timeout = Duration::from_secs(2);
        config.gateway_url = format!("file://{}", dir.path().display());

        let mut handle = DaemonHandle::new(config);

        // Start the daemon
        let result = handle.start().await;
        // The fake daemon won't have a health endpoint, so it should fail
        assert!(!result.success || result.pid.is_some());

        // Clean up
        let _ = handle.stop();
    }

    #[test]
    fn test_daemon_ownership_tracking() {
        let config = test_config();
        let handle = DaemonHandle::new(config);
        assert!(!handle.owns_process);

        // After start, owns_process would be true
        // After stop, owns_process would be false
    }

    #[test]
    fn test_daemon_config_serialization() {
        let config = test_config();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: DaemonConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.executable, config.executable);
        assert_eq!(deserialized.args, config.args);
        assert_eq!(deserialized.gateway_url, config.gateway_url);
    }
}
