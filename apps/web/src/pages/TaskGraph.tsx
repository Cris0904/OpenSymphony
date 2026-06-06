/**
 * Task Graph page.
 *
 * Renders project, milestone, issue, and sub-issue hierarchy
 * with runtime overlay badges showing run state, health, and
 * dependency information.
 */

import type {
  TaskGraphNode,
  TaskGraphSnapshot,
  TaskGraphNodeKind,
  TaskGraphStateCategory,
} from "@opensymphony/gateway-schema";

type Page =
  | { kind: "dashboard" }
  | { kind: "project"; projectId: string }
  | { kind: "task-graph"; projectId: string }
  | { kind: "run"; runId: string };

interface TaskGraphProps {
  projectId: string;
  navigate: (page: Page) => void;
}

// Fixture task graph data representing Linear hierarchy.
const fixtureTaskGraph: TaskGraphSnapshot = {
  schema_version: { major: 1, minor: 0, patch: 0 },
  project_id: "4b7bc834-ffad-4beb-bd63-4d79cd6c4f3a",
  generated_at: new Date().toISOString(),
  root_ids: ["milestone-m7"],
  nodes: [
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "milestone-m7",
      kind: "milestone",
      identifier: "M7",
      title: "M7: Shared Client And Desktop Alpha",
      state: "Completed",
      state_category: "done",
      parent_id: undefined,
      children: ["issue-402", "issue-411", "issue-414"],
      blocked_by: [],
      labels: ["milestone"],
      created_at: "2025-01-15T10:00:00Z",
      updated_at: "2025-06-05T21:00:00Z",
      priority: 1,
      estimate_minutes: undefined,
    },
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "issue-402",
      kind: "issue",
      identifier: "COE-402",
      title: "App Shell, Dashboard, Task Graph, And Run Views",
      state: "In Progress",
      state_category: "in_progress",
      parent_id: "milestone-m7",
      children: ["sub-issue-402-1", "sub-issue-402-2"],
      blocked_by: ["issue-392", "issue-394", "issue-397"],
      labels: ["frontend", "ui"],
      branch_name: "leonardogonzalez/coe-402-app-shell-dashboard-task-graph-and-run-views",
      created_at: "2025-06-01T10:00:00Z",
      updated_at: "2025-06-05T20:00:00Z",
      priority: 2,
      estimate_minutes: 480,
    },
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "sub-issue-402-1",
      kind: "sub_issue",
      identifier: "COE-402-1",
      title: "Layout Components (AppShell, Sidebar, StatusBar)",
      state: "In Progress",
      state_category: "in_progress",
      parent_id: "issue-402",
      children: [],
      blocked_by: [],
      labels: ["frontend"],
      branch_name: "leonardogonzalez/coe-402-app-shell-dashboard-task-graph-and-run-views",
      created_at: "2025-06-01T11:00:00Z",
      updated_at: "2025-06-05T19:00:00Z",
      priority: 1,
      estimate_minutes: 240,
    },
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "sub-issue-402-2",
      kind: "sub_issue",
      identifier: "COE-402-2",
      title: "Dashboard and Task Graph Pages",
      state: "Todo",
      state_category: "todo",
      parent_id: "issue-402",
      children: [],
      blocked_by: ["sub-issue-402-1"],
      labels: ["frontend"],
      branch_name: undefined,
      created_at: "2025-06-01T11:30:00Z",
      updated_at: "2025-06-01T11:30:00Z",
      priority: 2,
      estimate_minutes: 240,
    },
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "issue-411",
      kind: "issue",
      identifier: "COE-411",
      title: "Task Graph Editor and Runtime Overlay UI",
      state: "Todo",
      state_category: "todo",
      parent_id: "milestone-m7",
      children: [],
      blocked_by: ["issue-402"],
      labels: ["frontend", "editor"],
      branch_name: undefined,
      created_at: "2025-06-02T10:00:00Z",
      updated_at: "2025-06-02T10:00:00Z",
      priority: 2,
      estimate_minutes: 480,
    },
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "issue-414",
      kind: "issue",
      identifier: "COE-414",
      title: "Diff, Validation, Approval, and Run Action Views",
      state: "Todo",
      state_category: "todo",
      parent_id: "milestone-m7",
      children: [],
      blocked_by: ["issue-402"],
      labels: ["frontend", "diff"],
      branch_name: undefined,
      created_at: "2025-06-02T11:00:00Z",
      updated_at: "2025-06-02T11:00:00Z",
      priority: 3,
      estimate_minutes: 480,
    },
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "issue-392",
      kind: "issue",
      identifier: "COE-392",
      title: "Task Graph, Run Detail, File, And Diff Read APIs",
      state: "Done",
      state_category: "done",
      parent_id: "milestone-m7",
      children: [],
      blocked_by: [],
      labels: ["backend", "api"],
      branch_name: undefined,
      created_at: "2025-05-20T10:00:00Z",
      updated_at: "2025-06-01T15:00:00Z",
      priority: 1,
      estimate_minutes: 360,
    },
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "issue-394",
      kind: "issue",
      identifier: "COE-394",
      title: "Frontend Workspace and Shared Schemas",
      state: "Done",
      state_category: "done",
      parent_id: "milestone-m7",
      children: [],
      blocked_by: [],
      labels: ["frontend", "infrastructure"],
      branch_name: undefined,
      created_at: "2025-05-20T11:00:00Z",
      updated_at: "2025-06-02T14:00:00Z",
      priority: 1,
      estimate_minutes: 360,
    },
    {
      schema_version: { major: 1, minor: 0, patch: 0 },
      node_id: "issue-397",
      kind: "issue",
      identifier: "COE-397",
      title: "Gateway API Client, Transport Adapters, and Reducers",
      state: "Done",
      state_category: "done",
      parent_id: "milestone-m7",
      children: [],
      blocked_by: [],
      labels: ["frontend", "api"],
      branch_name: undefined,
      created_at: "2025-05-21T10:00:00Z",
      updated_at: "2025-06-03T16:00:00Z",
      priority: 1,
      estimate_minutes: 360,
    },
  ],
};

