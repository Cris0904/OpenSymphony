//! Tauri native commands orchestration.
//!
//! Re-exports types and provides the daemon_status command.
//! All other commands are defined in their respective modules
//! and registered directly in main.rs via module paths.

use serde::Serialize;
use tauri::command;
use crate::types::CommandResult;

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
