/**
 * Dashboard page.
 *
 * Renders gateway health, active runs, queue depth, retries,
 * recent events, and cost/token summary.
 */

import type {
  GatewayHealth,
  DashboardSnapshot,
  SnapshotEventSummary,
  ProjectSummary,
} from "@opensymphony/gateway-schema";
import type { Page } from "../types/navigation";
import {
  formatTimeAgo,
  formatTokens,
  formatCost,
  RUN_STATUS_COLORS,
} from "../lib/ui-utils";

interface DashboardProps {
  navigate: (page: Page) => void;
}

// Fixture data for read-only gateway mode.
const fixtureSnapshot: DashboardSnapshot = {
  schema_version: { major: 1, minor: 0, patch: 0 },
  generated_at: new Date().toISOString(),
  sequence: 42,
  health: "healthy",
  metrics: {
    running_issue_count: 3,
    retry_queue_depth: 1,
    total_input_tokens: 125_000,
    total_output_tokens: 89_000,
    total_cache_read_tokens: 45_000,
    total_cost_micros: 1_250_000,
  },
  projects: [
    {
      project_id: "4b7bc834-ffad-4beb-bd63-4d79cd6c4f3a",
      name: "OpenSymphony-bootstrap",
      milestone_count: 7,
      issue_count: 24,
      running_count: 3,
      completed_count: 18,
      failed_count: 2,
    },
  ],
  recent_events: [
    {
      happened_at: new Date(Date.now() - 60_000).toISOString(),
      issue_identifier: "COE-402",
      kind: "worker_started",
      summary: "Worker started for COE-402",
    },
    {
      happened_at: new Date(Date.now() - 120_000).toISOString(),
      issue_identifier: "COE-394",
      kind: "worker_completed",
      summary: "COE-394 completed successfully",
    },
    {
      happened_at: new Date(Date.now() - 180_000).toISOString(),
      issue_identifier: "COE-434",
      kind: "retry_scheduled",
      summary: "Retry scheduled for COE-434",
    },
    {
      happened_at: new Date(Date.now() - 240_000).toISOString(),
      kind: "warning",
      summary: "Gateway connection degraded, reconnecting...",
    },
    {
      happened_at: new Date(Date.now() - 300_000).toISOString(),
      issue_identifier: "COE-392",
      kind: "client_attached",
      summary: "Client attached to COE-392 run",
    },
  ],
};

export function Dashboard({ navigate }: DashboardProps): React.ReactElement {
  const snapshot = fixtureSnapshot;
  const { metrics, projects, recent_events } = snapshot;

  return (
    <div className="flex flex-col gap-4">
      {/* Header */}
      <header>
        <h1 style={{ margin: 0, fontSize: "20px", fontWeight: 600 }}>
          Dashboard
        </h1>
        <p style={{ margin: "4px 0 0", color: "var(--color-fg-muted)", fontSize: "13px" }}>
          Gateway health, active runs, and recent events
        </p>
      </header>

      {/* Health indicators */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          System Health
        </h2>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))", gap: "var(--space-3)" }}>
          <HealthCard
            title="Gateway"
            health={snapshot.health}
            icon="🔌"
          />
          <HealthCard
            title="Harness Pool"
            health={metrics.running_issue_count > 0 ? "healthy" : "starting"}
            icon="⚙️"
          />
          <HealthCard
            title="Linear Sync"
            health="healthy"
            icon="📋"
          />
        </div>
      </section>

      {/* Metrics summary */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Metrics
        </h2>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(150px, 1fr))", gap: "var(--space-3)" }}>
          <MetricCard label="Active Runs" value={metrics.running_issue_count} />
          <MetricCard label="Retry Queue" value={metrics.retry_queue_depth} />
          <MetricCard label="Input Tokens" value={formatTokens(metrics.total_input_tokens)} />
          <MetricCard label="Output Tokens" value={formatTokens(metrics.total_output_tokens)} />
          <MetricCard label="Cache Hits" value={formatTokens(metrics.total_cache_read_tokens)} />
          <MetricCard label="Total Cost" value={formatCost(metrics.total_cost_micros)} />
        </div>
      </section>

      {/* Projects */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Projects
        </h2>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))", gap: "var(--space-3)" }}>
          {projects.map((project) => (
            <ProjectCard
              key={project.project_id}
              project={project}
              onClick={() => navigate({ kind: "task-graph", projectId: project.project_id })}
            />
          ))}
        </div>
      </section>

      {/* Active runs */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Active Runs
        </h2>
        <RunList navigate={navigate} />
      </section>

      {/* Recent events */}
      <section>
        <h2 style={{ fontSize: "14px", fontWeight: 600, margin: "0 0 var(--space-3)" }}>
          Recent Events
        </h2>
        <div
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            overflow: "hidden",
          }}
        >
          {recent_events.map((event, idx) => (
            <EventRow
              key={idx}
              event={event}
              isLast={idx === recent_events.length - 1}
            />
          ))}
        </div>
      </section>
    </div>
  );
}

