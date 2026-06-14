import type { SchemaVersion } from "./version.js";

export type PlanningArtifactKind =
  | "intake"
  | "requirements"
  | "milestone_draft"
  | "issue_draft"
  | "sub_issue_draft"
  | "dependency_map"
  | "acceptance_criteria"
  | "verification_plan"
  | "research_summary"
  | "codebase_analysis";

/** Planning session artifact. */
export interface PlanningArtifact {
  schema_version: SchemaVersion;
  artifact_id: string;
  session_id: string;
  kind: PlanningArtifactKind;
  title: string;
  content: string;
  created_at: string;
  updated_at: string;
  generated_by?: string;
  approved: boolean;
  published_to_tracker: boolean;
}

export type PlanningSessionStatus =
  | "draft"
  | "in_review"
  | "approved"
  | "published"
  | "archived";

/** Planning session summary for listing. */
export interface PlanningSessionSummary {
  schema_version: SchemaVersion;
  session_id: string;
  project_id: string;
  title: string;
  status: PlanningSessionStatus;
  artifact_count: number;
  created_at: string;
  updated_at: string;
}

// ─── Linear Draft Preview & Publish Flow ─────────────────────────────────────

export type LinearDraftEntityKind =
  | "milestone"
  | "issue"
  | "sub_issue"
  | "relation"
  | "comment";

export type LinearDraftOperation = "create" | "update";

/** A single validation message for a task or the overall plan. */
export interface PlanValidationMessage {
  task_id?: string;
  field: string;
  message: string;
}

/** Summary produced by the draft and publish endpoints. */
export interface PlanValidationSummary {
  ok: boolean;
  error_count: number;
  warning_count: number;
  errors: PlanValidationMessage[];
  warnings: PlanValidationMessage[];
}

/** One concrete Linear entity that the draft/publish run will create or update. */
export interface LinearDraftEntity {
  entity_id: string;
  kind: LinearDraftEntityKind;
  op: LinearDraftOperation;
  source_task_id?: string;
  source_file?: string;
  title: string;
  milestone?: string;
  parent_id?: string;
  blocked_by: string[];
  blocks: string[];
  warnings: PlanValidationMessage[];
  payload: unknown;
}

/** POST /api/v1/planning/draft request body. */
export interface LinearDraftRequest {
  schema_version: SchemaVersion;
  correlation_id: string;
  manifest_path: string;
  repo_root: string;
  project_id: string;
  team_id: string;
  linear_project: string;
  publish_receipt_path: string;
  existing_receipt_path?: string;
}

/** POST /api/v1/planning/draft response body. */
export interface LinearDraftPreview {
  schema_version: SchemaVersion;
  draft_id: string;
  correlation_id: string;
  planning_wave: string;
  linear_project: string;
  project_id: string;
  team_id: string;
  manifest_path: string;
  publish_receipt_path: string;
  validation: PlanValidationSummary;
  entities: LinearDraftEntity[];
  can_publish: boolean;
}

/** Failure for one entity during publish. */
export interface LinearPublishFailure {
  entity_id: string;
  kind: LinearDraftEntityKind;
  source_task_id?: string;
  error: string;
}

/** POST /api/v1/planning/publish request body. */
export interface LinearPublishRequest {
  schema_version: SchemaVersion;
  draft_id: string;
  correlation_id: string;
  approved: boolean;
}

/** Milestone entry inside a publish receipt. */
export interface PublishedMilestone {
  name: string;
  milestone_id: string;
}

/** Task entry inside a publish receipt. */
export interface PublishedTask {
  task_id: string;
  issue: string;
  issue_id: string;
  url: string;
  file: string;
}

/** Publish receipt returned by the publish endpoint. */
export interface LinearPublishReceipt {
  planning_wave: string;
  linear_project: string;
  published_at: string;
  milestones: PublishedMilestone[];
  tasks: PublishedTask[];
}

/** POST /api/v1/planning/publish response body. */
export interface LinearPublishResponse {
  schema_version: SchemaVersion;
  draft_id: string;
  correlation_id: string;
  status: string;
  failures: LinearPublishFailure[];
  receipt: LinearPublishReceipt;
}
