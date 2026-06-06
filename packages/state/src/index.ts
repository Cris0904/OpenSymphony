/**
 * Reducer-driven state management for OpenSymphony clients.
 *
 * This package defines Redux-style reducer functions that keep the
 * client-side state model in sync with gateway snapshots and event
 * streams. The reducer is transport-agnostic and framework-neutral.
 */

import type {
  GatewayEnvelope,
  DashboardSnapshot,
  TaskGraphNode,
  TaskGraphSnapshot,
  RunDetail,
  RunPhase,
  RunStreamLiveness,
  TerminalFrame,
  ApprovalRequest,
  PlanningSessionSummary,
} from "@opensymphony/gateway-schema";

// -- State slices --

export interface DashboardSlice {
  snapshot: DashboardSnapshot | null;
  loading: boolean;
  error: string | null;
}

export interface TaskGraphSlice {
  nodes: Map<string, TaskGraphNode>;
  rootIds: string[];
  loading: boolean;
  error: string | null;
}

export interface RunSlice {
  runs: Map<string, RunDetail>;
  loading: boolean;
  error: string | null;
  /** Per-run liveness tracking state. */
  liveness: Map<string, RunLivenessState>;
}

/** Liveness tracking for a single run. */
export interface RunLivenessState {
  phase: RunPhase;
  stream: RunStreamLiveness;
  lastProgressAt?: string | null;
  stallDeadlineAt?: string | null;
  reconnectAttempts: number;
}

export interface TerminalSlice {
  frames: Map<string, TerminalFrame[]>;
  cursor: Map<string, number>;
  loading: boolean;
  error: string | null;
}

export interface ApprovalSlice {
  pending: ApprovalRequest[];
  resolved: Map<string, ApprovalRequest>;
  loading: boolean;
  error: string | null;
}

export interface PlanningSlice {
  sessions: Map<string, PlanningSessionSummary>;
  loading: boolean;
  error: string | null;
}

// -- Combined state --

export interface GatewayState {
  dashboard: DashboardSlice;
  taskGraph: TaskGraphSlice;
  run: RunSlice;
  terminal: TerminalSlice;
  approval: ApprovalSlice;
  planning: PlanningSlice;
}

export const initialState: GatewayState = {
  dashboard: { snapshot: null, loading: false, error: null },
  taskGraph: { nodes: new Map(), rootIds: [], loading: false, error: null },
  run: { runs: new Map(), loading: false, error: null, liveness: new Map() },
  terminal: { frames: new Map(), cursor: new Map(), loading: false, error: null },
  approval: { pending: [], resolved: new Map(), loading: false, error: null },
  planning: { sessions: new Map(), loading: false, error: null },
};

// -- Action types --

export type GatewayAction =
  | { type: "SNAPSHOT_RECEIVED"; payload: DashboardSnapshot }
  | { type: "TASK_GRAPH_RECEIVED"; payload: TaskGraphSnapshot }
  | { type: "RUN_UPDATED"; payload: RunDetail }
  | { type: "TERMINAL_FRAMES_RECEIVED"; runId: string; frames: TerminalFrame[] }
  | { type: "APPROVAL_RECEIVED"; payload: ApprovalRequest }
  | { type: "APPROVAL_RESOLVED"; approvalId: string; payload: ApprovalRequest }
  | { type: "PLANNING_SESSION_UPDATED"; payload: PlanningSessionSummary }
  | { type: "ENVELOPE_RECEIVED"; payload: GatewayEnvelope }
  | { type: "ERROR"; error: string }
  | { type: "LOADING"; loading: boolean }
  | { type: "LIVENESS_UPDATE"; runId: string; phase: RunPhase; stream: RunStreamLiveness; progressAt?: string | null }
  | { type: "LIVENESS_STALL"; runId: string; deadlineAt: string }
  | { type: "LIVENESS_RECONNECT"; runId: string };

// -- Reducer --