/** Health indicator card. */
function HealthCard({
  title,
  health,
  icon,
}: {
  title: string;
  health: GatewayHealth;
  icon: string;
}): React.ReactElement {
  const color = {
    healthy: "var(--color-success)",
    degraded: "var(--color-attention)",
    failed: "var(--color-danger)",
    starting: "var(--color-fg-subtle)",
  }[health];

  return (
    <div
      style={{
        background: "var(--color-bg-secondary)",
        border: "1px solid var(--color-border-default)",
        borderRadius: "var(--radius-md)",
        padding: "var(--space-3)",
        display: "flex",
        alignItems: "center",
        gap: "var(--space-3)",
      }}
    >
      <span style={{ fontSize: "20px" }}>{icon}</span>
      <div>
        <div style={{ fontSize: "13px", color: "var(--color-fg-muted)" }}>{title}</div>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-1)" }}>
          <span
            style={{
              width: 8,
              height: 8,
              borderRadius: "50%",
              background: color,
              display: "inline-block",
            }}
          />
          <span style={{ fontSize: "14px", fontWeight: 500, textTransform: "capitalize" }}>
            {health}
          </span>
        </div>
      </div>
    </div>
  );
}

/** Metric value card. */
function MetricCard({
  label,
  value,
}: {
  label: string;
  value: number | string;
}): React.ReactElement {
  return (
    <div
      style={{
        background: "var(--color-bg-secondary)",
        border: "1px solid var(--color-border-default)",
        borderRadius: "var(--radius-md)",
        padding: "var(--space-3)",
      }}
    >
      <div style={{ fontSize: "12px", color: "var(--color-fg-muted)" }}>{label}</div>
      <div style={{ fontSize: "18px", fontWeight: 600, marginTop: "4px" }}>{value}</div>
    </div>
  );
}

/** Project card with summary. */
function ProjectCard({
  project,
  onClick,
}: {
  project: ProjectSummary;
  onClick: () => void;
}): React.ReactElement {
  return (
    <button
      onClick={onClick}
      style={{
        background: "var(--color-bg-secondary)",
        border: "1px solid var(--color-border-default)",
        borderRadius: "var(--radius-md)",
        padding: "var(--space-3)",
        cursor: "pointer",
        textAlign: "left",
        color: "var(--color-fg-default)",
        transition: "border-color 0.15s",
      }}
      onMouseEnter={(e) => (e.currentTarget.style.borderColor = "var(--color-accent)")}
      onMouseLeave={(e) => (e.currentTarget.style.borderColor = "var(--color-border-default)")}
      tabIndex={0}
    >
      <div style={{ fontSize: "14px", fontWeight: 600, marginBottom: "var(--space-2)" }}>
        {project.name}
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-1)", fontSize: "12px" }}>
        <span style={{ color: "var(--color-fg-muted)" }}>Milestones</span>
        <span>{project.milestone_count}</span>
        <span style={{ color: "var(--color-fg-muted)" }}>Issues</span>
        <span>{project.issue_count}</span>
        <span style={{ color: "var(--color-fg-muted)" }}>Running</span>
        <span style={{ color: "var(--color-accent)" }}>{project.running_count}</span>
        <span style={{ color: "var(--color-fg-muted)" }}>Completed</span>
        <span style={{ color: "var(--color-success)" }}>{project.completed_count}</span>
        <span style={{ color: "var(--color-fg-muted)" }}>Failed</span>
        <span style={{ color: "var(--color-danger)" }}>{project.failed_count}</span>
      </div>
    </button>
  );
}

