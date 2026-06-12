//! OpenHands runtime state mirror for run detail views.
//!
//! The runtime mirror is the single source of truth that:
//!
//! - Tracks runtime attachment lifecycle (REST history sync → readiness barrier
//!   → WebSocket connect → reconcile → terminal/detach).
//! - Maps incoming OpenHands activity into normalized OpenSymphony liveness
//!   phases (`active`, `quiet`, `degraded`, `stalled`, `detached`, `terminal`).
//! - Emits structured [`RuntimeProgressSnapshot`]s so the gateway, scheduler, and
//!   event journal have a single consistent view of progress.
//! - Replaces hard total-turn timeout behavior with **progress-based idle
//!   detection**: the sliding deadline in `StallMetadata` never trips as long as
//!   the harness keeps emitting progress signals (events, status changes,
//!   token bumps). Only concrete idle silence escalates to `Stalled`.

use crate::opensymphony_domain::{
    ConversationId, DetachMetadata, DetachReason, DurationMs, HistorySyncStatus, LivenessState,
    ReconnectStatus, RuntimeLivenessPhase, RuntimeProgressSnapshot, StallMetadata, StreamHealth,
    TimestampMs,
};

use super::{
    events::{ConversationStateMirror, EventCache},
    models::{Conversation, EventEnvelope},
};

/// Synthetic `last_event_id` tag emitted when a status change advances the
/// cursor without a tightly matching event id. Operators can correlate this
/// with `last_event_kind` to understand what triggered the change.
pub const NO_EVENT_CURSOR_MARKER: &str = "runtime://status-change";

/// Configuration for a [`RuntimeMirror`].
#[derive(Debug, Clone)]
pub struct MirrorConfig {
    /// Idle timeout for `StallMetadata`: any silence longer than this advances
    /// toward the stalled phase.
    pub idle_timeout_ms: Option<DurationMs>,
    /// Total runtime cap; independent of progress-based detection. Anchored to
    /// `started_at`.
    pub total_runtime_cap_ms: Option<DurationMs>,
    /// After this idle window the run transitions from `RunningTurn` to `Quiet`
    /// (without yet escalating to `Stalled`).
    pub quiet_window_ms: Option<DurationMs>,
    /// Window after which the run is considered degraded if no events arrived
    /// at all since the previous snapshot (covers `/run` conflicts and other
    /// repeated endpoint failures).
    pub degrade_after_ms: Option<DurationMs>,
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self {
            idle_timeout_ms: Some(DurationMs::new(300_000)),
            total_runtime_cap_ms: None,
            quiet_window_ms: Some(DurationMs::new(60_000)),
            degrade_after_ms: Some(DurationMs::new(180_000)),
        }
    }
}

/// Single-run runtime state mirror.
///
/// Holds the canonical [`EventCache`] (REST + WS), the computed
/// [`ConversationStateMirror`], and the [`StallMetadata`] whose idle deadline
/// slides forward on every progress signal.
///
/// Clone is intentionally absent so mutations are linear and bookkeeping cannot
/// accidentally fork the state on the way through.
#[derive(Debug)]
pub struct RuntimeMirror {
    config: MirrorConfig,
    conversation_id: ConversationId,
    started_at: TimestampMs,
    state_mirror: ConversationStateMirror,
    event_cache: EventCache,
    stall: StallMetadata,
    stream_health: StreamHealth,
    history_sync_status: HistorySyncStatus,
    reconnect_status: ReconnectStatus,
    last_event_id: Option<String>,
    last_event_kind: Option<String>,
    last_event_at: Option<TimestampMs>,
    last_logical_event_at: Option<TimestampMs>,
    detach_metadata: Option<DetachMetadata>,
}

