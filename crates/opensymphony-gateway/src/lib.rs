use std::{convert::Infallible, path::Path, time::Duration};

use async_stream::stream;
use axum::{
    Json, Router,
    extract::{Path as ExtractPath, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use tokio::{net::TcpListener, sync::broadcast};

pub use crate::opensymphony_control::SnapshotStore;
pub use crate::opensymphony_domain::{
    ControlPlaneAgentServerStatus, ControlPlaneDaemonSnapshot, ControlPlaneDaemonState,
    ControlPlaneDaemonStatus, ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot,
    ControlPlaneMetricsSnapshot, ControlPlaneRecentEvent, ControlPlaneRecentEventKind,
    ControlPlaneWorkerOutcome, SnapshotEnvelope,
};
pub use crate::opensymphony_gateway_schema::{
    capability::{AuthMode, FeatureCapability, GatewayCapabilities, TransportCapability},
    snapshot::{
        DashboardSnapshot, GatewayHealth, GatewayMetrics, ProjectSummary, SnapshotEventKind,
        SnapshotEventSummary,
    },
    version::{GATEWAY_SCHEMA_VERSION, SchemaVersion},
};

const GATEWAY_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Combined state for the gateway router.
#[derive(Debug, Clone)]
struct GatewayState {
    store: SnapshotStore,
    /// Optional path to the built web app static assets directory.
    web_assets_dir: Option<String>,
}

/// V1 gateway server that exposes stable public DTO endpoints
/// on top of the internal control-plane `SnapshotStore`.
#[derive(Debug, Clone)]
pub struct GatewayServer {
    state: GatewayState,
}

impl GatewayServer {
    pub fn new(store: SnapshotStore) -> Self {
        Self {
            state: GatewayState {
                store,
                web_assets_dir: None,
            },
        }
    }

    /// Enable serving of the built web client from the given directory.
    /// The directory should contain the output of the Vite build
    /// (index.html, assets/, etc.).
    pub fn with_web_assets(mut self, dir: impl Into<String>) -> Self {
        self.state.web_assets_dir = Some(dir.into());
        self
    }

    pub fn router(&self) -> Router {
        let mut router = Router::new()
            .route("/api/v1/capabilities", get(capabilities))
            .route("/api/v1/dashboard/snapshot", get(dashboard_snapshot))
            .route("/api/v1/events", get(events));

        // Attach static web asset routes if configured.
        if self.state.web_assets_dir.is_some() {
            router = router
                .route("/app", get(web_asset_handler))
                .route("/app/", get(web_asset_handler))
                .route("/app/{*path}", get(web_asset_handler));
        }

        router.with_state(self.state.clone())
    }

    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        axum::serve(listener, self.router()).await
    }
}

/// Map internal control-plane state into the public dashboard snapshot DTO.
pub fn control_plane_to_dashboard_snapshot(envelope: &SnapshotEnvelope) -> DashboardSnapshot {
    let snapshot = &envelope.snapshot;
    let health = daemon_state_to_gateway_health(snapshot.daemon.state);
    let metrics = GatewayMetrics {
        running_issue_count: snapshot.metrics.running_issues,
        retry_queue_depth: snapshot.metrics.retry_queue_depth,
        total_input_tokens: snapshot.metrics.input_tokens,
        total_output_tokens: snapshot.metrics.output_tokens,
        total_cache_read_tokens: snapshot.metrics.cache_read_tokens,
        total_cost_micros: snapshot.metrics.total_cost_micros,
    };

    // For v1 we flatten all issues into a single synthetic project because the
    // control-plane does not yet expose per-project grouping.
    let projects = if snapshot.issues.is_empty() {
        Vec::new()
    } else {
        let running = snapshot
            .issues
            .iter()
            .filter(|i| matches!(i.runtime_state, ControlPlaneIssueRuntimeState::Running))
            .count() as u32;
        let completed = snapshot
            .issues
            .iter()
            .filter(|i| matches!(i.last_outcome, ControlPlaneWorkerOutcome::Completed))
            .count() as u32;
        let failed = snapshot
            .issues
            .iter()
            .filter(|i| matches!(i.last_outcome, ControlPlaneWorkerOutcome::Failed))
            .count() as u32;

        vec![ProjectSummary {
            project_id: "default".into(),
            name: "OpenSymphony".into(),
            milestone_count: 0,
            issue_count: snapshot.issues.len() as u32,
            running_count: running,
            completed_count: completed,
            failed_count: failed,
        }]
    };

    let recent_events = snapshot
        .recent_events
        .iter()
        .map(|e| SnapshotEventSummary {
            happened_at: e.happened_at,
            issue_identifier: e.issue_identifier.clone(),
            kind: recent_event_kind_to_snapshot_event_kind(&e.kind),
            summary: e.summary.clone(),
        })
        .collect();

    DashboardSnapshot {
        schema_version: SchemaVersion::v1(),
        generated_at: snapshot.generated_at,
        sequence: envelope.sequence,
        health,
        metrics,
        projects,
        recent_events,
    }
}

fn daemon_state_to_gateway_health(state: ControlPlaneDaemonState) -> GatewayHealth {
    match state {
        ControlPlaneDaemonState::Ready => GatewayHealth::Healthy,
        ControlPlaneDaemonState::Degraded => GatewayHealth::Degraded,
        ControlPlaneDaemonState::Starting => GatewayHealth::Starting,
        ControlPlaneDaemonState::Stopped => GatewayHealth::Failed,
    }
}

fn recent_event_kind_to_snapshot_event_kind(
    kind: &ControlPlaneRecentEventKind,
) -> SnapshotEventKind {
    match kind {
        ControlPlaneRecentEventKind::WorkerStarted => SnapshotEventKind::WorkerStarted,
        ControlPlaneRecentEventKind::WorkspacePrepared => SnapshotEventKind::WorkspacePrepared,
        ControlPlaneRecentEventKind::StreamAttached => SnapshotEventKind::StreamAttached,
        ControlPlaneRecentEventKind::SnapshotPublished => SnapshotEventKind::SnapshotPublished,
        ControlPlaneRecentEventKind::WorkerCompleted => SnapshotEventKind::WorkerCompleted,
        ControlPlaneRecentEventKind::RetryScheduled => SnapshotEventKind::RetryScheduled,
        ControlPlaneRecentEventKind::ClientAttached => SnapshotEventKind::ClientAttached,
        ControlPlaneRecentEventKind::ClientDetached => SnapshotEventKind::ClientDetached,
        ControlPlaneRecentEventKind::Warning => SnapshotEventKind::Warning,
    }
}

fn build_capabilities() -> GatewayCapabilities {
    GatewayCapabilities {
        schema_version: SchemaVersion::v1(),
        gateway_version: env!("CARGO_PKG_VERSION").into(),
        supported_api_versions: vec!["1.0.0".into()],
        transports: vec![
            TransportCapability {
                transport: "sse".into(),
                modes: vec!["snapshot".into()],
                supported_encodings: vec!["utf-8".into(), "base64".into()],
                bidirectional: false,
            },
            TransportCapability {
                transport: "websocket".into(),
                modes: vec!["json".into(), "binary".into()],
                supported_encodings: vec!["utf-8".into(), "base64".into()],
                bidirectional: true,
            },
            TransportCapability {
                transport: "http".into(),
                modes: vec!["rest".into()],
                supported_encodings: vec!["utf-8".into()],
                bidirectional: false,
            },
        ],
        features: vec![
            FeatureCapability {
                feature: "task_graph".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "run_detail".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "event_journal".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "terminal_stream".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "action_dispatch".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "planning".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "approval".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "rehydrate".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "linear_sync".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "openhands_harness".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "codex_harness".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "hosted_mode".into(),
                // Hosted mode is not yet available; login implementation is
                // tracked as a follow-up (out of scope for this PR).
                available: false,
                requires_auth: true,
                requires_plan: None,
            },
        ],
        auth_modes: vec![AuthMode::None, AuthMode::ApiKey],
        max_event_page_size: 1000,
        max_terminal_frame_batch: 500,
    }
}

async fn capabilities() -> Json<GatewayCapabilities> {
    Json(build_capabilities())
}

async fn dashboard_snapshot(State(state): State<GatewayState>) -> Json<DashboardSnapshot> {
    let envelope = state.store.current().await;
    Json(control_plane_to_dashboard_snapshot(&envelope))
}

async fn events(
    State(state): State<GatewayState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = state.store.subscribe();
    let initial = state.store.current().await;
    let store_clone = state.store.clone();
    let stream = stream! {
        let mut last_sent_sequence = initial.sequence;
        yield Ok(snapshot_event(&initial));
        while let Some(envelope) =
            next_snapshot_envelope(&store_clone, &mut receiver, &mut last_sent_sequence).await
        {
            yield Ok(snapshot_event(&envelope));
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(GATEWAY_KEEPALIVE_INTERVAL)
            .text("keepalive"),
    )
}

fn snapshot_event(envelope: &SnapshotEnvelope) -> Event {
    let dashboard = control_plane_to_dashboard_snapshot(envelope);
    let payload =
        serde_json::to_string(&dashboard).expect("DashboardSnapshot is always serializable");
    Event::default()
        .event("snapshot")
        .id(envelope.sequence.to_string())
        .data(payload)
}

async fn next_snapshot_envelope(
    store: &SnapshotStore,
    receiver: &mut broadcast::Receiver<SnapshotEnvelope>,
    last_sent_sequence: &mut u64,
) -> Option<SnapshotEnvelope> {
    loop {
        match receiver.recv().await {
            Ok(envelope) => {
                if envelope.sequence <= *last_sent_sequence {
                    continue;
                }
                *last_sent_sequence = envelope.sequence;
                return Some(envelope);
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                if let Some(envelope) = latest_from_store(store, *last_sent_sequence).await {
                    *last_sent_sequence = envelope.sequence;
                    return Some(envelope);
                }
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

async fn latest_from_store(
    store: &SnapshotStore,
    last_sent_sequence: u64,
) -> Option<SnapshotEnvelope> {
    let latest = store.current().await;
    (latest.sequence > last_sent_sequence).then_some(latest)
}

// ---------------------------------------------------------------------------
// Web client static asset serving
// ---------------------------------------------------------------------------

/// Resolve the requested path and verify it stays inside the assets directory.
/// Returns the resolved absolute path if safe, or `None` if the request is
/// outside the assets directory.
fn resolve_safe_path(assets_dir: &str, rest: &str) -> Option<std::path::PathBuf> {
    // Reject absolute paths early to avoid Path::new().join() discarding the base.
    if rest.starts_with('/') {
        return None;
    }

    let base = Path::new(assets_dir);
    let candidate = base.join(rest);
    match (candidate.canonicalize(), base.canonicalize()) {
        (Ok(resolved), Ok(base_resolved)) => {
            if resolved == base_resolved || resolved.starts_with(&base_resolved) {
                Some(resolved)
            } else {
                None
            }
        }
        // If canonicalize fails (file doesn't exist), do a static check.
        // Reject path traversal via both forward and backslash separators.
        _ => {
            let has_dotdot = candidate
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir));
            if has_dotdot { None } else { Some(candidate) }
        }
    }
}

/// Serve `index.html` from the given assets directory, returning `None` if not found.
async fn serve_index_html(assets_dir: &str) -> Option<Response> {
    let index_path = Path::new(assets_dir).join("index.html");
    serve_file(&index_path).await.ok()
}

/// Serve a static file from the web assets directory, or fall back to
/// `index.html` for SPA routes.
async fn web_asset_handler(
    State(state): State<GatewayState>,
    path: Option<ExtractPath<String>>,
) -> Response {
    // If web assets are not configured, return 404.
    let assets_dir = match &state.web_assets_dir {
        Some(dir) => dir,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let rest = path.map(|p| p.0).unwrap_or_default();

    // If the path is empty (root /app/), serve index.html directly.
    if rest.is_empty() {
        return serve_index_html(assets_dir)
            .await
            .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response());
    }

    // Resolve the joined path and verify it stays inside the assets directory.
    let safe_path = match resolve_safe_path(assets_dir, &rest) {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Try the exact file path first.
    if safe_path.is_file() {
        return match serve_file(&safe_path).await {
            Ok(resp) => resp,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    }

    // SPA fallback: if the path does not look like a static asset request,
    // serve index.html so client-side routing works.
    if !path_has_known_extension(&rest) {
        return serve_index_html(assets_dir)
            .await
            .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response());
    }

    StatusCode::NOT_FOUND.into_response()
}

/// Return true if the URL path segment looks like a request for a known static
/// asset file.  Paths that do not match these extensions are treated as SPA
/// routes and should fall back to `index.html`.
fn path_has_known_extension(path: &str) -> bool {
    if let Some(dot_pos) = path.rfind('.')
        && let Some(ext) = path.get(dot_pos + 1..)
    {
        return matches!(
            ext.to_lowercase().as_str(),
            "html"
                | "css"
                | "js"
                | "json"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "svg"
                | "ico"
                | "woff"
                | "woff2"
                | "ttf"
                | "eot"
                | "otf"
                | "map"
                | "txt"
                | "xml"
                | "webp"
                | "mp4"
                | "webm"
                | "mp3"
                | "wav"
                | "flac"
                | "pdf"
                | "zip"
                | "gz"
                | "tar"
                | "bz2"
        );
    }
    false
}

/// Read a file from disk and return it as an HTTP response with the correct
/// content type.
async fn serve_file(path: &Path) -> Result<Response, std::io::Error> {
    let bytes = tokio::fs::read(path).await?;
    let content_type = mime_type(path);
    Ok(([(axum::http::header::CONTENT_TYPE, content_type)], bytes).into_response())
}

/// Return a conservative MIME type for the given file extension.
fn mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("eot") => "application/vnd.ms-fontobject",
        Some("otf") => "font/otf",
        Some("map") => "application/json; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        Some(_) | None => "application/octet-stream",
    }
}