// Runtime overlay: map issue nodes to their current run state.
const runtimeOverlay: Record<string, { runId?: string; status: string; phase?: string }> = {
  "issue-402": { runId: "run-001", status: "running", phase: "code_generation" },
  "issue-411": { status: "unclaimed" },
  "issue-414": { status: "unclaimed" },
  "issue-392": { runId: "run-003", status: "running", phase: "validation" },
  "issue-394": { status: "released" },
  "issue-397": { status: "released" },
};

export function TaskGraph({ projectId, navigate }: TaskGraphProps): React.ReactElement {
  const graph = fixtureTaskGraph;
  const nodeMap = new Map(graph.nodes.map((n) => [n.node_id, n]));

  // Build hierarchical tree from flat node list.
  const rootNodes = graph.root_ids
    .map((id) => nodeMap.get(id))
    .filter((n): n is TaskGraphNode => n !== undefined);

  return (
    <div className="flex flex-col gap-4">
      {/* Header */}
      <header>
        <h1 style={{ margin: 0, fontSize: "20px", fontWeight: 600 }}>
          Task Graph
        </h1>
        <p style={{ margin: "4px 0 0", color: "var(--color-fg-muted)", fontSize: "13px" }}>
          Project hierarchy with runtime state overlays
        </p>
      </header>

      {/* Filters */}
      <FilterBar />

      {/* Graph tree */}
      <div
        style={{
          background: "var(--color-bg-secondary)",
          border: "1px solid var(--color-border-default)",
          borderRadius: "var(--radius-md)",
          padding: "var(--space-3)",
          overflow: "auto",
        }}
      >
        {rootNodes.map((node) => (
          <TreeNode
            key={node.node_id}
            node={node}
            nodeMap={nodeMap}
            depth={0}
            navigate={navigate}
            runtimeOverlay={runtimeOverlay}
          />
        ))}
      </div>

      {/* Legend */}
      <Legend />
    </div>
  );
}

