import type { EntityKind } from "./envelope.js";
import type { SchemaVersion } from "./version.js";

export type ActionKind =
  | "retry"
  | "cancel"
  | "pause"
  | "resume"
  | "rehydrate"
  | "comment"
  | "transition_issue"
  | "create_followup"
  | "approval_decision"
  | "publish_plan"
  | "task_graph_milestone"
  | "task_graph_issue"
  | "task_graph_sub_issue"
  | "task_graph_relation"
  | "task_graph_evidence";

export interface ActionTarget {
  entity_kind: EntityKind;
  entity_id: string;
}

/** Action dispatch payload for POST /api/v1/actions/dispatch. */
export interface ActionDispatch {
  schema_version: SchemaVersion;
  correlation_id: string;
  action_kind: ActionKind;
  target_entity: ActionTarget;
  payload?: unknown;
  idempotency_key?: string;
}

/** Outcome returned for a dispatched action. */
export type ActionStatus =
  | "accepted"
  | "rejected"
  | "queued"
  | "completed";

/** Classes of follow-up events the caller can expect after an action. */
export type ExpectedFollowup =
  | "action_completion"
  | "run_lifecycle"
  | "journal_entry"
  | "snapshot_update";

/** Permission check embedded in a receipt when a capability guard fires. */
export interface PermissionResult {
  allowed: boolean;
  required_capability: string;
  reason?: string;
}
