/**
 * Run Detail page.
 *
 * Renders run summary, event timeline placeholder, workspace metadata,
 * harness metadata, action capability bar, diff placeholder, validation
 * placeholder, long-running run phase/liveness, stream health, and
 * safe action availability.
 */

import type { RunDetail as RunDetailType, RunStatus, ReleaseReason } from "@opensymphony/gateway-schema";
import type { Page } from "../types/navigation";
import { RunStatusBadge } from "../components/RunStatusBadge";
import {
  formatTimeAgo,
  formatDuration,
  formatDateTime,
  LIVENESS_COLORS,
} from "../lib/ui-utils";

interface RunDetailProps {
  runId: string;
  navigate: (page: Page) => void;
}

// Fixture run data covering various states.
const fixtureRuns: Record<string, RunDetailType> = {
  "run-001": {
    schema_version: { major: 1, minor: 0, patch: 0 },
    run_id: "run-001",
    issue_id: "406107c4-99f7-4993-ae54-5ba822fab6f8",
    issue_identifier: "COE-402",
    worker_id: "worker-alpha-01",
    status: "running",
    claimed_at: "2025-06-05T18:00:00Z",
    started_at: "2025-06-05T18:01:00Z",
    finished_at: undefined,
    release_reason: undefined,
    turn_count: 12,
    max_turns: 30,
    retry_attempt: 0,
    input_tokens: 45_000,
    output_tokens: 28_000,
    cache_read_tokens: 12_000,
    runtime_seconds: 7200,
    conversation_id: "conv-001",
    workspace_path: "/workspace/COE-402",
    error: undefined,
  },
  "run-002": {
    schema_version: { major: 1, minor: 0, patch: 0 },
    run_id: "run-002",
    issue_id: "coe-434-id",
    issue_identifier: "COE-434",
    worker_id: "worker-beta-03",
    status: "retry_queued",
    claimed_at: "2025-06-05T10:00:00Z",
    started_at: "2025-06-05T10:05:00Z",
    finished_at: "2025-06-05T12:00:00Z",
    release_reason: "tracker_inactive",
    turn_count: 8,
    max_turns: 30,
    retry_attempt: 2,
    input_tokens: 30_000,
    output_tokens: 15_000,
    cache_read_tokens: 8_000,
    runtime_seconds: 3600,
    conversation_id: "conv-002",
    workspace_path: "/workspace/COE-434",
    error: "Harness session became unresponsive after 30 minutes of silence",
  },
  "run-003": {
    schema_version: { major: 1, minor: 0, patch: 0 },
    run_id: "run-003",
    issue_id: "coe-392-id",
    issue_identifier: "COE-392",
    worker_id: "worker-alpha-02",
    status: "running",
    claimed_at: "2025-06-05T19:00:00Z",
    started_at: "2025-06-05T19:02:00Z",
    finished_at: undefined,
    release_reason: undefined,
    turn_count: 5,
    max_turns: 30,
    retry_attempt: 0,
    input_tokens: 12_000,
    output_tokens: 8_000,
    cache_read_tokens: 3_000,
    runtime_seconds: 1800,
    conversation_id: "conv-003",
    workspace_path: "/workspace/COE-392",
    error: undefined,
  },
};

// Long-running run phase and liveness state.
const fixtureLivenessState: Record<string, {
  phase: string;
  liveness: "active" | "quiet" | "degraded" | "stalled" | "detached";
  lastProgress: string;
  streamHealth: "healthy" | "intermittent" | "lost";
  lastActivityAt: string;
  harnessAttached: boolean;
  detachedReason?: string;
}> = {
  "run-001": {
    phase: "code_generation",
    liveness: "active",
    lastProgress: "Generating implementation plan for AppShell component...",
    streamHealth: "healthy",
    lastActivityAt: new Date(Date.now() - 5000).toISOString(),
    harnessAttached: true,
  },
  "run-002": {
    phase: "waiting",
    liveness: "detached",
    lastProgress: "Retry queued - underlying harness work may still be active or detached",
    streamHealth: "lost",
    lastActivityAt: new Date(Date.now() - 7200000).toISOString(),
    harnessAttached: false,
    detachedReason: "Harness session terminated unexpectedly",
  },
  "run-003": {
    phase: "validation",
    liveness: "quiet",
    lastProgress: "Running test suite... waiting for completion",
    streamHealth: "intermittent",
    lastActivityAt: new Date(Date.now() - 120000).toISOString(),
    harnessAttached: true,
  },
};

