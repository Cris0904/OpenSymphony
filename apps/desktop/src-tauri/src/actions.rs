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
    if !p.exists() {
        return Err(DesktopError::NotFound);
    }
    if !is_safe_workspace_path(p) {
        return Err(DesktopError::PermissionDenied);
    }
    let canon = p.canonicalize().map_err(|e| DesktopError::Internal {
        message: format!("failed to canonicalize: {e}"),
    })?;
    let url = url::Url::from_file_path(&canon).map_err(|_| DesktopError::Internal {
        message: "invalid file path".into(),
    })?;
    let _ = app.opener().open_url(url.as_str(), None::<&str>);
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
    if !p.exists() {
        return Err(DesktopError::NotFound);
    }
    let canon = p.canonicalize().map_err(|e| DesktopError::Internal {
        message: format!("failed to canonicalize: {e}"),
    })?;
    let home = dirs::home_dir().ok_or_else(|| DesktopError::Internal {
        message: "could not determine home directory".into(),
    })?;
    let base = home.join(".opensymphony").join("workspaces");
    if !canon.starts_with(&base) {
        return Err(DesktopError::PermissionDenied);
    }
    let url = url::Url::from_file_path(&canon).map_err(|_| DesktopError::Internal {
        message: "invalid workspace path".into(),
    })?;
    let _ = app.opener().open_url(url.as_str(), None::<&str>);
    Ok(RevealWorkspaceResponse { revealed: true })
}

fn is_safe_workspace_path(path: &std::path::Path) -> bool {
    if let Some(home) = dirs::home_dir() {
        let os_base = home.join(".opensymphony");
        if path.starts_with(&os_base) {
            return true;
        }
    }
    let s = path.to_string_lossy();
    !s.starts_with("/System")
        && !s.starts_with("/usr")
        && !s.starts_with("/etc")
        && !s.starts_with("/private/var")
        && path.exists()
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
    req: CopyToClipboardRequest,
) -> CommandResult<CopyToClipboardResponse> {
    use arboard::Clipboard;
    let mut cb = Clipboard::new().map_err(|e| DesktopError::Internal {
        message: format!("clipboard unavailable: {e}"),
    })?;
    cb.set_text(&req.text).map_err(|e| DesktopError::Internal {
        message: format!("failed to copy: {e}"),
    })?;
    Ok(CopyToClipboardResponse { copied: true })
}

#[derive(Debug, Deserialize)]
pub struct OpenLinearLinkRequest {
    pub issue_id: String,
}

#[command]
pub async fn open_linear_link(
    app: tauri::AppHandle,
    req: OpenLinearLinkRequest,
) -> CommandResult<()> {
    let url = format!(
        "https://linear.app/trilogy-ai-coe/issue/{}",
        req.issue_id
    );
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
}
