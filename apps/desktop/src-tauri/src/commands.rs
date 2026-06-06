//! Tauri native commands exposed to the frontend.
//!
//! Every command uses narrow, strongly-typed request and response structs so
//! that the capability matrix stays auditable and the attack surface is small.
//!
//! Fields are stubbed and unused until COE-404/COE-409 implement real backends.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use tauri::command;
use thiserror::Error;

// ─── Error type ─────────────────────────────────────────────────────────────

/// Structured error type returned by desktop native commands.
/// Replaces opaque `String` errors so the frontend can distinguish
/// permission denied, not found, cancelled, and internal failure.
///
/// Uses internally-tagged serialization so every variant produces a uniform
/// JSON shape: `{"type":"Cancelled"}`, `{"type":"Internal","message":"..."}`.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "type")]
pub enum DesktopError {
    /// The user cancelled the operation (e.g., closed a file picker).
    #[error("operation cancelled")]
    Cancelled,
    /// The requested resource does not exist.
    #[error("resource not found")]
    NotFound,
    /// Insufficient permissions to perform the operation.
    #[error("permission denied")]
    PermissionDenied,
    /// The local daemon is not running.
    #[error("daemon unavailable")]
    DaemonUnavailable,
    /// Gateway connection failed.
    #[error("gateway error: {message}")]
    GatewayError { message: String },
    /// Generic internal error with a human-readable message.
    #[error("internal error: {message}")]
    Internal { message: String },
}

/// Alias for ergonomic command return types.
type CommandResult<T> = Result<T, DesktopError>;

// ─── File / Folder Selection ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OpenFileRequest {
    /// Human-readable title shown in the native dialog.
    pub title: Option<String>,
    /// Allowed MIME types (empty means all).
    pub accepts: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct OpenFileResponse {
    /// Absolute path chosen by the user, or `None` on cancel.
    pub path: Option<String>,
}

/// Stub: open a single-file picker dialog.
#[command]
pub async fn open_file(_req: OpenFileRequest) -> CommandResult<OpenFileResponse> {
    // Real implementation uses `tauri_plugin_dialog::ask` / `open`.
    Ok(OpenFileResponse { path: None })
}

#[derive(Debug, Serialize)]
pub struct OpenFolderResponse {
    pub path: Option<String>,
}

/// Stub: open a folder picker dialog.
#[command]
pub async fn open_folder(_title: Option<String>) -> CommandResult<OpenFolderResponse> {
    Ok(OpenFolderResponse { path: None })
}

// ─── Notification ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct NotifyRequest {
    pub title: String,
    pub body: String,
    /// Optional severity hint.
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

/// Stub: request a native OS notification.
#[command]
pub async fn notify(_req: NotifyRequest) -> CommandResult<NotifyResponse> {
    // Real implementation uses `tauri_plugin_notification::Notification`.
    Ok(NotifyResponse {
        acknowledged: false,
    })
}

// ─── Settings (local, non-secret) ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GetSettingRequest {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum SettingValue {
    Text(String),
    Flag(bool),
    Number(f64),
}

#[derive(Debug, Serialize)]
pub struct GetSettingResponse {
    pub value: Option<SettingValue>,
}

/// Stub: read a local setting by key.
#[command]
pub async fn get_setting(_req: GetSettingRequest) -> CommandResult<GetSettingResponse> {
    Ok(GetSettingResponse { value: None })
}

#[derive(Debug, Deserialize)]
pub struct SetSettingRequest {
    pub key: String,
    pub value: SettingValue,
}

#[derive(Debug, Serialize)]
pub struct SetSettingResponse {
    pub persisted: bool,
}

/// Stub: write a local setting by key.
#[command]
pub async fn set_setting(_req: SetSettingRequest) -> CommandResult<SetSettingResponse> {
    Ok(SetSettingResponse { persisted: false })
}

// ─── Local Process Supervision (stencil only) ───────────────────────────────

/// The shell plugin is loaded at minimal baseline (`shell:default`) to allow
/// future process supervision without requiring a capability redesign.
/// `shell:default` grants only the `open` helper (launch default app for a URL/path).
/// No `shell:execute` or `shell:kill` permissions are active.
/// COE-404 will implement whitelisted executable paths, PID tracking, and
/// input sanitization before any execute/kill permissions are added.

#[derive(Debug, Serialize)]
pub struct ProcessStatus {
    pub pid: Option<u32>,
    pub running: bool,
}

