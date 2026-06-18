//! Static `opensymphony workspace clone` subcommand (LOC-15).
//!
//! `after_create` is the static string `opensymphony workspace clone`. The
//! workspace manager injects the resolved `RepoRef` into the hook subprocess
//! via env vars; this subcommand reads them, materializes the `git` argv, and
//! never invokes a shell. The contract:
//!
//! * `OPENSYMPHONY_EXECUTION_REPO_URL` (required): the resolved clone URL
//!   (e.g. `git@github.com:org/repo.git`). Used as the `<url>` argv of
//!   `git clone`.
//! * `OPENSYMPHONY_EXECUTION_REPO_KEY` (required): the resolved
//!   `RepoRef.key` — the canonical short identifier of the repo
//!   (e.g. `org/repo`). **Not** a credential/SSH key. The subcommand uses it
//!   for log correlation so operators can match the clone output to the
//!   `RepoRef` that was resolved upstream; it is also exposed via the env
//!   contract for future consumers (credential routing, per-repo workspace
//!   organization) without requiring another seam.
//! * `OPENSYMPHONY_EXECUTION_REPO_BRANCH` (optional): the default branch to
//!   clone. If absent (or blank), clone the remote default branch.
//! * Empty cwd → clone. Cwd already contains `.git` → succeed (idempotent).
//! * Cwd contains partial non-git contents → fail with a clear error so the
//!   existing retry-after-failed-bootstrap behavior remains deterministic.

use std::{
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
};

use serde::Serialize;
use thiserror::Error;
use tokio::process::Command;

use crate::opensymphony_workspace::{HOOK_REPO_BRANCH_ENV, HOOK_REPO_KEY_ENV, HOOK_REPO_URL_ENV};

#[derive(Debug, Error)]
pub enum WorkspaceCloneError {
    #[error(
        "missing required env var {env}: {hint}; the workspace manager must inject \
         OPENSYMPHONY_EXECUTION_REPO_URL, OPENSYMPHONY_EXECUTION_REPO_BRANCH (optional), \
         and OPENSYMPHONY_EXECUTION_REPO_KEY before invoking this subcommand"
    )]
    MissingRequiredEnv { env: &'static str, hint: String },
    #[error("workspace directory {path} contains non-git contents; refuse to clone")]
    PartialNonGitWorkspace { path: PathBuf },
    #[error("git clone failed: {detail}")]
    GitFailed { detail: String },
}

/// Output of [`ClonePlan::resolve`]. Public for testing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClonePlan {
    pub url: String,
    pub key: String,
    pub branch: Option<String>,
    pub cwd: PathBuf,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CloneInputs {
    pub url: Option<String>,
    pub key: Option<String>,
    pub branch: Option<String>,
    pub cwd: PathBuf,
}

impl CloneInputs {
    pub fn from_env(env: &dyn EnvLookup, cwd: PathBuf) -> Self {
        Self {
            url: env.var(HOOK_REPO_URL_ENV),
            key: env.var(HOOK_REPO_KEY_ENV),
            branch: env
                .var(HOOK_REPO_BRANCH_ENV)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            cwd,
        }
    }
}

/// Abstraction over `std::env::var` for testability.
pub trait EnvLookup {
    fn var(&self, key: &str) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StdEnv;

impl EnvLookup for StdEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// Resolve inputs into a concrete clone plan. Errors out cleanly when a
/// required env var is missing or empty.
pub fn resolve_plan(inputs: &CloneInputs) -> Result<ClonePlan, WorkspaceCloneError> {
    let url = require_non_empty(inputs.url.as_deref(), HOOK_REPO_URL_ENV)?;
    let key = require_non_empty(inputs.key.as_deref(), HOOK_REPO_KEY_ENV)?;
    let branch = inputs
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let mut argv: Vec<String> = vec!["clone".to_string(), "--depth".to_string(), "1".to_string()];
    if let Some(branch) = branch.as_deref() {
        argv.push("--branch".to_string());
        argv.push(branch.to_string());
    }
    argv.push(url.clone());
    argv.push(".".to_string());

    Ok(ClonePlan {
        url,
        key,
        branch,
        cwd: inputs.cwd.clone(),
        argv,
    })
}

fn require_non_empty(
    value: Option<&str>,
    env: &'static str,
) -> Result<String, WorkspaceCloneError> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        _ => Err(WorkspaceCloneError::MissingRequiredEnv {
            env,
            hint: format!(
                "set `{env}` to the resolved repo URL/key before invoking this subcommand"
            ),
        }),
    }
}

/// Decide whether `cwd` is empty, already cloned, or partial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceDirState {
    Empty,
    AlreadyCloned,
    Partial,
    Missing,
}

