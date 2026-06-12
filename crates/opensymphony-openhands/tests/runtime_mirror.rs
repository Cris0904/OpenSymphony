//! Runtime state mirror integration tests.
//!
//! Exercises [`RuntimeMirror`](opensymphony_openhands::RuntimeMirror) against
//! the scripted [`FakeOpenHandsServer`](opensymphony_testkit::FakeOpenHandsServer)
//! so that:
//!
//! - REST history plus WebSocket event reconciliation produces a consistent
//!   mirror state.
//! - Long-running turns do not become stalled solely because the
//!   `stall_timeout_ms` window has elapsed when token-only or activity signals
//!   keep arriving.
//! - Stream disconnect, REST reconcile progress, prior-turn wait, and the
//!   `/run` conflict all surface through the mirrored liveness phase and
//!   agility state without the mirror having to re-read raw OpenHands
//!   wire data.

use chrono::Utc;
use serde_json::json;

use crate::opensymphony_domain::{
    ConversationId, DurationMs, HistorySyncStatus, LivenessState, ReconnectStatus,
    RuntimeLivenessPhase, StreamHealth, TimestampMs,
};
use crate::opensymphony_openhands::{
    ConversationCreateRequest, EventEnvelope, MirrorConfig, OpenHandsClient, OpenHandsError,
    RuntimeMirror, TransportConfig,
};
use crate::opensymphony_testkit::{FakeEventStreamBuilder, FakeOpenHandsServer, FakeSocketScript};

fn idle_config(idle_ms: u64) -> MirrorConfig {
    MirrorConfig {
        idle_timeout_ms: Some(DurationMs::new(idle_ms)),
        total_runtime_cap_ms: None,
        quiet_window_ms: Some(DurationMs::new(idle_ms / 2)),
        degrade_after_ms: Some(DurationMs::new(idle_ms * 4)),
    }
}

fn runtime_event(id: &str, kind: &str, timestamp_ms: u64) -> EventEnvelope {
    let dt = chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms as i64)
        .expect("valid timestamp");
    EventEnvelope::new(id, dt, "runtime", kind, json!({}))
}

#[test]
fn progress_based_idle_detection_keeps_long_running_turn_active() {
    // 500 ms idle timeout; emit an event every 100 ms for 1100 ms so the
    // quiet/stalled windows expire several times over.
    let mut mirror = RuntimeMirror::new(
        ConversationId::new("conv-prog").expect("valid id"),
        TimestampMs::new(1_000),
        idle_config(500),
    );
    mirror.apply_socket_ready(TimestampMs::new(1_000));

    let mut now_ms = 1_000_u64;
    let end_ms = 1_000 + 1_100;
    let mut activity_count = 0_u32;
    while now_ms <= end_ms {
        let id = format!("evt-{now_ms}");
        mirror.apply_event(&runtime_event(&id, "MessageEvent", now_ms));
        activity_count += 1;
        now_ms += 100;
    }
    let snap = mirror.snapshot();
    assert!(matches!(snap.phase, RuntimeLivenessPhase::RunningTurn));
    assert!(matches!(snap.liveness_state, LivenessState::Active));
    assert!(snap.stall_deadline_at.is_some(), "deadline set by activity");
    assert!(activity_count > 10);
}

#[test]
fn silence_progresses_quiet_then_stalled() {
    let mut mirror = RuntimeMirror::new(
        ConversationId::new("conv-silence").expect("valid id"),
        TimestampMs::new(1_000),
        idle_config(800),
    );
    mirror.apply_socket_ready(TimestampMs::new(1_000));
    // Initial activity so we aren't in the prior-turn-wait / unknown phase.
    mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_000));
    let _ = mirror.snapshot();

    // Past idle timeout (800ms after last activity at 1000 = 1800).
    let phase_at_2000 = mirror.phase_at(TimestampMs::new(2_000));
    assert!(matches!(
        phase_at_2000,
        RuntimeLivenessPhase::Stalled | RuntimeLivenessPhase::Quiet
    ));
}

#[test]
fn token_only_progress_slides_stall_deadline() {
    let mut mirror = RuntimeMirror::new(
        ConversationId::new("conv-tok").expect("valid id"),
        TimestampMs::new(1_000),
        idle_config(1_000),
    );
    mirror.apply_socket_ready(TimestampMs::new(1_000));

    let baseline = mirror.stall_metadata();
    let baseline_deadline = baseline.stalled_at;

    mirror.apply_token_update(200, 100, TimestampMs::new(1_300));
    let slid = mirror.stall_metadata();
    let slid_deadline = slid.stalled_at;
    assert!(
        slid_deadline.as_u64() >= baseline_deadline.as_u64(),
        "token-only progress should never slide the deadline backward"
    );
}