/// Stub: query whether the locally-supervised daemon process is running.
#[command]
pub async fn daemon_status() -> CommandResult<ProcessStatus> {
    // COE-404 will implement actual discovery + supervision.
    Ok(ProcessStatus {
        pid: None,
        running: false,
    })
}

// ─── Gateway Transport Commands (COE-410) ───────────────────────────────────
//
// These commands implement the desktop local transport adapter, allowing the
// Tauri webview frontend to communicate with the local OpenSymphony gateway
// using the same GatewayEnvelope and schema types as HTTP/WebSocket transports.
//
// Transport priority (per architecture doc 3.1):
// 1. In-process Rust channels (embedded host) - not available in webview
// 2. Native local IPC (Unix sockets/named pipes) - via loopback fallback
// 3. Tauri channels (this implementation) - high-volume frames to webview
// 4. Loopback HTTP/WebSocket - compatibility baseline

/// Gateway connection state managed by the Tauri app.
#[derive(Debug, Clone)]
pub struct GatewayConnection {
    pub base_url: String,
    pub auth_token: Option<String>,
    pub connected: bool,
}

impl Default for GatewayConnection {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8000".to_string(),
            auth_token: None,
            connected: false,
        }
    }
}

/// Request to attach to a local gateway instance.
#[derive(Debug, Deserialize)]
pub struct AttachGatewayRequest {
    /// Gateway base URL (e.g., "http://127.0.0.1:8000").
    pub base_url: String,
    /// Optional auth token for hosted or secured gateways.
    pub auth_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AttachGatewayResponse {
    pub connected: bool,
    pub profile: String,
}

/// Attach to a local or remote gateway instance.
#[command]
pub async fn attach_gateway(
    _state: tauri::State<'_, GatewayConnection>,
    req: AttachGatewayRequest,
) -> CommandResult<AttachGatewayResponse> {
    // Validate URL
    if !req.base_url.starts_with("http://") && !req.base_url.starts_with("https://") {
        return Err(DesktopError::GatewayError {
            message: "Invalid gateway URL".to_string(),
        });
    }

    // Determine profile based on URL
    let is_loopback = req.base_url.contains("127.0.0.1") || req.base_url.contains("localhost");
    let profile = if is_loopback {
        "loopback_http"
    } else {
        "websocket"
    };

    Ok(AttachGatewayResponse {
        connected: true,
        profile: profile.to_string(),
    })
}

/// Gateway capabilities response (mirrors gateway-schema).
#[derive(Debug, Serialize, Deserialize)]
pub struct GatewayCapabilitiesResponse {
    pub schema_version: String,
    pub auth_modes: Vec<String>,
    pub transports: Vec<ProfileTransportCapability>,
    pub features: Vec<ProfileFeatureCapability>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileTransportCapability {
    pub transport: String,
    pub supported: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileFeatureCapability {
    pub feature: String,
    pub enabled: bool,
}

/// Query gateway capabilities.
#[command]
pub async fn gateway_health(
    _state: tauri::State<'_, GatewayConnection>,
) -> CommandResult<GatewayCapabilitiesResponse> {
    // Stub: return default capabilities for local mode
    Ok(GatewayCapabilitiesResponse {
        schema_version: "v1".to_string(),
        auth_modes: vec!["none".to_string()],
        transports: vec![
            ProfileTransportCapability {
                transport: "loopback_http".to_string(),
                supported: true,
            },
            ProfileTransportCapability {
                transport: "loopback_websocket".to_string(),
                supported: true,
            },
            ProfileTransportCapability {
                transport: "tauri_channel".to_string(),
                supported: true,
            },
        ],
        features: vec![
            ProfileFeatureCapability {
                feature: "event_journal".to_string(),
                enabled: true,
            },
            ProfileFeatureCapability {
                feature: "cursor_replay".to_string(),
                enabled: true,
            },
        ],
    })
}

/// Get dashboard snapshot from gateway.
#[command]
pub async fn dashboard_snapshot(
    _state: tauri::State<'_, GatewayConnection>,
) -> CommandResult<serde_json::Value> {
    // Stub: will be wired to actual gateway in COE-404
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "projects": [],
        "runs": [],
        "events": [],
    }))
}

/// Get task graph for a project.
#[command]
pub async fn task_graph(
    _state: tauri::State<'_, GatewayConnection>,
    project_id: String,
) -> CommandResult<serde_json::Value> {
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "project_id": project_id,
        "nodes": [],
        "root_ids": [],
    }))
}