impl RuntimeMirror {
    /// Construct a new mirror for a freshly created conversation.
    pub fn new(
        conversation_id: ConversationId,
        started_at: TimestampMs,
        config: MirrorConfig,
    ) -> Self {
        let idle_timeout = config.idle_timeout_ms.unwrap_or(QUIET_SAFE_IDLE_FLOOR);
        // Quiet must be a *strict precursor* to Stalled: when quiet_window_ms
        // >= idle_timeout_ms the Quiet band collapses to zero width and the
        // phase precedence logic below can no longer surface Quiet. Clamp the
        // operator-supplied value so the precedence invariant holds.
        let config = clamp_quiet_window(config, idle_timeout);
        let stall =
            StallMetadata::with_runtime_cap(started_at, idle_timeout, config.total_runtime_cap_ms);
        Self {
            config,
            conversation_id,
            started_at,
            state_mirror: ConversationStateMirror::default(),
            event_cache: EventCache::new(),
            stall,
            stream_health: StreamHealth::Unknown,
            history_sync_status: HistorySyncStatus::Idle,
            reconnect_status: ReconnectStatus::Connected,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_logical_event_at: None,
            detach_metadata: None,
        }
    }

    /// Apply an initial conversation snapshot obtained from REST `get_conversation`.
    pub fn apply_initial_conversation_snapshot(&mut self, conversation: &Conversation) {
        self.state_mirror.apply_conversation(conversation);
        self.history_sync_status = HistorySyncStatus::InProgress;
        if self.state_mirror.raw_state().get("stats").is_some() {
            self.history_sync_status = HistorySyncStatus::Synced;
        }
    }