export function gatewayReducer(
  state: GatewayState,
  action: GatewayAction,
): GatewayState {
  switch (action.type) {
    case "SNAPSHOT_RECEIVED":
      return {
        ...state,
        dashboard: { snapshot: action.payload, loading: false, error: null },
      };

    case "TASK_GRAPH_RECEIVED": {
      const nodes = new Map(action.payload.nodes.map((n) => [n.node_id, n]));
      return {
        ...state,
        taskGraph: { nodes, rootIds: action.payload.root_ids, loading: false, error: null },
      };
    }

    case "RUN_UPDATED": {
      const updatedRuns = new Map(state.run.runs).set(action.payload.run_id, action.payload);
      // Mirror liveness from the full RunDetail into the dedicated liveness map
      // to keep a single, consistent read path for UI consumers.
      const updatedLiveness = new Map(state.run.liveness);
      if (action.payload.liveness) {
        updatedLiveness.set(action.payload.run_id, {
          phase: action.payload.liveness.phase,
          stream: action.payload.liveness.stream,
          lastProgressAt: action.payload.liveness.latest_progress?.happened_at ?? null,
          stallDeadlineAt: null,
          reconnectAttempts: 0,
        });
      }
      return {
        ...state,
        run: {
          runs: updatedRuns,
          liveness: updatedLiveness,
          loading: false,
          error: null,
        },
      };
    }

    case "TERMINAL_FRAMES_RECEIVED": {
      const frames = new Map(state.terminal.frames);
      const existing = frames.get(action.runId) ?? [];
      // Deduplicate by frame_sequence to handle replayed/overlapping batches.
      const existingSeqs = new Set(existing.map((f) => f.frame_sequence));
      const newFrames = action.frames.filter((f) => !existingSeqs.has(f.frame_sequence));
      frames.set(action.runId, [...existing, ...newFrames]);
      const cursor = new Map(state.terminal.cursor);
      if (newFrames.length > 0) {
        // Use max over ALL new frames to handle unsorted batches.
        const maxSeq = Math.max(...newFrames.map((f) => f.frame_sequence));
        const prevCursor = cursor.get(action.runId) ?? 0;
        cursor.set(action.runId, Math.max(prevCursor, maxSeq));
      }
      return {
        ...state,
        terminal: { ...state.terminal, frames, cursor, loading: false, error: null },
      };
    }

    case "APPROVAL_RECEIVED": {
      return {
        ...state,
        approval: {
          ...state.approval,
          pending: state.approval.pending.some(
            (a) => a.approval_id === action.payload.approval_id,
          )
            ? state.approval.pending
            : [...state.approval.pending, action.payload],
          loading: false,
          error: null,
        },
      };
    }

    case "APPROVAL_RESOLVED": {
      const approvalId = action.payload.approval_id;
      const resolved = new Map(state.approval.resolved);
      resolved.set(approvalId, action.payload);
      return {
        ...state,
        approval: {
          ...state.approval,
          pending: state.approval.pending.filter((a) => a.approval_id !== approvalId),
          resolved,
          loading: false,
          error: null,
        },
      };
    }

    case "PLANNING_SESSION_UPDATED": {
      const sessions = new Map(state.planning.sessions);
      sessions.set(action.payload.session_id, action.payload);
      return {
        ...state,
        planning: { sessions, loading: false, error: null },
      };
    }

    case "ENVELOPE_RECEIVED":
      // Forward to appropriate slice reducer based on event_kind.
      // Placeholder: no-op for now.
      return state;

    case "ERROR":
      return {
        ...state,
        dashboard: { ...state.dashboard, error: action.error, loading: false },
        taskGraph: { ...state.taskGraph, error: action.error, loading: false },
        run: { ...state.run, error: action.error, loading: false },
        terminal: { ...state.terminal, error: action.error, loading: false },
        approval: { ...state.approval, error: action.error, loading: false },
        planning: { ...state.planning, error: action.error, loading: false },
      };

    case "LOADING": {
      const { dashboard, taskGraph, run, terminal, approval, planning } = state;
      return {
        ...state,
        dashboard: { ...dashboard, loading: action.loading, error: action.loading ? null : dashboard.error },
        taskGraph: { ...taskGraph, loading: action.loading, error: action.loading ? null : taskGraph.error },
        run: { ...run, loading: action.loading, error: action.loading ? null : run.error },
        terminal: { ...terminal, loading: action.loading, error: action.loading ? null : terminal.error },
        approval: { ...approval, loading: action.loading, error: action.loading ? null : approval.error },
        planning: { ...planning, loading: action.loading, error: action.loading ? null : planning.error },
      };
    }

    case "LIVENESS_UPDATE": {
      const liveness = new Map(state.run.liveness);
      liveness.set(action.runId, {
        phase: action.phase,
        stream: action.stream,
        lastProgressAt: action.progressAt ?? liveness.get(action.runId)?.lastProgressAt ?? null,
        stallDeadlineAt: liveness.get(action.runId)?.stallDeadlineAt ?? null,
        reconnectAttempts: liveness.get(action.runId)?.reconnectAttempts ?? 0,
      });
      return {
        ...state,
        run: { ...state.run, liveness, loading: false, error: null },
      };
    }

    case "LIVENESS_STALL": {
      const stallLiveness = new Map(state.run.liveness);
      const existing = stallLiveness.get(action.runId);
      stallLiveness.set(action.runId, {
        phase: "stalled",
        stream: "stale",
        lastProgressAt: existing?.lastProgressAt ?? null,
        stallDeadlineAt: action.deadlineAt,
        reconnectAttempts: existing?.reconnectAttempts ?? 0,
      });
      return {
        ...state,
        run: { ...state.run, liveness: stallLiveness, loading: false, error: null },
      };
    }

    case "LIVENESS_RECONNECT": {
      const reconnectLiveness = new Map(state.run.liveness);
      const reconnectExisting = reconnectLiveness.get(action.runId);
      reconnectLiveness.set(action.runId, {
        phase: reconnectExisting?.phase ?? "active",
        stream: "stale",
        lastProgressAt: reconnectExisting?.lastProgressAt ?? null,
        stallDeadlineAt: reconnectExisting?.stallDeadlineAt ?? null,
        reconnectAttempts: (reconnectExisting?.reconnectAttempts ?? 0) + 1,
      });
      return {
        ...state,
        run: { ...state.run, liveness: reconnectLiveness, loading: false, error: null },
      };
    }

    default:
      return state;
  }
}