/** Recursive tree node renderer. */
function TreeNode({
  node,
  nodeMap,
  depth,
  navigate,
  runtimeOverlay,
}: {
  node: TaskGraphNode;
  nodeMap: Map<string, TaskGraphNode>;
  depth: number;
  navigate: (page: Page) => void;
  runtimeOverlay: Record<string, { runId?: string; status: string; phase?: string }>;
}): React.ReactElement {
  const children = node.children
    .map((id) => nodeMap.get(id))
    .filter((n): n is TaskGraphNode => n !== undefined);

  const overlay = runtimeOverlay[node.node_id];
  const stateCategoryColor = getStateCategoryColor(node.state_category);
  const kindIcon = getKindIcon(node.kind);

  return (
    <div>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
          paddingLeft: `${depth * 20}px`,
          padding: "var(--space-1) var(--space-2)",
          marginLeft: depth > 0 ? "0" : "0",
          borderLeft: depth > 0 ? "2px solid var(--color-border-muted)" : "none",
        }}
      >
        {/* Expand/collapse indicator (simplified: always show children) */}
        {children.length > 0 ? (
          <span style={{ fontSize: "10px", color: "var(--color-fg-subtle)", width: "12px" }}>▼</span>
        ) : (
          <span style={{ width: "12px" }} />
        )}

        {/* Kind icon */}
        <span style={{ fontSize: "14px" }}>{kindIcon}</span>

        {/* Node info */}
        <button
          onClick={() => {
            if (overlay?.runId) {
              navigate({ kind: "run", runId: overlay.runId });
            }
          }}
          style={{
            background: "none",
            border: "none",
            color: overlay?.runId ? "var(--color-accent)" : "var(--color-fg-default)",
            cursor: overlay?.runId ? "pointer" : "default",
            padding: 0,
            textAlign: "left",
            fontSize: "13px",
            fontWeight: 500,
          }}
          tabIndex={overlay?.runId ? 0 : -1}
        >
          <span>{node.identifier}</span>
          <span style={{ color: "var(--color-fg-muted)", marginLeft: "var(--space-1)" }}>
            {node.title}
          </span>
        </button>

        {/* State badge */}
        <StateBadge category={node.state_category} state={node.state} />

        {/* Runtime overlay badge */}
        {overlay && <RuntimeBadge status={overlay.status} phase={overlay.phase} />}

        {/* Blocked by indicator */}
        {node.blocked_by.length > 0 && (
          <span
            title={`Blocked by: ${node.blocked_by.join(", ")}`}
            style={{
              fontSize: "11px",
              color: "var(--color-fg-subtle)",
              marginLeft: "auto",
            }}
          >
            🔒 {node.blocked_by.length}
          </span>
        )}

        {/* Priority */}
        {node.priority !== undefined && (
          <span
            style={{
              fontSize: "10px",
              color: "var(--color-fg-subtle)",
              padding: "1px 4px",
              background: "var(--color-bg-tertiary)",
              borderRadius: "3px",
            }}
          >
            P{node.priority}
          </span>
        )}

        {/* Estimate */}
        {node.estimate_minutes !== undefined && (
          <span
            style={{
              fontSize: "10px",
              color: "var(--color-fg-subtle)",
              padding: "1px 4px",
              background: "var(--color-bg-tertiary)",
              borderRadius: "3px",
            }}
          >
            {node.estimate_minutes}m
          </span>
        )}
      </div>

      {/* Children */}
      {children.map((child) => (
        <TreeNode
          key={child.node_id}
          node={child}
          nodeMap={nodeMap}
          depth={depth + 1}
          navigate={navigate}
          runtimeOverlay={runtimeOverlay}
        />
      ))}
    </div>
  );
}

/** State category badge. */
function StateBadge({
  category,
  state,
}: {
  category: TaskGraphStateCategory;
  state: string;
}): React.ReactElement {
  const colors: Record<TaskGraphStateCategory, { bg: string; fg: string }> = {
    backlog: { bg: "rgba(110, 118, 129, 0.15)", fg: "var(--color-fg-subtle)" },
    todo: { bg: "rgba(139, 148, 158, 0.15)", fg: "var(--color-fg-muted)" },
    in_progress: { bg: "rgba(88, 166, 255, 0.15)", fg: "var(--color-accent)" },
    done: { bg: "rgba(63, 185, 80, 0.15)", fg: "var(--color-success)" },
    canceled: { bg: "rgba(248, 81, 73, 0.15)", fg: "var(--color-danger)" },
  };
  const { bg, fg } = colors[category];

  return (
    <span
      style={{
        fontSize: "10px",
        fontWeight: 500,
        padding: "1px 6px",
        borderRadius: "3px",
        background: bg,
        color: fg,
        textTransform: "capitalize",
        whiteSpace: "nowrap",
      }}
    >
      {state}
    </span>
  );
}

