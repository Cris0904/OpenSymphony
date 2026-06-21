//! Shared git remote/default-branch detection and repo slug derivation.
//!
//! Phase-1 multi-repo onboarding ([LOC-19](https://linear.app/localgputokenscrazy/issue/LOC-19/init-multi-repo-onboarding))
//! factors these helpers out of [`crate::opensymphony_cli::init_repo`] so
//! [`crate::opensymphony_cli::update_repo`] (and the
//! [LOC-20](https://linear.app/localgputokenscrazy/issue/LOC-20/existing-repo-project-set-migration)
//! migration path) can reuse them without duplicating `init`-private logic.

use std::{ops::Not, path::Path, process::Stdio, time::Duration};

/// Default timeout (in milliseconds) for the `git remote show <remote>` probe
/// used by [`read_remote_show_default_branch`]. Bounded so that an
/// unresponsive remote (air-gapped CI, slow proxy, etc.) cannot block the CLI
/// indefinitely. Override with the `OPENHANDS_GIT_REMOTE_SHOW_TIMEOUT_MS`
/// environment variable.
pub(crate) const DEFAULT_REMOTE_SHOW_TIMEOUT_MS: u64 = 5_000;

/// Polling interval (in milliseconds) used while waiting for `git remote show`
/// to complete. Kept small so the effective timeout is close to the configured
/// value, but large enough to avoid a tight spin loop.
const REMOTE_SHOW_POLL_INTERVAL_MS: u64 = 20;

const REMOTE_SHOW_TIMEOUT_ENV: &str = "OPENHANDS_GIT_REMOTE_SHOW_TIMEOUT_MS";

/// Resolves the configured `git remote show` timeout from the environment,
/// falling back to [`DEFAULT_REMOTE_SHOW_TIMEOUT_MS`]. Mirrors the parsing
/// pattern used for `OPENSYMPHONY_TEMPLATE_FETCH_TIMEOUT_MS` in
/// [`crate::opensymphony_cli::init_repo`] so operators only need to learn
/// one env-var shape across the CLI.
fn remote_show_timeout() -> Duration {
    remote_show_timeout_from_env(std::env::var(REMOTE_SHOW_TIMEOUT_ENV).ok().as_deref())
}

fn remote_show_timeout_from_env(value: Option<&str>) -> Duration {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|timeout_ms| *timeout_ms > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_REMOTE_SHOW_TIMEOUT_MS))
}

/// Result of probing the local git repository for its canonical clone remote.
///
/// `Selected` means we confidently picked one remote (always preferring
/// `origin` over a single other remote). `None` means no remotes or a probe
/// failure (e.g. not a git repo, no `git` binary). `Ambiguous` means multiple
/// non-`origin` remotes exist and we will not guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRemoteDetection {
    Selected { remote_name: String, url: String },
    None,
    Ambiguous(Vec<String>),
}

impl GitRemoteDetection {
    /// Returns the detected remote URL when one was confidently selected.
    pub fn url(&self) -> Option<&str> {
        match self {
            Self::Selected { url, .. } => Some(url.as_str()),
            Self::None | Self::Ambiguous(_) => None,
        }
    }

    /// Returns the detected remote name when one was confidently selected.
    pub fn remote_name(&self) -> Option<&str> {
        match self {
            Self::Selected { remote_name, .. } => Some(remote_name.as_str()),
            Self::None | Self::Ambiguous(_) => None,
        }
    }
}

/// Probes `target_repo` for a single git remote, preferring `origin`.
///
/// Returns:
/// - `GitRemoteDetection::Selected { remote_name, url }` when a single remote
///   was confidently picked.
/// - `GitRemoteDetection::None` when no remotes exist or the probe failed.
/// - `GitRemoteDetection::Ambiguous(remotes)` when multiple non-`origin`
///   remotes exist and no `origin` is present, so callers can prompt for a
///   choice instead of silently guessing.
pub fn detect_git_remote_url(target_repo: &Path) -> GitRemoteDetection {
    let output = std::process::Command::new("git")
        .args(["remote"])
        .current_dir(target_repo)
        .output();
    let Ok(output) = output else {
        return GitRemoteDetection::None;
    };
    if !output.status.success() {
        return GitRemoteDetection::None;
    }

    let remotes = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let Some(remote_name) = select_remote_name(&remotes) else {
        return if remotes.len() > 1 {
            GitRemoteDetection::Ambiguous(remotes)
        } else {
            GitRemoteDetection::None
        };
    };

    let get_url = std::process::Command::new("git")
        .args(["remote", "get-url", &remote_name])
        .current_dir(target_repo)
        .output();
    let Ok(get_url) = get_url else {
        return GitRemoteDetection::None;
    };
    if !get_url.status.success() {
        return GitRemoteDetection::None;
    }

    let url = String::from_utf8_lossy(&get_url.stdout).trim().to_owned();
    if url.is_empty() {
        GitRemoteDetection::None
    } else {
        GitRemoteDetection::Selected { remote_name, url }
    }
}