pub async fn classify_workspace_dir(cwd: &Path) -> std::io::Result<WorkspaceDirState> {
    match tokio::fs::metadata(cwd).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkspaceDirState::Missing);
        }
        Err(error) => return Err(error),
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Ok(WorkspaceDirState::Partial);
            }
        }
    }
    let mut entries = tokio::fs::read_dir(cwd).await?;
    let mut has_git = false;
    let mut has_other = false;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".git" {
            has_git = true;
        } else {
            has_other = true;
        }
    }
    Ok(match (has_git, has_other) {
        (true, _) => WorkspaceDirState::AlreadyCloned,
        (false, false) => WorkspaceDirState::Empty,
        (false, true) => WorkspaceDirState::Partial,
    })
}

pub async fn run(_args: WorkspaceCloneArgs) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let inputs = CloneInputs::from_env(&StdEnv, cwd.clone());
    match execute(inputs).await {
        Ok(report) => {
            eprintln!(
                "workspace clone: ok key={} url={} branch={} cwd={}",
                report.key,
                report.url,
                report.branch.as_deref().unwrap_or("<default>"),
                report.cwd.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("workspace clone: {error}");
            ExitCode::from(2)
        }
    }
}

#[derive(Debug, Default, Clone, clap::Args)]
pub struct WorkspaceCloneArgs {}

#[derive(Debug)]
struct ExecutionReport {
    key: String,
    url: String,
    branch: Option<String>,
    cwd: PathBuf,
}

async fn execute(inputs: CloneInputs) -> Result<ExecutionReport, WorkspaceCloneError> {
    let plan = resolve_plan(&inputs)?;
    let state = classify_workspace_dir(&plan.cwd).await.map_err(|error| {
        WorkspaceCloneError::GitFailed {
            detail: format!("inspecting workspace dir {}: {error}", plan.cwd.display()),
        }
    })?;
    match state {
        WorkspaceDirState::AlreadyCloned => {
            return Ok(ExecutionReport {
                key: plan.key,
                url: plan.url,
                branch: plan.branch,
                cwd: plan.cwd,
            });
        }
        WorkspaceDirState::Partial => {
            return Err(WorkspaceCloneError::PartialNonGitWorkspace { path: plan.cwd });
        }
        WorkspaceDirState::Empty | WorkspaceDirState::Missing => {}
    }

    spawn_git_clone(&plan).await?;

    Ok(ExecutionReport {
        key: plan.key,
        url: plan.url,
        branch: plan.branch,
        cwd: plan.cwd,
    })
}

