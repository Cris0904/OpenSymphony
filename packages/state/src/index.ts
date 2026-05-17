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
  run: { runs: new Map(), loading: false, error: null },
  terminal: { frames: new Map(), cursor: new Map(), loading: false, error: null },
  approval: { pending: [], resolved: new Map(), error: null },
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
  | { type: "LOADING"; loading: boolean };

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

    case "RUN_UPDATED":
      return {
        ...state,
        run: {
          ...state.run,
          runs: new Map(state.run.runs).set(action.payload.run_id, action.payload),
          loading: false,
          error: null,
        },
      };

    case "TERMINAL_FRAMES_RECEIVED": {
      const frames = new Map(state.terminal.frames);
      const existing = frames.get(action.runId) ?? [];
      frames.set(action.runId, [...existing, ...action.frames]);
      const cursor = new Map(state.terminal.cursor);
      if (action.frames.length > 0) {
        const lastSeq = action.frames[action.frames.length - 1].frame_sequence;
        cursor.set(action.runId, lastSeq);
      }
      return {
        ...state,
        terminal: { ...state.terminal, frames, cursor, error: state.terminal.error },
      };
    }

    case "APPROVAL_RECEIVED":
      return {
        ...state,
        approval: {
          ...state.approval,
          pending: state.approval.pending.some(
            (a) => a.approval_id === action.payload.approval_id,
          )
            ? state.approval.pending
            : [...state.approval.pending, action.payload],
        },
      };

    case "APPROVAL_RESOLVED": {
      const resolved = new Map(state.approval.resolved);
      resolved.set(action.approvalId, action.payload);
      return {
        ...state,
        approval: {
          ...state.approval,
          pending: state.approval.pending.filter((a) => a.approval_id !== action.approvalId),
          resolved,
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
        dashboard: { ...state.dashboard, error: action.error },
        taskGraph: { ...state.taskGraph, error: action.error },
        run: { ...state.run, error: action.error },
        terminal: { ...state.terminal, error: action.error },
        approval: { ...state.approval, error: action.error },
        planning: { ...state.planning, error: action.error },
      };

    case "LOADING":
      return {
        ...state,
        dashboard: { ...state.dashboard, loading: action.loading },
        taskGraph: { ...state.taskGraph, loading: action.loading },
        run: { ...state.run, loading: action.loading },
        terminal: { ...state.terminal, loading: action.loading },
        planning: { ...state.planning, loading: action.loading },
      };

    default:
      return state;
  }
}