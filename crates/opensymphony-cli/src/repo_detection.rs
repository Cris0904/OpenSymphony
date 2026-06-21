//! Shared git remote/default-branch detection and repo slug derivation.
//!
//! Phase-1 multi-repo onboarding ([LOC-19](https://linear.app/localgputokenscrazy/issue/LOC-19/init-multi-repo-onboarding))
//! factors these helpers out of [`crate::opensymphony_cli::init_repo`] so
//! [`crate::opensymphony_cli::update_repo`] (and the
//! [LOC-20](https://linear.app/localgputokenscrazy/issue/LOC-20/existing-repo-project-set-migration)
//! migration path) can reuse them without duplicating `init`-private logic.

use std::{ops::Not, path::Path};

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
/// Uses `git symbolic-ref refs/remotes/<remote>/HEAD` first (the modern,
/// deterministic source), then falls back to `git remote show <remote>` and
/// parses the `HEAD branch:` line. Returns `None` when neither probe succeeds
/// or returns an empty branch name.
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

fn read_remote_show_default_branch(target_repo: &Path, remote_name: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "show", remote_name])
        .current_dir(target_repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
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
    fn derive_repo_slug_from_dir_returns_last_component() {
        let path = Path::new("/tmp/example");
        assert_eq!(derive_repo_slug_from_dir(path), Some("example".to_string()));
        let rel = Path::new("relative-dir");
        assert_eq!(
            derive_repo_slug_from_dir(rel),
            Some("relative-dir".to_string())
        );
    }
}
