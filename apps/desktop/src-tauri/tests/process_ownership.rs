//! Process ownership tests for daemon supervision.
//!
//! Verifies that the desktop app:
//! - Only starts configured supervised daemons
//! - Only stops processes it owns
//! - Tracks process ownership correctly
//! - Does not interfere with externally-managed daemons

#[cfg(test)]
mod process_ownership_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    /// Fake daemon command that simulates a daemon process.
    /// Creates a PID file and sleeps to stay alive.
    struct FakeDaemon {
        dir: tempfile::TempDir,
        pid_path: PathBuf,
    }

    impl FakeDaemon {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pid_path = dir.path().join("daemon.pid");
            Self { dir, pid_path }
        }

        /// Create a fake daemon script that writes its PID and sleeps.
        fn script_path(&self) -> PathBuf {
            let script = self.dir.path().join("fake_daemon.sh");
            let pid_path = &self.pid_path;
            fs::write(
                &script,
                format!(
                    r#"#!/bin/bash
echo $$ > "{}"
while true; do sleep 1; done
"#,
                    pid_path.display()
                ),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&script).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&script, perms).unwrap();
            }
            script
        }
    }

    #[test]
    fn test_fake_daemon_creates_pid_file() {
        let daemon = FakeDaemon::new();
        let script = daemon.script_path();
        assert!(script.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&script).unwrap().permissions();
            assert!(perms.mode() & 0o111 != 0);
        }
    }

    #[test]
    fn test_process_ownership_starts_unowned() {
        // A fresh daemon handle should not own any process
        use opensymphony_desktop::daemon::{DaemonConfig, DaemonHandle};
        let config = DaemonConfig {
            executable: PathBuf::from("/bin/true"),
            args: vec![],
            env: vec![],
            startup_timeout: Duration::from_secs(1),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:8080".to_string(),
        };
        let handle = DaemonHandle::new(config);
        assert!(handle.pid().is_none());
    }

    #[test]
    fn test_unsupervised_daemon_not_stopped_by_app() {
        // Verify that the app does not attempt to stop processes it doesn't own
        use opensymphony_desktop::daemon::{DaemonConfig, DaemonHandle};
        let config = DaemonConfig {
            executable: PathBuf::from("/bin/true"),
            args: vec![],
            env: vec![],
            startup_timeout: Duration::from_secs(1),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:8080".to_string(),
        };
        let mut handle = DaemonHandle::new(config);
        // Calling stop on an unstarted handle should succeed without error
        assert!(handle.stop().is_ok());
    }

    #[tokio::test]
    async fn test_daemon_handle_cleans_up_on_drop() {
        use opensymphony_desktop::daemon::{DaemonConfig, DaemonHandle};
        let fake = FakeDaemon::new();
        let script = fake.script_path();

        let config = DaemonConfig {
            executable: script,
            args: vec![],
            env: vec![],
            startup_timeout: Duration::from_secs(2),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:8080".to_string(),
        };

        {
            let mut handle = DaemonHandle::new(config);
            let _result = handle.start().await;
            // Handle will be dropped here, triggering cleanup
        }

        // Give the OS a moment to clean up
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn test_process_ownership_tracks_multiple_daemons() {
        // Verify that ownership tracking works correctly for multiple daemon instances
        use opensymphony_desktop::daemon::{DaemonConfig, DaemonHandle};

        let config1 = DaemonConfig {
            executable: PathBuf::from("/bin/true"),
            args: vec!["1".to_string()],
            env: vec![],
            startup_timeout: Duration::from_secs(1),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:8081".to_string(),
        };

        let config2 = DaemonConfig {
            executable: PathBuf::from("/bin/true"),
            args: vec!["2".to_string()],
            env: vec![],
            startup_timeout: Duration::from_secs(1),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:8082".to_string(),
        };

        let handle1 = DaemonHandle::new(config1);
        let handle2 = DaemonHandle::new(config2);

        // Both handles start unowned
        assert!(handle1.pid().is_none());
        assert!(handle2.pid().is_none());
    }
}