export function RunDetail({ runId, navigate }: RunDetailProps): React.ReactElement {
  const run = fixtureRuns[runId];
  const liveness = fixtureLivenessState[runId];

  if (!run) {
    return (
      <div style={{ padding: "var(--space-4)", textAlign: "center", color: "var(--color-fg-muted)" }}>
        <p style={{ fontSize: "16px", marginBottom: "var(--space-3)" }}>Run not found</p>
        <button
          onClick={() => navigate({ kind: "dashboard" })}
          style={{
            background: "var(--color-accent)",
            border: "none",
            color: "#fff",
            padding: "var(--space-2) var(--space-4)",
            borderRadius: "var(--radius-md)",
            cursor: "pointer",
          }}
        >
          Back to Dashboard
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {/* Header */}
      <header>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-3)" }}>
          <button
            onClick={() => navigate({ kind: "dashboard" })}
            style={{
              background: "none",
              border: "none",
              color: "var(--color-fg-muted)",
              cursor: "pointer",
              padding: "var(--space-1)",
              fontSize: "16px",
            }}
          >
            ←
          </button>
          <div>
            <h1 style={{ margin: 0, fontSize: "20px", fontWeight: 600 }}>
              Run Detail: {run.run_id}
            </h1>
            <p style={{ margin: "4px 0 0", color: "var(--color-fg-muted)", fontSize: "13px" }}>
              {run.issue_identifier} — {run.worker_id}
            </p>
          </div>
          <div style={{ flex: 1 }} />
          <RunStatusBadge status={run.status} />
          {liveness && <LivenessBadge liveness={liveness.liveness} />}
        </div>
      </header>

      {/* Run summary */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Summary
        </h2>
        <div
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            overflow: "hidden",
          }}
        >
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 0 }}>
            <DetailRow label="Issue" value={run.issue_identifier} />
            <DetailRow label="Worker" value={run.worker_id} />
            <DetailRow label="Status" value={run.status} />
            <DetailRow
              label="Release Reason"
              value={run.release_reason ?? "—"}
            />
            <DetailRow label="Turns" value={`${run.turn_count} / ${run.max_turns}`} />
            <DetailRow
              label="Retry Attempt"
              value={run.retry_attempt !== undefined ? run.retry_attempt.toString() : "—"}
            />
            <DetailRow label="Started" value={run.started_at ? formatDateTime(run.started_at) : "—"} />
            <DetailRow label="Finished" value={run.finished_at ? formatDateTime(run.finished_at) : "—"} />
            <DetailRow label="Runtime" value={formatDuration(run.runtime_seconds)} />
            <DetailRow label="Conversation" value={run.conversation_id ?? "—"} />
          </div>
          {run.error && (
            <div
              style={{
                padding: "var(--space-3)",
                borderTop: "1px solid var(--color-border-default)",
                background: "rgba(248, 81, 73, 0.05)",
              }}
            >
              <span style={{ fontSize: "12px", color: "var(--color-fg-muted)" }}>Error:</span>
              <div style={{ fontSize: "13px", color: "var(--color-danger)", marginTop: "4px" }}>
                {run.error}
              </div>
            </div>
          )}
        </div>
      </section>

      {/* Liveness and stream health */}
      {liveness && (
        <section>
          <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
            Liveness & Stream Health
          </h2>
          <div
            style={{
              background: "var(--color-bg-secondary)",
              border: "1px solid var(--color-border-default)",
              borderRadius: "var(--radius-md)",
              padding: "var(--space-3)",
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))",
              gap: "var(--space-3)",
            }}
          >
            <LivenessInfo label="Phase" value={liveness.phase} />
            <LivenessInfo label="Liveness" value={liveness.liveness} colored />
            <LivenessInfo label="Stream Health" value={liveness.streamHealth} colored />
            <LivenessInfo label="Harness Attached" value={liveness.harnessAttached ? "Yes" : "No"} />
            <LivenessInfo label="Last Activity" value={formatTimeAgo(liveness.lastActivityAt)} />
            <LivenessInfo label="Last Progress" value={liveness.lastProgress} />
            {liveness.detachedReason && (
              <div
                style={{
                  gridColumn: "1 / -1",
                  padding: "var(--space-2) var(--space-3)",
                  background: "rgba(210, 153, 34, 0.1)",
                  border: "1px solid rgba(210, 153, 34, 0.3)",
                  borderRadius: "var(--radius-sm)",
                  fontSize: "12px",
                  color: "var(--color-attention)",
                }}
              >
                ⚠️ {liveness.detachedReason}
              </div>
            )}
          </div>
        </section>
      )}

      {/* Token usage */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Token Usage
        </h2>
        <div
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            padding: "var(--space-3)",
            display: "grid",
            gridTemplateColumns: "repeat(auto-fit, minmax(150px, 1fr))",
            gap: "var(--space-3)",
          }}
        >
          <TokenCard label="Input" value={run.input_tokens} />
          <TokenCard label="Output" value={run.output_tokens} />
          <TokenCard label="Cache Read" value={run.cache_read_tokens} />
          <TokenCard
            label="Total"
            value={run.input_tokens + run.output_tokens + run.cache_read_tokens}
          />
        </div>
      </section>

      {/* Workspace metadata */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Workspace
        </h2>
        <div
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            padding: "var(--space-3)",
          }}
        >
          <div style={{ fontSize: "13px", fontFamily: "var(--font-mono)", color: "var(--color-fg-muted)" }}>
            {run.workspace_path ?? "—"}
          </div>
        </div>
      </section>

      {/* Harness metadata */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Harness Metadata
        </h2>
        <div
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            padding: "var(--space-3)",
          }}
        >
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-2)", fontSize: "13px" }}>
            <span style={{ color: "var(--color-fg-muted)" }}>Worker ID:</span>
            <span>{run.worker_id}</span>
            <span style={{ color: "var(--color-fg-muted)" }}>Conversation ID:</span>
            <span>{run.conversation_id ?? "—"}</span>
            <span style={{ color: "var(--color-fg-muted)" }}>Max Turns:</span>
            <span>{run.max_turns}</span>
          </div>
        </div>
      </section>

      {/* Event timeline placeholder */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Event Timeline
        </h2>
        <div
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            padding: "var(--space-4)",
            textAlign: "center",
            color: "var(--color-fg-subtle)",
            fontSize: "13px",
          }}
        >
          📊 Event timeline placeholder — rendering coming soon
        </div>
      </section>

      {/* Action capability bar */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Actions
        </h2>
        <ActionBar run={run} liveness={liveness} />
      </section>

      {/* Diff placeholder */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Diff Viewer
        </h2>
        <div
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            padding: "var(--space-4)",
            textAlign: "center",
            color: "var(--color-fg-subtle)",
            fontSize: "13px",
          }}
        >
          📝 Diff viewer placeholder — file changes and diffs coming soon
        </div>
      </section>

      {/* Validation placeholder */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Validation
        </h2>
        <div
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            padding: "var(--space-4)",
            textAlign: "center",
            color: "var(--color-fg-subtle)",
            fontSize: "13px",
          }}
        >
          ✅ Validation results placeholder — test evidence and checks coming soon
        </div>
      </section>
    </div>
  );
}

