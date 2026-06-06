//! Native action commands for desktop convenience operations.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use tauri::command;
use tauri_plugin_opener::OpenerExt;

use crate::types::{CommandResult, DesktopError};

#[derive(Debug, Deserialize)]
pub struct OpenFileRequest {
    pub title: Option<String>,
    pub accepts: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct OpenFileResponse {
    pub path: Option<String>,
}

#[command]
pub async fn open_file(_req: OpenFileRequest) -> CommandResult<OpenFileResponse> {
    Ok(OpenFileResponse { path: None })
}

#[derive(Debug, Serialize)]
pub struct OpenFolderResponse {
    pub path: Option<String>,
}

#[command]
pub async fn open_folder(_title: Option<String>) -> CommandResult<OpenFolderResponse> {
    Ok(OpenFolderResponse { path: None })
}

#[derive(Debug, Deserialize)]
pub struct OpenRepositoryFolderRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct OpenRepositoryFolderResponse {
    pub opened: bool,
}

#[command]
pub async fn open_repository_folder(
    app: tauri::AppHandle,
    req: OpenRepositoryFolderRequest,
) -> CommandResult<OpenRepositoryFolderResponse> {
    let p = std::path::Path::new(&req.path);
    // Canonicalize FIRST to resolve symlinks before safety check (prevent TOCTOU bypass)
    // canonicalize() returns io::Error for non-existent paths naturally
    let canon = p.canonicalize().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DesktopError::NotFound,
        std::io::ErrorKind::PermissionDenied => DesktopError::PermissionDenied,
        _ => DesktopError::Internal {
            message: format!("failed to canonicalize: {e}"),
        },
    })?;
    if !is_safe_workspace_path(&canon) {
        return Err(DesktopError::PermissionDenied);
    }
    let url = url::Url::from_file_path(&canon).map_err(|_| DesktopError::Internal {
        message: "invalid file path".into(),
    })?;
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| DesktopError::Internal {
        message: format!("failed to open folder: {e}"),
    })?;
    Ok(OpenRepositoryFolderResponse { opened: true })
}

#[derive(Debug, Deserialize)]
pub struct RevealWorkspaceRequest {
    pub path: String,
    pub safety_token: String,
}

#[derive(Debug, Serialize)]
pub struct RevealWorkspaceResponse {
    pub revealed: bool,
}

#[command]
pub async fn reveal_workspace(
    app: tauri::AppHandle,
    req: RevealWorkspaceRequest,
) -> CommandResult<RevealWorkspaceResponse> {
    if req.safety_token != "opensymphony-workspace" {
        return Err(DesktopError::PermissionDenied);
    }
    let p = std::path::Path::new(&req.path);

    // Canonicalize home_dir first to resolve any symlinks in the home path
    let home = dirs::home_dir().ok_or_else(|| DesktopError::Internal {
        message: "could not determine home directory".into(),
    })?;
    let canon_home = home.canonicalize().map_err(|e| DesktopError::Internal {
        message: format!("failed to canonicalize home directory: {e}"),
    })?;
    let canon_base = canon_home.join(".opensymphony").join("workspaces");

    // Canonicalize the input path and check containment against the resolved base
    let canon = p.canonicalize().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DesktopError::NotFound,
        std::io::ErrorKind::PermissionDenied => DesktopError::PermissionDenied,
        _ => DesktopError::Internal {
            message: format!("failed to canonicalize: {e}"),
        },
    })?;
    if !canon.starts_with(&canon_base) {
        return Err(DesktopError::PermissionDenied);
    }

    let url = url::Url::from_file_path(&canon).map_err(|_| DesktopError::Internal {
        message: "invalid workspace path".into(),
    })?;
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| DesktopError::Internal {
        message: format!("failed to reveal workspace: {e}"),
    })?;
    Ok(RevealWorkspaceResponse { revealed: true })
}

fn is_safe_workspace_path(path: &std::path::Path) -> bool {
    // Whitelist-based security check: default to false for unknown paths.
    // Only allow paths under ~/.opensymphony/workspaces/ after full canonicalization.
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    
    // Canonicalize home first to resolve any symlinks in the home path itself
    let Ok(canon_home) = home.canonicalize() else {
        return false;
    };
    
    // Build the canonical base path
    let canon_base = canon_home.join(".opensymphony").join("workspaces");
    // Ensure the base directory exists for containment checks
    if !canon_base.exists() {
        return false;
    }
    let Ok(canon_base) = canon_base.canonicalize() else {
        return false;
    };
    
    // Try to canonicalize the input path - if it doesn't exist or can't be resolved, reject it
    let Ok(canon_path) = path.canonicalize() else {
        return false;
    };
    
    // Check strict containment under the canonical base
    canon_path.starts_with(&canon_base)
}