/** Runtime status badge overlay. */
function RuntimeBadge({
  status,
  phase,
}: {
  status: string;
  phase?: string;
}): React.ReactElement {
  const colors: Record<string, { bg: string; fg: string }> = {
    running: { bg: "rgba(63, 185, 80, 0.15)", fg: "var(--color-success)" },
    retry_queued: { bg: "rgba(210, 153, 34, 0.15)", fg: "var(--color-attention)" },
    released: { bg: "rgba(139, 148, 158, 0.15)", fg: "var(--color-fg-muted)" },
    claimed: { bg: "rgba(88, 166, 255, 0.15)", fg: "var(--color-accent)" },
    unclaimed: { bg: "rgba(110, 118, 129, 0.15)", fg: "var(--color-fg-subtle)" },
  };
  const { bg, fg } = colors[status] ?? colors.unclaimed;

  return (
    <span
      title={phase ? `Status: ${status}, Phase: ${phase}` : `Status: ${status}`}
      style={{
        fontSize: "10px",
        fontWeight: 500,
        padding: "1px 6px",
        borderRadius: "10px",
        background: bg,
        color: fg,
        textTransform: "capitalize",
        whiteSpace: "nowrap",
        display: "flex",
        alignItems: "center",
        gap: "4px",
      }}
    >
      {status === "running" && (
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
      {status.replace("_", " ")}
      {phase && (
        <span style={{ opacity: 0.7, fontSize: "9px" }}>{phase}</span>
      )}
    </span>
  );
}

/** Filter bar for task graph. */
function FilterBar(): React.ReactElement {
  return (
    <div style={{ display: "flex", gap: "var(--space-2)", flexWrap: "wrap" }}>
      {["All", "In Progress", "Todo", "Done", "Blocked"].map((filter) => (
        <button
          key={filter}
          style={{
            padding: "var(--space-1) var(--space-3)",
            background: filter === "All" ? "var(--color-accent)" : "var(--color-bg-tertiary)",
            border: "none",
            borderRadius: "var(--radius-md)",
            color: filter === "All" ? "#fff" : "var(--color-fg-default)",
            cursor: "pointer",
            fontSize: "12px",
            fontWeight: filter === "All" ? 500 : 400,
          }}
          tabIndex={0}
        >
          {filter}
        </button>
      ))}
    </div>
  );
}

/** Legend for node types and statuses. */
function Legend(): React.ReactElement {
  return (
    <div
      style={{
        display: "flex",
        gap: "var(--space-3)",
        fontSize: "11px",
        color: "var(--color-fg-muted)",
        flexWrap: "wrap",
      }}
    >
      <span>
        <span style={{ marginRight: "4px" }}>🏁</span> Milestone
      </span>
      <span>
        <span style={{ marginRight: "4px" }}>📋</span> Issue
      </span>
      <span>
        <span style={{ marginRight: "4px" }}>☐</span> Sub-issue
      </span>
      <span style={{ marginLeft: "var(--space-3)" }}>
        <span
          style={{
            display: "inline-block",
            width: 8,
            height: 8,
            borderRadius: "50%",
            background: "var(--color-success)",
            marginRight: "4px",
            verticalAlign: "middle",
          }}
        />
        Running
      </span>
      <span>
        <span
          style={{
            display: "inline-block",
            width: 8,
            height: 8,
            borderRadius: "50%",
            background: "var(--color-attention)",
            marginRight: "4px",
            verticalAlign: "middle",
          }}
        />
        Retry Queued
      </span>
      <span>
        <span
          style={{
            display: "inline-block",
            width: 8,
            height: 8,
            borderRadius: "50%",
            background: "var(--color-fg-subtle)",
            marginRight: "4px",
            verticalAlign: "middle",
          }}
        />
        Unclaimed
      </span>
    </div>
  );
}

function getKindIcon(kind: TaskGraphNodeKind): string {
  switch (kind) {
    case "milestone":
      return "🏁";
    case "issue":
      return "📋";
    case "sub_issue":
      return "☐";
  }
}

function getStateCategoryColor(category: TaskGraphStateCategory): string {
  switch (category) {
    case "done":
      return "var(--color-success)";
    case "in_progress":
      return "var(--color-accent)";
    case "todo":
      return "var(--color-fg-muted)";
    case "backlog":
      return "var(--color-fg-subtle)";
    case "canceled":
      return "var(--color-danger)";
  }
}

// Pulse animation for running status.
const style = document.createElement("style");
style.textContent = `
@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}
`;
if (!document.querySelector('style[data-pulse]')) {
  style.setAttribute("data-pulse", "true");
  document.head.appendChild(style);
}