    /// Apply a REST history payload. Events recorded here share the same
    /// backing cache as WebSocket events, so a later WebSocket replay dedupes
    /// against this materialization.
    pub fn apply_rest_history<I>(&mut self, history: I) -> Vec<EventEnvelope>
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        let events: Vec<_> = history.into_iter().collect();
        let applied = self.event_cache.merge_new_events(events.clone());
        for event in &applied {
            self.state_mirror.apply_event(event);
        }
        if let Some(last) = applied.last() {
            self.cursor_from_event(last);
            self.slide_deadline(timestamp_for_event(last));
        }
        // Whether all events were new or some were already cached, REST replay
        // has materialised the conversation history. Subsequent websocket
        // traffic will dedupe against this materialization.
        self.history_sync_status = HistorySyncStatus::Synced;
        applied
    }

    /// Mark the readiness barrier as passed.
    pub fn apply_socket_ready(&mut self, now: TimestampMs) {
        self.stream_health = StreamHealth::Ready;
        self.reconnect_status = ReconnectStatus::Connected;
        self.slide_deadline(now);
    }

    /// Mark the WebSocket as disconnected.
    pub fn apply_socket_disconnected(&mut self, reason: &str, now: TimestampMs) {
        self.stream_health = StreamHealth::Disconnected;
        self.reconnect_status = ReconnectStatus::Pending;
        self.history_sync_status = HistorySyncStatus::Stale;
        let hint = if reason.is_empty() {
            "socket disconnected".to_string()
        } else {
            reason.to_string()
        };
        self.attach_state_change(&hint);
        self.slide_deadline(now);
    }

    /// Mark a reconnect attempt as scheduled (with the next retry backoff).
    pub fn apply_reconnect_pending(&mut self) {
        self.stream_health = StreamHealth::Reconnecting;
        self.reconnect_status = ReconnectStatus::Pending;
    }

    /// Mark the WebSocket as successfully reconnected and reconcile missed events.
    pub fn apply_reconnect_succeeded<I>(
        &mut self,
        replayed: I,
        now: TimestampMs,
    ) -> Vec<EventEnvelope>
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        let events: Vec<_> = replayed.into_iter().collect();
        let applied = self.event_cache.merge_new_events(events.clone());
        for event in &applied {
            self.state_mirror.apply_event(event);
        }
        if let Some(last) = applied.last() {
            self.cursor_from_event(last);
        }
        self.stream_health = StreamHealth::Ready;
        self.reconnect_status = ReconnectStatus::Connected;
        self.history_sync_status = HistorySyncStatus::Synced;
        self.slide_deadline(now);
        applied
    }

    /// Record a reconnect exhausted outcome.
    pub fn apply_reconnect_exhausted(&mut self, now: TimestampMs) {
        self.reconnect_status = ReconnectStatus::Exhausted;
        self.stream_health = StreamHealth::Failed;
        self.slide_deadline(now);
    }

    /// Apply a single WebSocket event.
    ///
    /// Returns `true` if the event was newly inserted (false on dedupe).
    pub fn apply_event(&mut self, event: &EventEnvelope) -> bool {
        let event_at = timestamp_for_event(event);
        let inserted = self.event_cache.insert(event.clone());
        if !inserted {
            self.slide_deadline(event_at);
            return false;
        }
        self.state_mirror.apply_event(event);
        self.cursor_from_event(event);
        self.slide_deadline(event_at);
        true
    }

    /// Observe a runtime-reported execution status change without an event.
    pub fn apply_status_change(&mut self, status: &str, now: TimestampMs) {
        self.state_mirror
            .apply_conversation_execution_status(&synthetic_conversation(status));
        self.attach_state_change(status);
        self.slide_deadline(now);
    }

    /// Observe a token usage bump (typically derived from an LLM completion log).
    /// Counts are deltas since the last call and they are merged into the
    /// state mirror's raw statistics blob so subsequent snapshots reflect
    /// the new totals. All three token buckets (prompt / completion /
    /// cache-read) are forwarded so callers don't silently drop cache read
    /// counts.
    pub fn apply_token_update(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        now: TimestampMs,
    ) {
        self.state_mirror
            .apply_token_counts(input_tokens, output_tokens, cache_read_tokens);
        self.slide_deadline(now);
    }

    /// Mark the run as terminal with the actual OpenHands execution status
    /// (`finished`, `error`, `stuck`, etc). The status is forwarded to the
    /// state mirror so downstream liveness and diagnostic phases observe the
    /// real reason — never collapse to a hardcoded `finished`.
    pub fn apply_terminal(&mut self, status: &str, summary: &str, now: TimestampMs) {
        self.state_mirror
            .apply_conversation_execution_status(&synthetic_conversation(status));
        self.attach_state_change(summary);
        self.slide_deadline(now);
    }

    /// Mark the stream as failed (transport-level failure).
    pub fn apply_stream_failure(&mut self, now: TimestampMs) {
        self.stream_health = StreamHealth::Failed;
        self.attach_state_change("stream_failure");
        self.slide_deadline(now);
    }

    /// Mark the run as detached (worker lost ownership, runtime is unrecoverable).
    pub fn apply_detach(&mut self, reason: DetachReason, summary: String, now: TimestampMs) {
        let prev_status = self.state_mirror.execution_status().map(str::to_string);
        let metadata = DetachMetadata {
            reason,
            detached_at: now,
            last_execution_status: prev_status,
            summary,
        };
        self.detach_metadata = Some(metadata);
        self.stream_health = StreamHealth::Detached;
        // Detached override supersedes inactive state changes but stays inside the
        // bookkeeping so event_count / cursor remain meaningful for diagnostics.
        self.attach_state_change("detached");
        self.slide_deadline(now);
    }

    fn attach_state_change(&mut self, reason: &str) {
        self.last_event_kind = Some(reason.to_string());
        self.last_event_id = Some(format!("{NO_EVENT_CURSOR_MARKER}/{reason}"));
    }

    /// Slide the progress-based idle deadline forward.
    fn slide_deadline(&mut self, now: TimestampMs) {
        if now.as_u64() == 0 {
            return;
        }
        self.stall.observe_activity(now);
        if self.last_logical_event_at.is_none() {
            self.last_logical_event_at = Some(now);
        }
        if self.state_mirror.execution_status().is_none() {
            self.state_mirror
                .apply_conversation_execution_status(&synthetic_conversation("running"));
        }
    }

    /// Build the current snapshot at `now`.
    ///
    /// The caller-supplied timestamp is required because the mirror cannot
    /// read wall-clock time on its own; using `last_logical_event_at` here
    /// would mask `Quiet`/`Stalled`/`Degraded` transitions that depend on
    /// the elapsed-since-last-activity comparison that
    /// [`RuntimeMirror::phase_at`] performs.
    pub fn snapshot_at(&self, now: TimestampMs) -> RuntimeProgressSnapshot {
        self.build_snapshot(now)
    }

    /// Backwards-compatible shorthand for [`RuntimeMirror::snapshot_at`] that
    /// pins `now` to the timestamp of the last observed activity. **This
    /// always reports `RunningTurn` / `WaitingOnPriorTurn`** because the
    /// elapsed-since-last-activity delta collapses to zero — prefer
    /// [`RuntimeMirror::snapshot_at`] whenever silence matters.
    pub fn snapshot(&self) -> RuntimeProgressSnapshot {
        let at = self.last_logical_event_at.unwrap_or(self.started_at);
        self.build_snapshot(at)
    }

    fn build_snapshot(&self, at: TimestampMs) -> RuntimeProgressSnapshot {
        let phase = self.phase_at(at);
        let stall_deadline_at = if self.stall.stalled_at.as_u64() == 0 {
            None
        } else {
            Some(self.stall.stalled_at)
        };
        let (input_tokens, output_tokens, cache_read_tokens) = self
            .state_mirror
            .accumulated_token_usage()
            .unwrap_or((0, 0, 0));
        RuntimeProgressSnapshot::initial(phase)
            .update_with(phase)
            .with_event_count(self.event_cache.items().len() as u64)
            .with_input_tokens(input_tokens)
            .with_output_tokens(output_tokens)
            .with_cache_read_tokens(cache_read_tokens)
            .with_execution_status(Some(
                self.state_mirror
                    .execution_status()
                    .unwrap_or("")
                    .to_string(),
            ))
            .with_stream_health(self.stream_health)
            .with_history_sync_status(self.history_sync_status)
            .with_reconnect_status(self.reconnect_status)
            .with_last_activity_at(Some(self.stall.last_activity_at))
            .with_stall_deadline_at(stall_deadline_at)
            .with_last_event_cursor(self.last_event_id.clone())
            .with_last_event_kind(self.last_event_kind.clone())
            .with_last_event_at(self.last_event_at)
            .with_detach_metadata(self.detach_metadata.clone())
            .build()
    }

    /// Compute the current phase from mirror state.
    pub fn phase(&self) -> RuntimeLivenessPhase {
        let at = self.last_logical_event_at.unwrap_or(self.started_at);
        self.phase_at(at)
    }

    /// Compute the current six-state aggregation.
    pub fn liveness_state(&self) -> LivenessState {
        self.phase().liveness_state()
    }

    /// Derive a phase from explicit inputs (used in tests and when projecting
    /// the phase onto a historical timestamp).
    pub fn phase_at(&self, now: TimestampMs) -> RuntimeLivenessPhase {
        if self.detach_metadata.is_some() {
            return RuntimeLivenessPhase::Detached;
        }
        if self.state_mirror.terminal_status().is_some() {
            return RuntimeLivenessPhase::Terminal;
        }
        if matches!(
            self.stream_health,
            StreamHealth::Reconnecting | StreamHealth::Disconnected | StreamHealth::HistorySyncing
        ) {
            return RuntimeLivenessPhase::Reconciling;
        }
        if matches!(self.reconnect_status, ReconnectStatus::Exhausted)
            || matches!(self.stream_health, StreamHealth::Failed)
        {
            return RuntimeLivenessPhase::Degraded;
        }
        if matches!(
            self.stream_health,
            StreamHealth::Attaching | StreamHealth::Unknown
        ) {
            return RuntimeLivenessPhase::WaitingOnPriorTurn;
        }
        // Quiet precedes Stalled on purpose: the docstring on [`RuntimeLivenessPhase::Quiet`]
        // promises that we let a quiet-but-not-yet-stalled run surface new events before we
        // declare it stalled. We also guard against configs where `quiet_window >=
        // idle_timeout` so the operator cannot accidentally collapse Quiet into Stalled.
        let quiet_window = self
            .config
            .quiet_window_ms
            .unwrap_or(DurationMs::new(0))
            .as_u64();
        let idle_timeout = self
            .config
            .idle_timeout_ms
            .unwrap_or(QUIET_SAFE_IDLE_FLOOR)
            .as_u64();
        // Quiet is the band *between* the quiet_window mark and the stall mark —
        // once we cross idle_timeout the linear envelope transitions to Stalled.
        let quiet_in_range = quiet_window > 0
            && quiet_window < idle_timeout
            && now
                >= self
                    .stall
                    .last_activity_at
                    .saturating_add(DurationMs::new(quiet_window))
            && !self.stall.is_stalled_at(now);
        if quiet_in_range {
            return RuntimeLivenessPhase::Quiet;
        }
        if self.stall.is_stalled_at(now) {
            return RuntimeLivenessPhase::Stalled;
        }
        RuntimeLivenessPhase::RunningTurn
    }

    fn cursor_from_event(&mut self, event: &EventEnvelope) {
        self.last_event_id = Some(event.id.clone());
        self.last_event_kind = Some(event.kind.clone());
        self.last_event_at = Some(timestamp_for_event(event));
        self.last_logical_event_at = Some(timestamp_for_event(event));
    }

    /// Conversation id this mirror tracks.
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    /// Total observed event count.
    pub fn observed_event_count(&self) -> u64 {
        self.event_cache.items().len() as u64
    }

    /// Current last event cursor (`event_id`).
    pub fn last_event_cursor(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    /// Internal getter used by tests for assertions around stall timing.
    pub fn stall_metadata(&self) -> StallMetadata {
        self.stall
    }

    /// Stream health visible to consumers.
    pub fn stream_health(&self) -> StreamHealth {
        self.stream_health
    }

    /// History sync status visible to consumers.
    pub fn history_sync_status(&self) -> HistorySyncStatus {
        self.history_sync_status
    }

    /// Reconnect status visible to consumers.
    pub fn reconnect_status(&self) -> ReconnectStatus {
        self.reconnect_status
    }

    /// Provides direct read access to the inner `ConversationStateMirror`
    /// for payloads that need its raw_state.
    pub fn conversation_mirror(&self) -> &ConversationStateMirror {
        &self.state_mirror
    }
}