/** List of active runs with state badges. */
function RunList({ navigate }: { navigate: (p: Page) => void }): React.ReactElement {
  // Fixture data showing various run states.
  const runs = [
    { id: "run-001", issue: "COE-402", status: "running" as const, phase: "code_generation", lastProgress: "Generating implementation plan...", startedAt: new Date(Date.now() - 300_000).toISOString() },
    { id: "run-002", issue: "COE-434", status: "retry_queued" as const, phase: "waiting", lastProgress: "Queued for retry (attempt 2)", startedAt: new Date(Date.now() - 600_000).toISOString() },
    { id: "run-003", issue: "COE-392", status: "running" as const, phase: "validation", lastProgress: "Running test suite...", startedAt: new Date(Date.now() - 120_000).toISOString() },
  ];

  return (
    <div
      style={{
        background: "var(--color-bg-secondary)",
        border: "1px solid var(--color-border-default)",
        borderRadius: "var(--radius-md)",
        overflow: "hidden",
      }}
    >
      {runs.map((run, idx) => (
        <button
          key={run.id}
          onClick={() => navigate({ kind: "run", runId: run.id })}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-3)",
            width: "100%",
            padding: "var(--space-3)",
            background: "transparent",
            border: "none",
            borderBottom: idx < runs.length - 1 ? "1px solid var(--color-border-default)" : "none",
            color: "var(--color-fg-default)",
            cursor: "pointer",
            textAlign: "left",
          }}
          onMouseEnter={(e) => (e.currentTarget.style.background = "var(--color-bg-tertiary)")}
          onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
          tabIndex={0}
        >
          {/* Status badge */}
          <RunStatusBadge status={run.status} />

          {/* Run info */}
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
              <span style={{ fontWeight: 500 }}>{run.issue}</span>
              <span style={{ fontSize: "11px", color: "var(--color-fg-subtle)" }}>
                {run.phase}
              </span>
            </div>
            <div style={{ fontSize: "12px", color: "var(--color-fg-muted)", marginTop: "2px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {run.lastProgress}
            </div>
          </div>

          {/* Time ago */}
          <span style={{ fontSize: "11px", color: "var(--color-fg-subtle)", whiteSpace: "nowrap" }}>
            {formatTimeAgo(run.startedAt)}
          </span>
        </button>
      ))}
    </div>
  );
}

/** Run status badge component. */
function RunStatusBadge({ status }: { status: string }): React.ReactElement {
  const { bg, fg } = RUN_STATUS_COLORS[status] ?? RUN_STATUS_COLORS.unclaimed;

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
        whiteSpace: "nowrap",
        minWidth: "70px",
        textAlign: "center",
      }}
    >
      {status.replace("_", " ")}
    </span>
  );
}

/** Recent event row. */
function EventRow({
  event,
  isLast,
}: {
  event: SnapshotEventSummary;
  isLast: boolean;
}): React.ReactElement {
  const eventIcon: Record<string, string> = {
    worker_started: "▶️",
    worker_completed: "✅",
    retry_scheduled: "🔄",
    client_attached: "🔗",
    client_detached: "🔌",
    warning: "⚠️",
    snapshot_published: "📤",
    stream_attached: "📡",
    workspace_prepared: "📦",
  };

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-3)",
        padding: "var(--space-2) var(--space-3)",
        borderBottom: isLast ? "none" : "1px solid var(--color-border-muted)",
      }}
    >
      <span style={{ fontSize: "14px" }}>{eventIcon[event.kind] ?? "📌"}</span>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: "13px" }}>
          {event.issue_identifier && (
            <span style={{ fontWeight: 500, marginRight: "var(--space-1)" }}>
              {event.issue_identifier}
            </span>
          )}
          <span style={{ color: "var(--color-fg-muted)" }}>{event.summary}</span>
        </div>
        <div style={{ fontSize: "11px", color: "var(--color-fg-subtle)", marginTop: "2px" }}>
          {event.kind}
        </div>
      </div>
      <span style={{ fontSize: "11px", color: "var(--color-fg-subtle)", whiteSpace: "nowrap" }}>
        {formatTimeAgo(event.happened_at)}
      </span>
    </div>
  );
}