#[derive(Debug, Deserialize)]
pub struct CopyToClipboardRequest {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct CopyToClipboardResponse {
    pub copied: bool,
}

#[command]
pub async fn copy_to_clipboard(
    app: tauri::AppHandle,
    req: CopyToClipboardRequest,
) -> CommandResult<CopyToClipboardResponse> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(&req.text).map_err(|e| DesktopError::Internal {
        message: format!("failed to copy: {e}"),
    })?;
    Ok(CopyToClipboardResponse { copied: true })
}

#[derive(Debug, Deserialize)]
pub struct OpenLinearLinkRequest {
    pub issue_id: String,
}

// Configurable Linear workspace base URL (override at build time if needed)
const LINEAR_WORKSPACE_BASE: &str = "https://linear.app/trilogy-ai-coe";

#[command]
pub async fn open_linear_link(
    app: tauri::AppHandle,
    req: OpenLinearLinkRequest,
) -> CommandResult<()> {
    let encoded_id = urlencoding::encode(&req.issue_id);
    let url = format!("{}/issue/{}", LINEAR_WORKSPACE_BASE, encoded_id);
    app.opener().open_url(&url, None::<&str>).map_err(|e| DesktopError::Internal {
        message: format!("failed to open link: {e}"),
    })
}

#[derive(Debug, Deserialize)]
pub struct NotifyRequest {
    pub title: String,
    pub body: String,
    pub level: Option<NotifyLevel>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum NotifyLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Serialize)]
pub struct NotifyResponse {
    pub acknowledged: bool,
}

