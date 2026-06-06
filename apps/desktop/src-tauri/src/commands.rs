//! Tauri native commands orchestration.
//!
//! Re-exports types and provides the daemon_status command.
//! All other commands are defined in their respective modules
//! and registered directly in main.rs via module paths.

use serde::Serialize;
use tauri::command;

pub use crate::types::{CommandResult, DesktopError};

pub use crate::settings::{
    get_setting, set_setting, GetSettingRequest, GetSettingResponse, SetSettingRequest,
    SetSettingResponse, SettingValue,
};
pub use crate::keychain::{
    credential_status, delete_credential, get_credential, set_credential,
    CredentialStatusRequest, CredentialStatusResponse, DeleteCredentialRequest,
    GetCredentialRequest, GetCredentialResponse, SetCredentialRequest,
    SetCredentialResponse,
};
pub use crate::actions::{
    copy_to_clipboard, notify, open_file, open_folder, open_linear_link,
    open_repository_folder, reveal_workspace, CopyToClipboardRequest, CopyToClipboardResponse,
    NotifyRequest, NotifyResponse, OpenFileRequest, OpenFileResponse, OpenFolderResponse,
    OpenLinearLinkRequest, OpenRepositoryFolderRequest, OpenRepositoryFolderResponse,
    RevealWorkspaceRequest, RevealWorkspaceResponse, NotifyLevel,
};

// --- Local Process Supervision (stencil only) ---

#[derive(Debug, Serialize)]
pub struct ProcessStatus {
    pub pid: Option<u32>,
    pub running: bool,
}

#[command]
pub async fn daemon_status() -> CommandResult<ProcessStatus> {
    // COE-404 will implement actual discovery + supervision.
    Ok(ProcessStatus {
        pid: None,
        running: false,
    })
}