/** Liveness badge for run state. */
function LivenessBadge({
  liveness,
}: {
  liveness: "active" | "quiet" | "degraded" | "stalled" | "detached";
}): React.ReactElement {
  const { bg, fg } = LIVENESS_COLORS[liveness] ?? LIVENESS_COLORS.quiet;

  return (
    <span
      style={{
        fontSize: "11px",
        fontWeight: 500,
        padding: "2px 8px",
        borderRadius: "10px",
        background: bg,
        color: fg,
        textTransform: "capitalize",
        display: "flex",
        alignItems: "center",
        gap: "4px",
      }}
    >
      {liveness === "active" && (
        <span
          style={{
            width: 6,
            height: 6,
            borderRadius: "50%",
            background: fg,
            display: "inline-block",
            animation: "pulse 2s infinite",
          }}
        />
      )}
      {liveness}
    </span>
  );
}

/** Detail row in summary table. */
function DetailRow({ label, value }: { label: string; value: string }): React.ReactElement {
  return (
    <div
      style={{
        display: "flex",
        justifyContent: "space-between",
        padding: "var(--space-2) var(--space-3)",
        borderBottom: "1px solid var(--color-border-muted)",
        fontSize: "13px",
      }}
    >
      <span style={{ color: "var(--color-fg-muted)" }}>{label}</span>
      <span>{value}</span>
    </div>
  );
}