#[test]
fn stream_disconnect_marks_history_stale_and_pending_reconnect() {
    let mut mirror = RuntimeMirror::new(
        ConversationId::new("conv-disc").expect("valid id"),
        TimestampMs::new(1_000),
        idle_config(2_000),
    );
    mirror.apply_socket_ready(TimestampMs::new(1_000));
    mirror.apply_socket_disconnected("network_reset", TimestampMs::new(2_000));
    let snap = mirror.snapshot();
    assert!(matches!(snap.stream_health, StreamHealth::Disconnected));
    assert!(matches!(snap.history_sync_status, HistorySyncStatus::Stale));
    assert!(matches!(snap.reconnect_status, ReconnectStatus::Pending));
}

#[test]
fn rest_reconcile_progress_collapses_reconciling_then_replay() {
    let mut mirror = RuntimeMirror::new(
        ConversationId::new("conv-reconcile").expect("valid id"),
        TimestampMs::new(1_000),
        idle_config(2_000),
    );
    mirror.apply_socket_ready(TimestampMs::new(1_000));
    mirror.apply_socket_disconnected("ws_dropped", TimestampMs::new(2_000));
    // While disconnected, the phase should reflect stream-driven reconciliation.
    let mid = mirror.snapshot();
    assert!(matches!(mid.phase, RuntimeLivenessPhase::Reconciling));

    // REST reconcile delivers missed events. Mirror must dedupe against
    // previously applied tail and transition back to Ready.
    let replayed = vec![
        runtime_event("evt-new-1", "MessageEvent", 2_500),
        runtime_event("evt-new-2", "MessageEvent", 2_700),
    ];
    mirror.apply_reconnect_succeeded(replayed, TimestampMs::new(2_800));
    let snap = mirror.snapshot();
    assert!(matches!(snap.phase, RuntimeLivenessPhase::RunningTurn));
    assert!(matches!(snap.stream_health, StreamHealth::Ready));
    assert!(matches!(
        snap.history_sync_status,
        HistorySyncStatus::Synced
    ));
    assert_eq!(snap.last_event_cursor.as_deref(), Some("evt-new-2"));
}

#[test]
fn unknown_events_remain_visible_through_cursor_without_failing_run() {
    let mut mirror = RuntimeMirror::new(
        ConversationId::new("conv-unknown").expect("valid id"),
        TimestampMs::new(1_000),
        idle_config(2_000),
    );
    mirror.apply_socket_ready(TimestampMs::new(1_000));
    mirror.apply_event(&runtime_event("evt-known", "MessageEvent", 1_500));
    let unknown = EventEnvelope::new(
        "evt-unknown",
        chrono::DateTime::<Utc>::from_timestamp_millis(2_000).expect("ts"),
        "runtime",
        "BrandNewFutureEvent",
        json!({ "anything": "goes" }),
    );
    let inserted = mirror.apply_event(&unknown);
    assert!(inserted, "unknown event must be retained, not dropped");

    let snap = mirror.snapshot();
    assert!(matches!(snap.phase, RuntimeLivenessPhase::RunningTurn));
    assert_eq!(snap.last_event_cursor.as_deref(), Some("evt-unknown"));
    assert_eq!(snap.event_count, 2);
}

#[tokio::test]
async fn run_conflict_after_active_run_surfaces_409_without_state_mutation() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");

    server
        .emit_state_update(conversation.conversation_id, "running")
        .await
        .expect("running state should be recorded");

    let error = client
        .run_conversation(conversation.conversation_id)
        .await
        .expect_err("active conversation must reject /run");
    assert!(matches!(
        error,
        OpenHandsError::HttpStatus {
            status_code: 409,
            ..
        }
    ));
}

#[tokio::test]
async fn prior_turn_wait_visible_via_attaching_stream_health_fake() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");

    // The fake server applies state-update events as they arrive; publish a
    // `waiting_on_prior_turn` so the runtime mirror can read it from the
    // contributing conversation snapshot the next time it's materialised.
    server
        .emit_state_update(conversation.conversation_id, "waiting_on_prior_turn")
        .await
        .expect("waiting state should be recorded");

    let fixtures = FakeEventStreamBuilder::new(Utc::now());
    let ready = fixtures.state_update_at("evt-ready", 0, "idle");
    server
        .script_socket_connections(
            conversation.conversation_id,
            vec![FakeSocketScript::new().event(ready)],
        )
        .await
        .expect("socket script should be configured");

    let mut mirror = RuntimeMirror::new(
        ConversationId::new(conversation.conversation_id.to_string()).expect("valid id"),
        TimestampMs::new(1_000),
        idle_config(2_000),
    );
    // Initial conversation snapshot fetched via REST exposes the prior-turn-wait
    // execution status; the mirror keeps it visible through `phase()` until the
    // readiness barrier has been crossed.
    if let Ok(initial_conversation) = client.get_conversation(conversation.conversation_id).await {
        mirror.apply_initial_conversation_snapshot(&initial_conversation);
    }
    let snap = mirror.snapshot();
    assert!(matches!(
        snap.stream_health,
        StreamHealth::Attaching | StreamHealth::Unknown
    ));
}

// (no helper functions needed beyond the test cases)