#[command]
pub async fn notify(
    app: tauri::AppHandle,
    req: NotifyRequest,
) -> CommandResult<NotifyResponse> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(&req.title)
        .body(&req.body)
        .show()
        .map_err(|e| DesktopError::Internal {
            message: format!("failed to show notification: {e}"),
        })?;
    Ok(NotifyResponse { acknowledged: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notify_level_deserialization() {
        let i: NotifyLevel = serde_json::from_str(r#""info""#).unwrap();
        assert!(matches!(i, NotifyLevel::Info));
        let w: NotifyLevel = serde_json::from_str(r#""warning""#).unwrap();
        assert!(matches!(w, NotifyLevel::Warning));
        let e: NotifyLevel = serde_json::from_str(r#""error""#).unwrap();
        assert!(matches!(e, NotifyLevel::Error));
        // Invalid level should fail
        assert!(serde_json::from_str::<NotifyLevel>(r#""critical""#).is_err());
    }

    #[test]
    fn test_copy_request() {
        let r = CopyToClipboardRequest { text: "test".into() };
        assert_eq!(r.text, "test");
    }

    #[test]
    fn test_linear_link_request() {
        let r = OpenLinearLinkRequest { issue_id: "COE-409".into() };
        assert_eq!(r.issue_id, "COE-409");
    }

    #[test]
    fn test_linear_url_constant() {
        assert!(LINEAR_WORKSPACE_BASE.starts_with("https://linear.app/"));
        let url = format!("{}/issue/{}", LINEAR_WORKSPACE_BASE, "COE-123");
        assert_eq!(url, "https://linear.app/trilogy-ai-coe/issue/COE-123");
    }

    #[test]
    fn test_safety_workspace_path_allows_home() {
        // Create a temporary test workspace directory to test with real paths
        let home = dirs::home_dir().expect("home dir available");
        let test_workspace = home.join(".opensymphony").join("workspaces").join("test-safe-path");
        
        // Create the directory if it doesn't exist
        std::fs::create_dir_all(&test_workspace).ok();
        
        let result = is_safe_workspace_path(&test_workspace);
        
        // Clean up
        std::fs::remove_dir(&test_workspace).ok();
        
        assert!(result, "Existing workspace path under ~/.opensymphony/workspaces should be allowed");
    }

    #[test]
    fn test_safety_workspace_path_blocks_system() {
        let path = std::path::Path::new("/System/Volumes/Data");
        assert!(!is_safe_workspace_path(path));
        let path = std::path::Path::new("/usr/bin/something");
        assert!(!is_safe_workspace_path(path));
        let path = std::path::Path::new("/etc/passwd");
        assert!(!is_safe_workspace_path(path));
        let path = std::path::Path::new("/private/var/folders");
        assert!(!is_safe_workspace_path(path));
    }

    #[test]
    fn test_notify_request_structure() {
        let req = NotifyRequest {
            title: "Test".into(),
            body: "Body".into(),
            level: Some(NotifyLevel::Info),
        };
        assert_eq!(req.title, "Test");
        assert!(matches!(req.level, Some(NotifyLevel::Info)));
    }

    #[test]
    fn test_is_safe_workspace_path_blocks_tricky_system_paths() {
        let blocked = vec![
            "/System/Volumes/Data/.opensymphony/workspaces/escape",
            "/usr/local/.opensymphony/workspaces/escape",
            "/etc/opensymphony/workspaces/escape",
            "/private/var/folders/.opensymphony/test",
        ];
        for path_str in blocked {
            assert!(
                !is_safe_workspace_path(std::path::Path::new(path_str)),
                "Path {path_str} should be blocked by system prefix check"
            );
        }
    }

    #[test]
    fn test_canonicalize_nonexistent_path_error_kind() {
        let path = std::path::Path::new("/definitely/does/not/exist/12345");
        let result = path.canonicalize();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_is_safe_workspace_path_allows_opensymphony_subdirs() {
        // Create actual test workspace directories to test with real paths
        let home = dirs::home_dir().expect("home dir available");
        let base = home.join(".opensymphony").join("workspaces");
        
        // Create base directory if it doesn't exist
        std::fs::create_dir_all(&base).ok();
        
        let valid = vec![
            base.join("test-subdir-1"),
            base.join("test-subdir-2").join("nested"),
            base.join("test-subdir-3"),
        ];
        
        // Create the test directories
        for path in &valid {
            std::fs::create_dir_all(path).ok();
        }
        
        for path in &valid {
            assert!(
                is_safe_workspace_path(path),
                "Path {:?} should be allowed", path
            );
        }
        
        // Clean up
        for path in valid.iter().rev() {
            std::fs::remove_dir(path).ok();
        }
    }

    #[test]
    fn test_is_safe_workspace_path_blocks_path_traversal_attempts() {
        // Paths that try to escape via .. or mimic .opensymphony elsewhere should be blocked
        let home = dirs::home_dir().expect("home dir available");
        let blocked = vec![
            home.join(".opensymphony").join("..").join("..").join("etc").join("passwd"),
            std::path::PathBuf::from("/var/.opensymphony/workspaces/test"),
            std::path::PathBuf::from("/tmp/.opensymphony/workspaces/test"),
            home.join(".opensymphony").join("workspaces").join("..").join("..").join("etc").join("shadow"),
        ];
        for path in blocked {
            assert!(
                !is_safe_workspace_path(&path),
                "Path {:?} should be blocked", path
            );
        }
    }

    #[test]
    fn test_notify_all_levels_deserialize() {
        // Verify all valid notification levels work
        for level in &["info", "warning", "error"] {
            let json = format!(r#"{{"title":"T","body":"B","level":"{}"}}"#, level);
            let result: Result<NotifyRequest, _> = serde_json::from_str(&json);
            assert!(result.is_ok(), "Level '{}' should deserialize", level);
        }
        // Missing level should still work (defaults to None)
        let json = r#"{"title":"T","body":"B"}"#;
        let req: NotifyRequest = serde_json::from_str(json).unwrap();
        assert!(req.level.is_none());
    }

    #[test]
    fn test_open_linear_link_request_url_encoding() {
        // Verify URL encoding handles special characters in issue IDs
        let req = OpenLinearLinkRequest {
            issue_id: "COE-409".into(),
        };
        let encoded = urlencoding::encode(&req.issue_id);
        let url = format!("{}/issue/{}", LINEAR_WORKSPACE_BASE, encoded);
        assert_eq!(url, "https://linear.app/trilogy-ai-coe/issue/COE-409");
        
        // Test with special characters that need encoding
        let req_special = OpenLinearLinkRequest {
            issue_id: "COE-409/test".into(),
        };
        let encoded_special = urlencoding::encode(&req_special.issue_id);
        assert!(encoded_special.to_string().contains("%2F"), "Slash should be URL-encoded");
    }
}