/** Compute safe action set for a run based on its phase and stream health.
 *
 * Matrix:
 * | phase         | stream  | retry | cancel | rehydrate | detach |
 * |---------------|---------|-------|--------|-----------|--------|
 * | active        | healthy | false | true   | false     | false  |
 * | active        | stale   | false | true   | true      | false  |
 * | active        | dead    | false | false  | false     | true   |
 * | quiet         | stale   | false | true   | true      | false  |
 * | degraded      | stale   | false | true   | true      | false  |
 * | stalled       | stale   | true  | true   | true      | false  |
 * | retry_queued  | any     | true  | false  | false     | false  |
 * | cancelled     | any     | true  | false  | false     | false  |
 * | detached      | dead    | true  | false  | true      | false  |
 */
export function computeSafeActions(
  phase: RunPhase,
  stream: RunStreamLiveness,
): { retry: boolean; cancel: boolean; rehydrate: boolean; detach: boolean } {
  const retry = ["stalled", "retry_queued", "cancelled", "detached"].includes(phase);
  const cancel = ["active", "quiet", "degraded", "stalled"].includes(phase) && stream !== "dead";
  const rehydrate = (
    (["active", "quiet", "degraded", "stalled"].includes(phase) && stream === "stale") ||
    (phase === "detached" && stream === "dead")
  );
  const detach = phase === "active" && stream === "dead";

  return { retry, cancel, rehydrate, detach };
}