async fn spawn_git_clone(plan: &ClonePlan) -> Result<(), WorkspaceCloneError> {
    let mut command = Command::new("git");
    command
        .args(&plan.argv)
        .current_dir(&plan.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = command
        .status()
        .await
        .map_err(|error| WorkspaceCloneError::GitFailed {
            detail: format!("spawning git clone: {error}"),
        })?;
    if !status.success() {
        return Err(WorkspaceCloneError::GitFailed {
            detail: format!(
                "git exited with status {} (argv: git {})",
                status,
                plan.argv.join(" ")
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MapEnv(HashMap<&'static str, String>);
    impl EnvLookup for MapEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
    }

    fn inputs(url: Option<&str>, key: Option<&str>, branch: Option<&str>) -> CloneInputs {
        CloneInputs {
            url: url.map(str::to_owned),
            key: key.map(str::to_owned),
            branch: branch.map(str::to_owned),
            cwd: PathBuf::from("/tmp/symphony-ws"),
        }
    }

    #[test]
    fn plan_includes_branch_when_env_present() {
        let plan = resolve_plan(&inputs(
            Some("git@github.com:foo/bar.git"),
            Some("foo/bar"),
            Some("develop"),
        ))
        .unwrap();
        assert_eq!(plan.url, "git@github.com:foo/bar.git");
        assert_eq!(plan.branch.as_deref(), Some("develop"));
        assert_eq!(
            plan.argv,
            vec![
                "clone",
                "--depth",
                "1",
                "--branch",
                "develop",
                "git@github.com:foo/bar.git",
                "."
            ]
        );
    }

    #[test]
    fn plan_omits_branch_when_env_absent() {
        let plan = resolve_plan(&inputs(
            Some("git@github.com:foo/bar.git"),
            Some("foo/bar"),
            None,
        ))
        .unwrap();
        assert_eq!(plan.branch, None);
        assert_eq!(
            plan.argv,
            vec!["clone", "--depth", "1", "git@github.com:foo/bar.git", "."]
        );
    }

    #[test]
    fn plan_treats_blank_branch_as_absent() {
        let plan = resolve_plan(&inputs(
            Some("git@github.com:foo/bar.git"),
            Some("foo/bar"),
            Some("   "),
        ))
        .unwrap();
        assert_eq!(plan.branch, None);
        assert!(!plan.argv.iter().any(|arg| arg == "--branch"));
    }

    #[test]
    fn missing_url_is_rejected() {
        let err = resolve_plan(&inputs(None, Some("foo/bar"), None)).unwrap_err();
        match err {
            WorkspaceCloneError::MissingRequiredEnv { env, .. } => {
                assert_eq!(env, HOOK_REPO_URL_ENV);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn missing_key_is_rejected() {
        let err =
            resolve_plan(&inputs(Some("git@github.com:foo/bar.git"), None, None)).unwrap_err();
        match err {
            WorkspaceCloneError::MissingRequiredEnv { env, .. } => {
                assert_eq!(env, HOOK_REPO_KEY_ENV);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn blank_url_is_rejected() {
        let err = resolve_plan(&inputs(Some("   "), Some("foo/bar"), None)).unwrap_err();
        assert!(matches!(
            err,
            WorkspaceCloneError::MissingRequiredEnv {
                env: HOOK_REPO_URL_ENV,
                ..
            }
        ));
    }

    #[test]
    fn from_env_reads_expected_keys() {
        let mut map = HashMap::new();
        map.insert(HOOK_REPO_URL_ENV, "git@github.com:foo/bar.git".to_string());
        map.insert(HOOK_REPO_KEY_ENV, "foo/bar".to_string());
        map.insert(HOOK_REPO_BRANCH_ENV, "develop".to_string());
        let inputs = CloneInputs::from_env(&MapEnv(map), PathBuf::from("/tmp/symphony-ws"));
        assert_eq!(inputs.url.as_deref(), Some("git@github.com:foo/bar.git"));
        assert_eq!(inputs.key.as_deref(), Some("foo/bar"));
        assert_eq!(inputs.branch.as_deref(), Some("develop"));
    }

    #[test]
    fn from_env_omits_blank_branch() {
        let mut map = HashMap::new();
        map.insert(HOOK_REPO_URL_ENV, "git@github.com:foo/bar.git".to_string());
        map.insert(HOOK_REPO_KEY_ENV, "foo/bar".to_string());
        map.insert(HOOK_REPO_BRANCH_ENV, "   ".to_string());
        let inputs = CloneInputs::from_env(&MapEnv(map), PathBuf::from("/tmp/symphony-ws"));
        assert!(inputs.branch.is_none());
    }

    #[tokio::test]
    async fn classify_empty_directory_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            classify_workspace_dir(dir.path()).await.unwrap(),
            WorkspaceDirState::Empty
        );
    }

    #[tokio::test]
    async fn classify_git_dir_is_already_cloned() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(dir.path().join(".git"))
            .await
            .unwrap();
        assert_eq!(
            classify_workspace_dir(dir.path()).await.unwrap(),
            WorkspaceDirState::AlreadyCloned
        );
    }

    #[tokio::test]
    async fn classify_partial_contents_is_partial() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("README.md"), b"leftover")
            .await
            .unwrap();
        assert_eq!(
            classify_workspace_dir(dir.path()).await.unwrap(),
            WorkspaceDirState::Partial
        );
    }

    /// Confirms that `OPENSYMPHONY_EXECUTION_REPO_KEY` is used (not dead code):
    /// `execute()` must thread the resolved key through `ExecutionReport` so
    /// the success log line can correlate the clone with the `RepoRef` the
    /// scheduler resolved upstream. We drive this through the `AlreadyCloned`
    /// short-circuit so no real `git` process is spawned.
    #[tokio::test]
    async fn execute_carries_resolved_key_for_logging() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(dir.path().join(".git"))
            .await
            .unwrap();

        let mut inputs = inputs(
            Some("git@github.com:foo/bar.git"),
            Some("foo/bar"),
            Some("develop"),
        );
        inputs.cwd = dir.path().to_path_buf();

        let report = execute(inputs)
            .await
            .expect("already-cloned should succeed");
        assert_eq!(report.key, "foo/bar");
        assert_eq!(report.url, "git@github.com:foo/bar.git");
        assert_eq!(report.branch.as_deref(), Some("develop"));
    }

    /// Same guarantee for the missing-branch path so the resolved `key` still
    /// reaches the log line even when no branch is injected.
    #[tokio::test]
    async fn execute_carries_resolved_key_when_branch_absent() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(dir.path().join(".git"))
            .await
            .unwrap();

        let mut inputs = inputs(Some("git@github.com:foo/bar.git"), Some("foo/bar"), None);
        inputs.cwd = dir.path().to_path_buf();

        let report = execute(inputs)
            .await
            .expect("already-cloned should succeed");
        assert_eq!(report.key, "foo/bar");
        assert!(report.branch.is_none());
    }
}