/// Selects the canonical remote name from the list of configured remotes.
///
/// Always prefers `origin`. When no `origin` is configured, falls back to the
/// single remote when exactly one is configured; otherwise returns `None` so
/// callers can treat the situation as ambiguous.
pub fn select_remote_name(remotes: &[String]) -> Option<String> {
    if remotes.iter().any(|remote| remote == "origin") {
        Some("origin".to_string())
    } else if remotes.len() == 1 {
        remotes.first().cloned()
    } else {
        None
    }
}

/// Probes `target_repo` for the default branch of `remote_name`.
///
/// Probes in order:
/// 1. `git symbolic-ref refs/remotes/<remote>/HEAD` (local-only, no network;
///    preferred when the repo has been freshly cloned with `--no-single-branch`
///    or already ran `git remote set-head <remote> --auto`).
/// 2. `git remote show <remote>`, parsing the `HEAD branch:` line. This step
///    makes a synchronous network call to the remote server, so it is
///    bounded by [`DEFAULT_REMOTE_SHOW_TIMEOUT_MS`] (5s by default,
///    configurable via `OPENHANDS_GIT_REMOTE_SHOW_TIMEOUT_MS`). On timeout or
///    other failure the helper falls through to `None` and emits a
///    structured `tracing::warn!`.
///
/// Returns `None` when neither probe succeeds or returns an empty branch
/// name. The caller-visible signature is intentionally unchanged from
/// the pre-timeout implementation; the only behavioural difference is
/// that the `git remote show` path can no longer hang the CLI.
pub fn detect_git_default_branch(target_repo: &Path, remote_name: &str) -> Option<String> {
    if let Some(branch) = read_symbolic_default_branch(target_repo, remote_name) {
        return Some(branch);
    }
    read_remote_show_default_branch(target_repo, remote_name)
}

fn read_symbolic_default_branch(target_repo: &Path, remote_name: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["symbolic-ref", &format!("refs/remotes/{remote_name}/HEAD")])
        .current_dir(target_repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let branch = raw.strip_prefix("refs/remotes/")?.rsplit('/').next()?;
    let trimmed = branch.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Best-effort detection of the default branch via `git remote show <remote>`.
///
/// **Network dependency (LOC-19/LOC-27):** this command makes a network
/// call to the remote server, so it can hang or fail silently in CI or
/// air-gapped environments. The caller falls back to
/// [`read_symbolic_default_branch`] (a local-only path) first, and this
/// helper bounds the network round-trip by
/// [`DEFAULT_REMOTE_SHOW_TIMEOUT_MS`] (5s by default, configurable via
/// `OPENHANDS_GIT_REMOTE_SHOW_TIMEOUT_MS`). On timeout the child `git`
/// process is killed and reaped (no zombies), a `tracing::warn!` is
/// emitted with structured fields, and the helper returns `None` so the
/// caller can fall back to the inventory entry without a `default_branch`.
fn read_remote_show_default_branch(target_repo: &Path, remote_name: &str) -> Option<String> {
    read_remote_show_default_branch_with_git(target_repo, remote_name, "git", remote_show_timeout())
}

fn read_remote_show_default_branch_with_git(
    target_repo: &Path,
    remote_name: &str,
    git_bin: &str,
    timeout: Duration,
) -> Option<String> {
    let mut child = std::process::Command::new(git_bin)
        .args(["remote", "show", remote_name])
        .current_dir(target_repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let started = std::time::Instant::now();
    let poll_interval = Duration::from_millis(REMOTE_SHOW_POLL_INTERVAL_MS);
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                let output = child.wait_with_output().ok()?;
                if !output.status.success() {
                    return None;
                }
                return parse_remote_show_default_branch(&output.stdout);
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    tracing::warn!(
                        target_repo = %target_repo.display(),
                        remote_name,
                        timeout_ms = timeout.as_millis() as u64,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "git remote show timed out; falling back to default_branch = None"
                    );
                    return None;
                }
                std::thread::sleep(poll_interval);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(
                    target_repo = %target_repo.display(),
                    remote_name,
                    error = %error,
                    "git remote show failed to poll; falling back to default_branch = None"
                );
                return None;
            }
        }
    }
}

fn parse_remote_show_default_branch(stdout: &[u8]) -> Option<String> {
    for line in String::from_utf8_lossy(stdout).lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("HEAD branch:") else {
            continue;
        };
        let candidate = rest.trim();
        if candidate.is_empty() || candidate.eq_ignore_ascii_case("(unknown)") {
            return None;
        }
        return Some(candidate.to_owned());
    }
    None
}