/// Get run details.
#[command]
pub async fn run_detail(
    _state: tauri::State<'_, GatewayConnection>,
    run_id: String,
) -> CommandResult<serde_json::Value> {
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "run_id": run_id,
        "status": "idle",
        "events": [],
    }))
}

/// Get run events with cursor support.
#[command]
pub async fn run_events(
    _state: tauri::State<'_, GatewayConnection>,
    run_id: String,
    cursor: Option<u64>,
    page_size: Option<u64>,
) -> CommandResult<serde_json::Value> {
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "run_id": run_id,
        "cursor": cursor.unwrap_or(0),
        "page_size": page_size.unwrap_or(100),
        "events": [],
        "has_more": false,
    }))
}

/// Get terminal snapshot.
#[command]
pub async fn terminal_snapshot(
    _state: tauri::State<'_, GatewayConnection>,
    run_id: String,
    terminal_id: String,
) -> CommandResult<serde_json::Value> {
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "run_id": run_id,
        "terminal_id": terminal_id,
        "content": "",
        "cursor": 0,
    }))
}

/// Connection profile for local gateway discovery.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub name: String,
    pub profile_type: String,
    pub base_url: String,
    pub auth_mode: String,
    pub available: bool,
}

/// Get available connection profiles for the desktop app.
#[command]
pub async fn get_connection_profiles() -> CommandResult<Vec<ConnectionProfile>> {
    Ok(vec![
        ConnectionProfile {
            name: "Local Daemon".to_string(),
            profile_type: "loopback_http".to_string(),
            base_url: "http://127.0.0.1:8000".to_string(),
            auth_mode: "none".to_string(),
            available: true,
        },
        ConnectionProfile {
            name: "Local Gateway (WebSocket)".to_string(),
            profile_type: "loopback_websocket".to_string(),
            base_url: "ws://127.0.0.1:8000".to_string(),
            auth_mode: "none".to_string(),
            available: true,
        },
        ConnectionProfile {
            name: "Tauri Native".to_string(),
            profile_type: "tauri_channel".to_string(),
            base_url: "tauri://local".to_string(),
            auth_mode: "none".to_string(),
            available: true,
        },
    ])
}

// ─── Gateway Local Stream Transport (COE-410) ──────────────────────────────

/// Schema version used in all gateway payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SchemaVersion {
    pub fn v1() -> Self {
        Self {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
}

/// Stream cursor for replay and resumable subscriptions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamCursor {
    pub sequence: u64,
    pub partition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_anchor: Option<u64>,
}

/// Entity reference in gateway envelopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRef {
    pub kind: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
}

/// Gateway event envelope — the shared contract across all transport profiles.
/// Local and remote transports must produce identical envelope shapes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayEnvelope {
    pub schema_version: SchemaVersion,
    pub cursor: StreamCursor,
    pub entity_ref: EntityRef,
    pub event_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_payload: Option<serde_json::Value>,
    pub emitted_at: String,
}

/// Health capability response for the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayCapabilities {
    pub schema_version: SchemaVersion,
    pub gateway_version: String,
    pub supported_api_versions: Vec<String>,
    pub transports: Vec<GatewayTransportCapability>,
    pub features: Vec<GatewayFeatureCapability>,
    pub auth_modes: Vec<String>,
    pub max_event_page_size: usize,
    pub max_terminal_frame_batch: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayTransportCapability {
    pub transport: String,
    pub modes: Vec<String>,
    pub supported_encodings: Vec<String>,
    pub bidirectional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayFeatureCapability {
    pub feature: String,
    pub available: bool,
    pub requires_auth: bool,
}

/// Request to subscribe to the gateway event stream via Tauri channel.
#[derive(Debug, Deserialize)]
pub struct SubscribeEventsRequest {
    /// Optional cursor to resume from (sequence number).
    pub cursor: Option<u64>,
}

/// Request to subscribe to terminal frames for a specific run.
#[derive(Debug, Deserialize)]
pub struct SubscribeTerminalRequest {
    pub run_id: String,
    /// Optional cursor to resume from (sequence number).
    pub cursor: Option<u64>,
}

/// Terminal frame payload for high-throughput local streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalFrame {
    pub schema_version: SchemaVersion,
    pub frame_sequence: u64,
    pub stream_id: String,
    pub run_id: String,
    pub terminal_session_id: String,
    pub frame_kind: String,
    pub encoding: String,
    pub content: String,
    pub timestamp: String,
}