/** Liveness info card. */
function LivenessInfo({
  label,
  value,
  colored,
}: {
  label: string;
  value: string;
  colored?: boolean;
}): React.ReactElement {
  const colorMap: Record<string, string> = {
    active: "var(--color-success)",
    quiet: "var(--color-fg-muted)",
    degraded: "var(--color-attention)",
    stalled: "var(--color-danger)",
    detached: "var(--color-fg-subtle)",
    healthy: "var(--color-success)",
    intermittent: "var(--color-attention)",
    lost: "var(--color-danger)",
  };

  return (
    <div>
      <div style={{ fontSize: "11px", color: "var(--color-fg-muted)", marginBottom: "2px" }}>
        {label}
      </div>
      <div
        style={{
          fontSize: "13px",
          color: colored && colorMap[value] ? colorMap[value] : "var(--color-fg-default)",
          textTransform: colored ? "capitalize" : undefined,
        }}
      >
        {value}
      </div>
    </div>
  );
}

/** Token usage card. */
function TokenCard({ label, value }: { label: string; value: number }): React.ReactElement {
  return (
    <div>
      <div style={{ fontSize: "11px", color: "var(--color-fg-muted)" }}>{label}</div>
      <div style={{ fontSize: "16px", fontWeight: 600, fontFamily: "var(--font-mono)" }}>
        {value.toLocaleString()}
      </div>
    </div>
  );
}

/** Action capability bar. */
function ActionBar({
  run,
  liveness,
}: {
  run: RunDetailType;
  liveness?: { liveness: string; phase: string };
}): React.ReactElement {
  const isRunning = run.status === "running";
  const isRetryQueued = run.status === "retry_queued";
  const isDetached = liveness?.liveness === "detached";

  return (
    <div style={{ display: "flex", gap: "var(--space-2)", flexWrap: "wrap" }}>
      {isRunning && (
        <ActionButton label="🛑 Cancel" danger />
      )}
      {isRetryQueued && (
        <>
          <ActionButton label="▶️ Retry Now" />
          <ActionButton label="🗑️ Remove from Queue" danger />
        </>
      )}
      {!isRunning && !isRetryQueued && (
        <ActionButton label="🔄 Restart" />
      )}
      {isDetached && (
        <ActionButton label="🔗 Reattach" />
      )}
      <ActionButton label="📋 View Logs" disabled />
      <ActionButton label="📊 View Metrics" disabled />
    </div>
  );
}

/** Action button component. */
function ActionButton({
  label,
  danger,
  disabled,
}: {
  label: string;
  danger?: boolean;
  disabled?: boolean;
}): React.ReactElement {
  return (
    <button
      style={{
        padding: "var(--space-2) var(--space-3)",
        background: danger ? "rgba(248, 81, 73, 0.15)" : disabled ? "var(--color-bg-tertiary)" : "var(--color-bg-tertiary)",
        border: `1px solid ${danger ? "rgba(248, 81, 73, 0.3)" : disabled ? "var(--color-border-muted)" : "var(--color-border-default)"}`,
        borderRadius: "var(--radius-md)",
        color: danger ? "var(--color-danger)" : disabled ? "var(--color-fg-subtle)" : "var(--color-fg-default)",
        cursor: disabled ? "not-allowed" : "pointer",
        fontSize: "12px",
        fontWeight: 500,
      }}
      disabled={disabled}
      tabIndex={disabled ? -1 : 0}
    >
      {label}
    </button>
  );
}
