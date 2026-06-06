//! Shared types for desktop commands.

use serde::Serialize;
use thiserror::Error;

/// Structured error type returned by desktop native commands.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "type")]
pub enum DesktopError {
    #[error("operation cancelled")]
    Cancelled,
    #[error("resource not found")]
    NotFound,
    #[error("permission denied")]
    PermissionDenied,
    #[error("daemon unavailable")]
    DaemonUnavailable,
    #[error("internal error: {message}")]
    Internal { message: String },
    #[error("keychain error: {message}")]
    Keychain { message: String },
    #[error("settings error: {message}")]
    Settings { message: String },
}

pub type CommandResult<T> = Result<T, DesktopError>;