/// Gateway health status.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum GatewayHealthStatus {
    #[serde(rename = "healthy")]
    Healthy,
    #[serde(rename = "degraded")]
    Degraded,
    #[serde(rename = "unavailable")]
    Unavailable,
}

/// Local gateway connection info.
#[derive(Debug, Serialize)]
pub struct GatewayConnectionInfo {
    pub status: GatewayHealthStatus,
    pub profile: String,
    pub base_uri: String,
    pub transports: Vec<String>,
}

/// Query gateway capabilities.
/// Used by the frontend transport factory to select the optimal profile.
#[command]
pub async fn gateway_capabilities() -> CommandResult<GatewayCapabilities> {
    Ok(GatewayCapabilities {
        schema_version: SchemaVersion::v1(),
        gateway_version: env!("CARGO_PKG_VERSION").to_string(),
        supported_api_versions: vec!["1.0.0".to_string()],
        transports: vec![
            GatewayTransportCapability {
                transport: "tauri_channel".to_string(),
                modes: vec!["json".to_string()],
                supported_encodings: vec!["utf-8".to_string()],
                bidirectional: true,
            },
            GatewayTransportCapability {
                transport: "loopback_http".to_string(),
                modes: vec!["json".to_string()],
                supported_encodings: vec!["utf-8".to_string()],
                bidirectional: false,
            },
            GatewayTransportCapability {
                transport: "loopback_websocket".to_string(),
                modes: vec!["json".to_string(), "binary".to_string()],
                supported_encodings: vec!["utf-8".to_string(), "base64".to_string()],
                bidirectional: true,
            },
        ],
        features: vec![
            GatewayFeatureCapability {
                feature: "task_graph".to_string(),
                available: true,
                requires_auth: false,
            },
            GatewayFeatureCapability {
                feature: "terminal_stream".to_string(),
                available: true,
                requires_auth: false,
            },
        ],
        auth_modes: vec!["none".to_string(), "api_key".to_string()],
        max_event_page_size: 1000,
        max_terminal_frame_batch: 500,
    })
}

/// Query the local gateway health and connection info.
#[command]
pub async fn gateway_connection_info() -> CommandResult<GatewayConnectionInfo> {
    // COE-404 will implement actual gateway discovery.
    // For now, report that the local gateway is available via fallback.
    Ok(GatewayConnectionInfo {
        status: GatewayHealthStatus::Healthy,
        profile: "loopback_http".to_string(),
        base_uri: "http://localhost:8080".to_string(),
        transports: vec![
            "tauri_channel".to_string(),
            "loopback_http".to_string(),
            "loopback_websocket".to_string(),
        ],
    })
}

/// Subscribe to the gateway event stream via a Tauri channel.
///
/// This provides a high-throughput, zero-copy path from the local gateway
/// to the webview frontend. The channel carries GatewayEnvelope instances
/// that are identical in structure to those delivered via HTTP/SSE or
/// WebSocket transports.
///
/// The caller provides a `tauri::ipc::Channel` through which envelopes
/// are streamed. This enables backpressure handling and avoids the
/// HTTP/WebSocket overhead for local desktop mode.
#[command]
pub async fn subscribe_events(
    _req: SubscribeEventsRequest,
    _tx: tauri::ipc::Channel<GatewayEnvelope>,
) -> CommandResult<()> {
    // COE-409 will wire this to the actual gateway event stream.
    // The channel transport enables high-throughput local delivery.
    Ok(())
}

/// Subscribe to terminal frames for a specific run via a Tauri channel.
///
/// Terminal frames are high-volume and benefit from the zero-copy-friendly
/// Rust frame buffer path. This command establishes a dedicated channel
/// for terminal streaming.
#[command]
pub async fn subscribe_terminal(
    _req: SubscribeTerminalRequest,
    _tx: tauri::ipc::Channel<GatewayEnvelope>,
) -> CommandResult<()> {
    // COE-409 will wire this to the actual gateway terminal stream.
    Ok(())
}

/// Unsubscribe from the gateway event stream.
/// Clean up the channel and release resources.
#[command]
pub async fn unsubscribe_events() -> CommandResult<()> {
    Ok(())
}

/// Unsubscribe from terminal frame streaming.
#[command]
pub async fn unsubscribe_terminal(_run_id: String) -> CommandResult<()> {
    Ok(())
}