const QUIET_SAFE_IDLE_FLOOR: DurationMs = DurationMs::new(86_400_000);

/// Clamp `quiet_window_ms` so it is strictly less than `idle_timeout_ms` and
/// remain zero-width by default when no idle timeout was supplied. The
/// returned config matches the input on every other field.
fn clamp_quiet_window(mut config: MirrorConfig, idle_timeout: DurationMs) -> MirrorConfig {
    let Some(quiet) = config.quiet_window_ms else {
        return config;
    };
    if quiet.as_u64() < idle_timeout.as_u64() {
        return config;
    }
    // Reserve at least 1 ms so the Quiet band is non-empty; if even 1 ms is
    // too aggressive (idle_timeout already at 0), suppress Quiet entirely.
    let clamped = idle_timeout
        .as_u64()
        .saturating_sub(1)
        .max(if quiet.as_u64() == 0 { 0 } else { 1 });
    config.quiet_window_ms = Some(DurationMs::new(clamped));
    config
}

fn timestamp_for_event(event: &EventEnvelope) -> TimestampMs {
    TimestampMs::new(event.timestamp.timestamp_millis().max(0) as u64)
}

fn synthetic_conversation(status: &str) -> Conversation {
    use crate::opensymphony_openhands::{
        AgentConfig, ConfirmationPolicy, Conversation, LlmConfig, WorkspaceConfig,
    };
    use uuid::Uuid;
    Conversation {
        conversation_id: Uuid::nil(),
        workspace: WorkspaceConfig {
            working_dir: "/tmp/synthetic".to_string(),
            kind: "LocalWorkspace".to_string(),
        },
        persistence_dir: "/tmp/synthetic/persistence".to_string(),
        max_iterations: 0,
        stuck_detection: false,
        execution_status: status.to_string(),
        confirmation_policy: ConfirmationPolicy {
            kind: "NeverConfirm".to_string(),
        },
        agent: AgentConfig {
            kind: "Agent".to_string(),
            llm: LlmConfig {
                model: "synthetic".to_string(),
                api_key: None,
                base_url: None,
                usage_id: None,
            },
            condenser: None,
            tools: None,
            include_default_tools: None,
        },
        stats: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_openhands::{
        AgentConfig, ConfirmationPolicy, Conversation, LlmConfig, WorkspaceConfig,
    };
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uuid::Uuid;

    fn conversation_with_status(status: &str) -> Conversation {
        Conversation {
            conversation_id: Uuid::nil(),
            workspace: WorkspaceConfig {
                working_dir: "/tmp/conv".to_string(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: "/tmp/conv/persistence".to_string(),
            max_iterations: 4,
            stuck_detection: true,
            execution_status: status.to_string(),
            confirmation_policy: ConfirmationPolicy {
                kind: "NeverConfirm".to_string(),
            },
            agent: AgentConfig {
                kind: "Agent".to_string(),
                llm: LlmConfig {
                    model: "openai/gpt-5.4".to_string(),
                    api_key: None,
                    base_url: None,
                    usage_id: None,
                },
                condenser: None,
                tools: None,
                include_default_tools: None,
            },
            stats: None,
        }
    }

    fn idle_config(idle_ms: u64) -> MirrorConfig {
        MirrorConfig {
            idle_timeout_ms: Some(DurationMs::new(idle_ms)),
            total_runtime_cap_ms: None,
            quiet_window_ms: Some(DurationMs::new(idle_ms / 2)),
            degrade_after_ms: None,
        }
    }

    fn mirror_with_config(idle_ms: u64) -> RuntimeMirror {
        RuntimeMirror::new(
            ConversationId::new("conv-200").expect("valid id"),
            TimestampMs::new(1_000),
            idle_config(idle_ms),
        )
    }

    fn runtime_event(id: &str, kind: &str, timestamp_ms: u64) -> EventEnvelope {
        let dt = Utc.timestamp_millis_opt(timestamp_ms as i64).unwrap();
        EventEnvelope::new(id, dt, "runtime", kind, json!({}))
    }

    fn runtime_state_update(id: &str, status: &str, timestamp_ms: u64) -> EventEnvelope {
        let dt = Utc.timestamp_millis_opt(timestamp_ms as i64).unwrap();
        let stamp_value = status.to_string();
        EventEnvelope::new(
            id,
            dt,
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": stamp_value,
                "state_delta": { "execution_status": stamp_value },
            }),
        )
    }

    fn user_message_event(id: &str, timestamp_ms: u64, text: &str) -> EventEnvelope {
        let dt = Utc.timestamp_millis_opt(timestamp_ms as i64).unwrap();
        EventEnvelope::new(
            id,
            dt,
            "user",
            "MessageEvent",
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": text }]
            }),
        )
    }

    #[test]
    fn new_mirror_starts_in_unknown_state() {
        let mirror = mirror_with_config(300_000);
        let snap = mirror.snapshot();
        assert!(matches!(
            snap.phase,
            RuntimeLivenessPhase::WaitingOnPriorTurn | RuntimeLivenessPhase::RunningTurn
        ));
        assert!(matches!(snap.stream_health, StreamHealth::Unknown));
        assert_eq!(snap.event_count, 0);
        assert!(matches!(snap.liveness_state, LivenessState::Active));
    }

    #[test]
    fn ready_then_event_slides_stall_deadline() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("idle"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        let snap = mirror.snapshot();
        assert!(matches!(snap.phase, RuntimeLivenessPhase::RunningTurn));
        assert!(matches!(snap.stream_health, StreamHealth::Ready));
        assert_eq!(snap.last_event_cursor.as_deref(), Some("evt-1"));
        let deadline = snap.stall_deadline_at.expect("deadline");
        assert!(deadline.as_u64() >= 1_500 + 2_000);
    }

    #[test]
    fn long_running_progress_keeps_running_turn() {
        // 300 ms idle timeout; emit an event every 100 ms for 600 ms.
        let mut mirror = mirror_with_config(300);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        let ready_at = 1_000_u64;
        mirror.apply_socket_ready(TimestampMs::new(ready_at));

        let mut now = ready_at;
        let end = ready_at + 600;
        let mut progress_count = 0;
        while now <= end {
            let id = format!("evt-{now}");
            mirror.apply_event(&runtime_event(&id, "MessageEvent", now));
            progress_count += 1;
            now += 100;
        }
        let snap = mirror.snapshot();
        assert_eq!(progress_count, 7);
        assert!(
            !matches!(snap.phase, RuntimeLivenessPhase::Stalled),
            "long-running progress should not stall (phase was {:?})",
            snap.phase
        );
        assert!(matches!(snap.phase, RuntimeLivenessPhase::RunningTurn));
    }

    #[test]
    fn silence_progresses_through_quiet_then_stalled() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));

        let last_event_at = mirror.last_logical_event_at.expect("set");
        let quiet_window = 1_000_u64;
        // quiet window = half of idle_timeout_ms = 1_000; advance 1_300 ms later
        let quiet_now = last_event_at.as_u64() + quiet_window + 300;
        assert!(matches!(
            mirror.phase_at(TimestampMs::new(quiet_now)),
            RuntimeLivenessPhase::Quiet
        ));

        let stalled_now = last_event_at.as_u64() + 2_500;
        assert!(matches!(
            mirror.phase_at(TimestampMs::new(stalled_now)),
            RuntimeLivenessPhase::Stalled
        ));
    }

    #[test]
    fn detached_metadata_overrides_phase_to_detached() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_detach(
            DetachReason::Unreachable,
            "lost ownership".to_string(),
            TimestampMs::new(2_000),
        );
        let phase = mirror.phase();
        assert!(matches!(phase, RuntimeLivenessPhase::Detached));
        let snap = mirror.snapshot();
        assert!(matches!(snap.liveness_state, LivenessState::Detached));
        assert!(snap.detach_metadata.is_some());
    }

    #[test]
    fn reconcile_progress_collapses_rest_then_ws_replay() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));

        let rest_events = vec![runtime_event("evt-rest", "MessageEvent", 1_500)];
        let applied = mirror.apply_rest_history(rest_events);
        assert_eq!(applied.len(), 1);
        assert_eq!(mirror.observed_event_count(), 1);

        let ws_replay = vec![runtime_event("evt-rest", "MessageEvent", 1_500)];
        let ws_applied = mirror.apply_reconnect_succeeded(ws_replay, TimestampMs::new(2_000));
        assert!(ws_applied.is_empty(), "duplicate ids must dedupe");
        assert_eq!(mirror.observed_event_count(), 1);
        assert_eq!(mirror.history_sync_status(), HistorySyncStatus::Synced);
    }

    #[test]
    fn reconnect_exhausted_transitions_to_degraded() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_socket_disconnected("ws closed", TimestampMs::new(1_500));
        mirror.apply_reconnect_pending();
        mirror.apply_reconnect_exhausted(TimestampMs::new(2_500));
        let snap = mirror.snapshot();
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Degraded));
        assert!(matches!(snap.stream_health, StreamHealth::Failed));
        assert!(matches!(snap.reconnect_status, ReconnectStatus::Exhausted));
    }

    #[test]
    fn terminal_status_transitions_to_terminal_phase() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_event(&runtime_state_update("evt-finished", "finished", 1_900));
        let snap = mirror.snapshot();
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Terminal));
        assert!(matches!(snap.liveness_state, LivenessState::Terminal));
    }

    #[test]
    fn apply_terminal_propagates_supplied_status_not_hardcoded_finished() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_terminal("error", "openhands tripwire", TimestampMs::new(1_900));
        let snap = mirror.snapshot();
        assert_eq!(
            snap.execution_status.as_deref(),
            Some("error"),
            "apply_terminal must forward the actual terminal status into the state mirror"
        );
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Terminal));
    }

    #[test]
    fn stream_failure_drives_degraded_phase() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_stream_failure(TimestampMs::new(2_000));
        let snap = mirror.snapshot();
        assert!(matches!(snap.stream_health, StreamHealth::Failed));
        // A stream failure with no terminal reporting doesn't mark terminal; it's degraded.
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Degraded));
    }

    #[test]
    fn state_update_event_propagates_execution_status_into_snapshot() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("idle"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_state_update("evt-1", "running", 1_500));
        let snap = mirror.snapshot();
        assert_eq!(snap.execution_status.as_deref(), Some("running"));
    }

    #[test]
    fn status_change_advances_cursor_with_marker() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("idle"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_status_change("running", TimestampMs::new(1_500));
        let snap = mirror.snapshot();
        assert!(
            snap.last_event_cursor
                .as_deref()
                .unwrap_or_default()
                .starts_with(NO_EVENT_CURSOR_MARKER)
        );
        assert_eq!(snap.last_event_kind.as_deref(), Some("running"));
    }

    #[test]
    fn token_only_progress_slides_deadline() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_100));

        let mut now = 1_500;
        let mut last_deadline = mirror
            .snapshot()
            .stall_deadline_at
            .expect("deadline")
            .as_u64();
        for _ in 0..5 {
            mirror.apply_token_update(100, 50, 10, TimestampMs::new(now));
            let current = mirror
                .snapshot()
                .stall_deadline_at
                .expect("deadline")
                .as_u64();
            assert!(current >= last_deadline, "deadline should slide forward");
            last_deadline = current;
            now += 100;
        }
    }

    #[test]
    fn dedupe_replayed_event_keeps_unique_count() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        let envelope = user_message_event("evt-dup", 1_500, "hi again");
        assert!(mirror.apply_event(&envelope));
        assert!(!mirror.apply_event(&envelope));
        assert_eq!(mirror.observed_event_count(), 1);
    }

    #[test]
    fn prior_turn_wait_visible_via_attaching_stream_health() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status(
            "waiting_on_prior_turn",
        ));
        let snap = mirror.snapshot();
        assert!(matches!(
            snap.phase,
            RuntimeLivenessPhase::WaitingOnPriorTurn
        ));
    }

    #[test]
    fn unknown_event_advances_cursor() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        let envelope = EventEnvelope::new(
            "evt-mystery",
            Utc.timestamp_millis_opt(1_500).unwrap(),
            "runtime",
            "BrandNewEventType",
            json!({ "structure": "future" }),
        );
        assert!(mirror.apply_event(&envelope));
        let snap = mirror.snapshot();
        assert_eq!(snap.last_event_cursor.as_deref(), Some("evt-mystery"));
        assert_eq!(snap.last_event_kind.as_deref(), Some("BrandNewEventType"));
    }

    #[test]
    fn stream_disconnect_forces_history_to_stale() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        let rest = vec![runtime_event("evt-rest", "MessageEvent", 1_500)];
        mirror.apply_rest_history(rest);
        assert_eq!(mirror.history_sync_status(), HistorySyncStatus::Synced);

        mirror.apply_socket_disconnected("closed", TimestampMs::new(2_000));
        assert_eq!(mirror.history_sync_status(), HistorySyncStatus::Stale);
        assert!(matches!(mirror.stream_health(), StreamHealth::Disconnected));
        assert!(matches!(
            mirror.reconnect_status(),
            ReconnectStatus::Pending
        ));
    }

    #[test]
    fn snapshot_reports_liveness_state_aggregation() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        let snap = mirror.snapshot();
        assert!(matches!(snap.liveness_state, LivenessState::Active));
    }
}