/// Derives a project-set inventory repo slug from a git remote URL.
///
/// Returns the last path segment of the URL, with a trailing `.git` trimmed,
/// when it can be confidently extracted. Returns `None` for malformed URLs
/// (no path component or no parseable URL/scp-like form) so callers can fall
/// back to directory-based naming.
pub fn derive_repo_slug_from_remote(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = if let Some(scp) = scp_like_path(trimmed) {
        scp
    } else if let Ok(parsed) = url::Url::parse(trimmed) {
        parsed.path().trim_end_matches('/').to_owned()
    } else {
        return None;
    };

    let segment = last_segment(&path)?;
    let stripped = segment.strip_suffix(".git").unwrap_or(&segment);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_owned())
    }
}

/// Treats `git@host:owner/repo.git`-style URLs as `owner/repo` paths.
fn scp_like_path(url: &str) -> Option<String> {
    let (user_host, path) = url.split_once(':')?;
    if user_host.contains('/') || user_host.contains('@').not() {
        return None;
    }
    if path.starts_with('/') || path.starts_with('~') {
        return None;
    }
    Some(path.to_owned())
}

fn last_segment(path: &str) -> Option<String> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_owned())
}

/// Derives a project-set inventory repo slug from the directory name at
/// `repo_dir`. Returns `None` when the directory has no usable name component.
pub fn derive_repo_slug_from_dir(repo_dir: &Path) -> Option<String> {
    let last = repo_dir
        .components()
        .next_back()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())?;
    let trimmed = last.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == "/" {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_remote_prefers_origin_then_single_remote() {
        assert_eq!(
            select_remote_name(&["fork".to_string(), "origin".to_string()]),
            Some("origin".to_string())
        );
        assert_eq!(
            select_remote_name(&["upstream".to_string()]),
            Some("upstream".to_string())
        );
        assert_eq!(
            select_remote_name(&["fork".to_string(), "upstream".to_string()]),
            None
        );
        assert_eq!(select_remote_name(&[]), None);
    }

    #[test]
    fn detection_url_and_remote_name_only_return_selected() {
        let selected = GitRemoteDetection::Selected {
            remote_name: "origin".to_string(),
            url: "https://github.com/example/demo.git".to_string(),
        };
        assert_eq!(selected.url(), Some("https://github.com/example/demo.git"));
        assert_eq!(selected.remote_name(), Some("origin"));

        assert_eq!(GitRemoteDetection::None.url(), None);
        assert_eq!(
            GitRemoteDetection::Ambiguous(vec!["fork".to_string()]).url(),
            None
        );
    }

    #[test]
    fn derive_repo_slug_from_remote_handles_https_and_ssh_forms() {
        assert_eq!(
            derive_repo_slug_from_remote("https://github.com/kumanday/OpenSymphony.git"),
            Some("OpenSymphony".to_string())
        );
        assert_eq!(
            derive_repo_slug_from_remote("https://github.com/kumanday/OpenSymphony"),
            Some("OpenSymphony".to_string())
        );
        assert_eq!(
            derive_repo_slug_from_remote("git@github.com:kumanday/OpenSymphony.git"),
            Some("OpenSymphony".to_string())
        );
        assert_eq!(
            derive_repo_slug_from_remote("ssh://git@github.com/kumanday/OpenSymphony.git"),
            Some("OpenSymphony".to_string())
        );
        assert_eq!(
            derive_repo_slug_from_remote("git@github.com:kumanday/OpenSymphony"),
            Some("OpenSymphony".to_string())
        );
    }

    #[test]
    fn derive_repo_slug_from_remote_rejects_unusable_inputs() {
        assert_eq!(derive_repo_slug_from_remote(""), None);
        assert_eq!(derive_repo_slug_from_remote("not-a-url"), None);
        // Trailing slash with no path segment → still no segment.
        assert_eq!(derive_repo_slug_from_remote("https://github.com/"), None);
    }

    #[test]
    fn derive_repo_slug_from_remote_rejects_malformed_scp_paths() {
        // SCP-like form with a leading slash inside the path (e.g.
        // `git@github.com:/org/repo.git`) is malformed; `scp_like_path`
        // rejects it so the caller can fall back to `derive_repo_slug_from_dir`
        // rather than producing a low-quality slug from a broken path.
        assert_eq!(
            derive_repo_slug_from_remote("git@github.com:/org/repo.git"),
            None
        );
        // SCP-like form starting with `~` (e.g. `git@host:~/repo.git`) is
        // also rejected for the same reason.
        assert_eq!(
            derive_repo_slug_from_remote("git@github.com:~/repo.git"),
            None
        );
    }

    #[test]
    fn derive_repo_slug_from_dir_returns_last_component() {
        let path = Path::new("/tmp/example");
        assert_eq!(derive_repo_slug_from_dir(path), Some("example".to_string()));
        let rel = Path::new("relative-dir");
        assert_eq!(
            derive_repo_slug_from_dir(rel),
            Some("relative-dir".to_string())
        );
    }

    #[test]
    fn remote_show_timeout_from_env_defaults_when_unset() {
        assert_eq!(
            remote_show_timeout_from_env(None),
            Duration::from_millis(DEFAULT_REMOTE_SHOW_TIMEOUT_MS)
        );
    }

    #[test]
    fn remote_show_timeout_from_env_defaults_when_blank() {
        assert_eq!(
            remote_show_timeout_from_env(Some("")),
            Duration::from_millis(DEFAULT_REMOTE_SHOW_TIMEOUT_MS)
        );
    }

    #[test]
    fn remote_show_timeout_from_env_parses_positive_integer() {
        assert_eq!(
            remote_show_timeout_from_env(Some("250")),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn remote_show_timeout_from_env_falls_back_on_zero_or_negative_or_garbage() {
        // Zero/negative/garbage values are not meaningful timeouts, so the
        // helper falls back to the default rather than degrading to an
        // immediate `None` or a panic.
        assert_eq!(
            remote_show_timeout_from_env(Some("0")),
            Duration::from_millis(DEFAULT_REMOTE_SHOW_TIMEOUT_MS)
        );
        assert_eq!(
            remote_show_timeout_from_env(Some("-1")),
            Duration::from_millis(DEFAULT_REMOTE_SHOW_TIMEOUT_MS)
        );
        assert_eq!(
            remote_show_timeout_from_env(Some("not-a-number")),
            Duration::from_millis(DEFAULT_REMOTE_SHOW_TIMEOUT_MS)
        );
    }

    #[test]
    fn detect_git_default_branch_returns_none_when_git_remote_show_times_out() {
        // Wrap `git` so that `git remote show <remote>` sleeps past the
        // configured timeout (and the symbolic-ref probe is also redirected
        // so the timeout path is the only fallback). The wrapper exits
        // non-zero, so even if the timeout were too generous the call
        // would still resolve to `None` via the `status.success()` branch.
        let bin_dir = tempfile::TempDir::new().expect("bin dir should exist");
        let fake_git = bin_dir.path().join("git");
        let wrapper = r#"#!/bin/sh
# LOC-27 regression wrapper: simulate an unresponsive `git remote show`
# by sleeping well past the test timeout. The symbolic-ref path is also
# forced to fail so the timeout fallback is the one being exercised.
if [ "$1" = "remote" ] && [ "$2" = "show" ]; then
  sleep 5
  exit 0
fi
exit 128
"#;
        std::fs::write(&fake_git, wrapper).expect("wrapper should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&fake_git)
                .expect("wrapper metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&fake_git, permissions).expect("wrapper should be executable");
        }

        let repo_dir = tempfile::TempDir::new().expect("repo dir should exist");
        let timeout = Duration::from_millis(100);
        let started = std::time::Instant::now();
        let result = read_remote_show_default_branch_with_git(
            repo_dir.path(),
            "origin",
            fake_git.to_str().expect("wrapper path should be utf-8"),
            timeout,
        );
        let elapsed = started.elapsed();

        // The helper must return `None` (no panic, no hang).
        assert_eq!(
            result, None,
            "detect_git_default_branch must return None when git remote show times out"
        );
        // The timeout must actually have fired (not a fast path that
        // happened to return None for some other reason) and the call
        // must not have run to completion of the 5-second wrapper sleep.
        assert!(
            elapsed >= timeout,
            "call should have waited at least the configured timeout, elapsed = {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "call should have been killed by the timeout, elapsed = {elapsed:?}"
        );
    }

    #[test]
    fn detect_git_default_branch_returns_branch_when_git_remote_show_completes() {
        // Sanity check the happy path: a wrapper that prints a `HEAD branch:`
        // line should be parsed even when invoked through the timeout helper.
        let bin_dir = tempfile::TempDir::new().expect("bin dir should exist");
        let fake_git = bin_dir.path().join("git");
        let wrapper = r#"#!/bin/sh
if [ "$1" = "remote" ] && [ "$2" = "show" ]; then
  printf '* remote origin\n  HEAD branch: main\n'
  exit 0
fi
exit 0
"#;
        std::fs::write(&fake_git, wrapper).expect("wrapper should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&fake_git)
                .expect("wrapper metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&fake_git, permissions).expect("wrapper should be executable");
        }

        let repo_dir = tempfile::TempDir::new().expect("repo dir should exist");
        let result = read_remote_show_default_branch_with_git(
            repo_dir.path(),
            "origin",
            fake_git.to_str().expect("wrapper path should be utf-8"),
            Duration::from_secs(2),
        );
        assert_eq!(result, Some("main".to_string()));
    }
}
