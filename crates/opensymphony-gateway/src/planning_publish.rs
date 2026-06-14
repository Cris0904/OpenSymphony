//! Gateway-mediated planning draft preview and publish flow.
//!
//! Provides `POST /api/v1/planning/draft` and `POST /api/v1/planning/publish`.
//! The draft endpoint reads a `docs/tasks/task-package.yaml` manifest, validates
//! the manifest and each declared task file, and returns the exact Linear
//! mutation payloads that would be executed. The publish endpoint requires an
//! explicit `approved: true` flag and drives the [`LinearMutationClient`] trait
//! to create or update milestones, issues, sub-issues, blocker relations, and
//! evidence comments, then writes the `linear-publish.yaml` receipt.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use chrono::{Duration, Utc};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use super::task_graph_mutations::{
    IssueOp, LinearMutationClient, MilestoneOp, SubIssueOp, TaskGraphEvidenceRequest,
    TaskGraphIssueRequest, TaskGraphMilestoneRequest, TaskGraphRelationRequest,
    TaskGraphSubIssueRequest,
};
use crate::opensymphony_gateway_schema::planning::{
    LinearDraftEntity, LinearDraftEntityKind, LinearDraftOperation, LinearDraftPreview,
    LinearDraftRequest, LinearPublishFailure, LinearPublishReceipt, LinearPublishRequest,
    LinearPublishResponse, LinearPublishResult, PlanValidationMessage, PlanValidationSummary,
    PublishedMilestone, PublishedTask,
};
use crate::opensymphony_gateway_schema::version::SchemaVersion;
use crate::opensymphony_planning::compiler::{
    LinearPublishEntity as YamlPublishEntity, LinearPublishReceipt as YamlPublishReceipt,
    MilestoneReceipt as YamlMilestoneReceipt, TaskKind,
};
use crate::opensymphony_planning::generator::domain::TaskId;
use crate::opensymphony_planning::graph_validate::{
    ManifestValidator, TaskFrontmatter, TaskPackageManifestFile, load_manifest, parse_task_file,
};

fn milestone_linked_tasks(
    milestone: &str,
    parsed_tasks: &BTreeMap<TaskId, ParsedTask>,
) -> Vec<TaskId> {
    parsed_tasks
        .iter()
        .filter(|(_, task)| task.frontmatter.milestone.as_deref() == Some(milestone))
        .map(|(task_id, _)| task_id.clone())
        .collect()
}
/// Resolve `raw` relative to `repo_root` and ensure the normalized path stays
/// within the repository. Rejects absolute paths and paths that escape the
/// workspace via `..` segments.
fn resolve_path_within_repo(repo_root: &Path, raw: &str) -> Result<PathBuf, PublishError> {
    let raw_path = Path::new(raw);
    if raw_path.is_absolute() {
        return Err(PublishError::LoadManifest(format!(
            "path must be relative to repo_root: {raw}"
        )));
    }
    let joined = repo_root.join(raw_path);
    let normalized = normalize_path(&joined);
    let base_normalized = normalize_path(repo_root);
    if !normalized.starts_with(&base_normalized) {
        return Err(PublishError::LoadManifest(format!(
            "path escapes repo_root: {raw}"
        )));
    }
    Ok(joined)
}

/// Normalize a path by resolving `.` and `..` segments without touching the
/// filesystem. This is intentionally conservative: symlinks are not followed,
/// so containment is checked against the logical path.
fn normalize_path(path: &Path) -> PathBuf {
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    let mut rooted = false;
    for component in path.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::Prefix(_) => rooted = true,
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                stack.pop();
            }
            std::path::Component::Normal(seg) => stack.push(seg.to_owned()),
        }
    }
    if rooted && !stack.is_empty() {
        let mut out = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
        for seg in stack {
            out.push(seg);
        }
        out
    } else {
        stack.iter().collect()
    }
}

/// Write a file atomically by creating a sibling temp file and then renaming it
/// into place. This keeps the original file intact if the write is interrupted
/// so retries can read a valid receipt rather than a truncated one.
async fn atomic_write_file(path: &Path, contents: &str) -> Result<(), PublishError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("receipt.yaml");
    let temp_path = parent.join(format!(
        ".{file_name}.tmp-{}-{}",
        process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    tokio::fs::write(&temp_path, contents)
        .await
        .map_err(|e| PublishError::ReceiptWrite(e.to_string()))?;
    tokio::fs::rename(&temp_path, path)
        .await
        .map_err(|e| PublishError::ReceiptWrite(e.to_string()))?;
    Ok(())
}

/// Drafts older than this are eligible for eviction on new draft creation.
const DRAFT_TTL_SECONDS: i64 = 24 * 60 * 60;

/// Remove draft entries (and their per-draft locks) that are older than the
/// TTL. Drafts that are currently inside the publish critical section are
/// skipped because their per-draft lock is held.
fn evict_stale_drafts_and_locks(
    drafts: &mut BTreeMap<String, Draft>,
    locks: &mut BTreeMap<String, Arc<Mutex<()>>>,
    cutoff: chrono::DateTime<Utc>,
) {
    let mut keys_to_remove = Vec::new();
    for (key, draft) in drafts.iter() {
        if draft.created_at < cutoff {
            keys_to_remove.push(key.clone());
        }
    }
    for key in keys_to_remove {
        // Only evict if the per-draft lock is not currently held by a publish.
        if let Some(lock) = locks.get(&key)
            && lock.try_lock().is_err()
        {
            continue;
        }
        drafts.remove(&key);
        locks.remove(&key);
    }
}

/// Shared state for the `/api/v1/planning/*` endpoints.
#[derive(Clone)]
pub struct PlanningPublishState {
    drafts: Arc<RwLock<BTreeMap<String, Draft>>>,
    /// Per-draft mutex that serializes the publish critical section for a
    /// single draft id. The outer map is only held while acquiring the per-draft
    /// lock, so the critical section itself does not block unrelated drafts.
    draft_locks: Arc<Mutex<BTreeMap<String, Arc<Mutex<()>>>>>,
    pub(crate) linear_mutations: Option<Arc<dyn LinearMutationClient>>,
}

impl std::fmt::Debug for PlanningPublishState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanningPublishState")
            .field("drafts", &self.drafts)
            .field("draft_locks", &"<BTreeMap<String, Mutex<()>>>")
            .field("linear_mutations", &"<dyn LinearMutationClient>")
            .finish()
    }
}

impl Default for PlanningPublishState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanningPublishState {
    /// Create an empty planning state with no drafts and no mutation client.
    pub fn new() -> Self {
        Self {
            drafts: Arc::new(RwLock::new(BTreeMap::new())),
            draft_locks: Arc::new(Mutex::new(BTreeMap::new())),
            linear_mutations: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Draft {
    draft_id: String,
    request: LinearDraftRequest,
    manifest: TaskPackageManifestFile,
    entities: Vec<LinearDraftEntity>,
    existing_receipt: YamlPublishReceipt,
    can_publish: bool,
    parsed_tasks: BTreeMap<TaskId, ParsedTask>,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ParsedTask {
    file: String,
    frontmatter: TaskFrontmatter,
    body: String,
}

/// Build the axum router for `/api/v1/planning/*`.
pub fn planning_router() -> Router<PlanningPublishState> {
    Router::new()
        .route("/draft", post(planning_draft_handler))
        .route("/publish", post(planning_publish_handler))
}

#[derive(Debug, thiserror::Error)]
enum PublishError {
    #[error("failed to load manifest: {0}")]
    LoadManifest(String),
    #[error("draft not found: {0}")]
    DraftNotFound(String),
    #[error("publish not approved")]
    NotApproved,
    #[error("Linear mutation client is not configured")]
    MutationClientUnavailable,
    #[error("failed to write publish receipt: {0}")]
    ReceiptWrite(String),
    #[error("publish failed: {0}")]
    PublishFailed(String),
}

impl PublishError {
    fn status_code(&self) -> StatusCode {
        match self {
            PublishError::LoadManifest(_) => StatusCode::BAD_REQUEST,
            PublishError::DraftNotFound(_) => StatusCode::NOT_FOUND,
            PublishError::NotApproved => StatusCode::BAD_REQUEST,
            PublishError::MutationClientUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            PublishError::ReceiptWrite(_) => StatusCode::INTERNAL_SERVER_ERROR,
            PublishError::PublishFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for PublishError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = serde_json::json!({"error": self.to_string()});
        (status, Json(body)).into_response()
    }
}

async fn planning_draft_handler(
    State(state): State<PlanningPublishState>,
    Json(request): Json<LinearDraftRequest>,
) -> Response {
    match generate_draft(request.clone()).await {
        Ok((manifest, parsed_tasks, entities, existing_receipt, validation, can_publish)) => {
            let draft_id = Uuid::new_v4().to_string();
            let preview = LinearDraftPreview {
                schema_version: SchemaVersion::default(),
                draft_id: draft_id.clone(),
                correlation_id: request.correlation_id.clone(),
                planning_wave: manifest.planning_wave.clone(),
                linear_project: request.linear_project.clone(),
                project_id: request.project_id.clone(),
                team_id: request.team_id.clone(),
                manifest_path: request.manifest_path.clone(),
                publish_receipt_path: request.publish_receipt_path.clone(),
                validation,
                entities: entities.clone(),
                can_publish,
            };
            let draft = Draft {
                draft_id: draft_id.clone(),
                request,
                manifest,
                entities,
                existing_receipt,
                can_publish,
                parsed_tasks,
                created_at: Utc::now(),
            };
            // Insert the new draft and evict stale drafts/locks in one critical
            // section to avoid unbounded growth in long-running processes.
            let now = Utc::now();
            let cutoff = now - Duration::seconds(DRAFT_TTL_SECONDS);
            {
                let mut locks = state.draft_locks.lock().await;
                let mut drafts = state.drafts.write().await;
                drafts.insert(draft_id, draft);
                evict_stale_drafts_and_locks(&mut drafts, &mut locks, cutoff);
            }
            (StatusCode::OK, Json(preview)).into_response()
        }
        Err(err) => err.into_response(),
    }
}

async fn planning_publish_handler(
    State(state): State<PlanningPublishState>,
    Json(request): Json<LinearPublishRequest>,
) -> Response {
    if !request.approved {
        return PublishError::NotApproved.into_response();
    }

    let client = match state.linear_mutations.clone() {
        Some(c) => c,
        None => return PublishError::MutationClientUnavailable.into_response(),
    };

    let draft_id = request.draft_id.clone();

    // Serialize publish operations for a single draft id. The outer map is only
    // held while acquiring the per-draft lock, so unrelated drafts can still
    // publish concurrently.
    let lock = {
        let mut locks = state.draft_locks.lock().await;
        locks
            .entry(draft_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };

    let response = {
        let _guard = lock.lock().await;

        let draft = match state.drafts.read().await.get(&draft_id).cloned() {
            Some(d) => d,
            None => return PublishError::DraftNotFound(draft_id).into_response(),
        };

        // Regenerate the draft immediately before publish so that any partial
        // receipt written by a previous attempt is picked up and already-created
        // entities are emitted as updates instead of duplicated creates.
        let original_draft_id = draft.draft_id.clone();
        let draft = match generate_draft(draft.request.clone()).await {
            Ok((manifest, parsed_tasks, entities, existing_receipt, _validation, can_publish)) => {
                Draft {
                    draft_id: original_draft_id,
                    request: draft.request,
                    manifest,
                    entities,
                    existing_receipt,
                    can_publish,
                    parsed_tasks,
                    created_at: draft.created_at,
                }
            }
            Err(err) => return err.into_response(),
        };

        if !draft.can_publish {
            let response = rejected_response(&draft, &request, "draft validation failed");
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(response)).into_response();
        }

        let response = match execute_publish(draft, client, &request.correlation_id).await {
            Ok(response) => (StatusCode::OK, Json(response)).into_response(),
            Err(err) => err.into_response(),
        };

        // After the receipt is safely written, the draft is consumed and can be
        // removed to avoid unbounded growth in long-running gateway processes.
        state.drafts.write().await.remove(&draft_id);
        response
    };

    state.draft_locks.lock().await.remove(&draft_id);
    response
}

async fn generate_draft(
    request: LinearDraftRequest,
) -> Result<
    (
        TaskPackageManifestFile,
        BTreeMap<TaskId, ParsedTask>,
        Vec<LinearDraftEntity>,
        YamlPublishReceipt,
        PlanValidationSummary,
        bool,
    ),
    PublishError,
> {
    let repo_root = PathBuf::from(&request.repo_root);
    let manifest_path = resolve_path_within_repo(&repo_root, &request.manifest_path)?;
    let manifest =
        load_manifest(&manifest_path).map_err(|e| PublishError::LoadManifest(e.to_string()))?;

    let validation_result = ManifestValidator::validate_against_repo_root(&manifest, &repo_root);
    let existing_receipt = read_existing_receipt(&request, &repo_root).await?;

    let mut parsed_tasks = BTreeMap::new();
    let mut path_errors: Vec<PlanValidationMessage> = Vec::new();
    for entry in &manifest.tasks {
        let path = match resolve_path_within_repo(&repo_root, &entry.file) {
            Ok(p) => p,
            Err(e) => {
                path_errors.push(PlanValidationMessage {
                    task_id: Some(entry.id.clone()),
                    field: "file".into(),
                    message: e.to_string(),
                });
                continue;
            }
        };
        match parse_task_file(&path) {
            Ok(parsed) => {
                parsed_tasks.insert(
                    TaskId::new(entry.id.clone()),
                    ParsedTask {
                        file: entry.file.clone(),
                        frontmatter: parsed.frontmatter,
                        body: parsed.body,
                    },
                );
            }
            Err(_) => {
                // Manifest validation already reports the parse error; skip
                // the entity so we don't build broken payloads.
            }
        }
    }

    let (mut errors, mut warnings) = manifest_validation_messages(&validation_result);
    errors.extend(path_errors);
    let (entities, build_errors, build_warnings) =
        build_entities(&request, &manifest, &parsed_tasks, &existing_receipt);
    errors.extend(build_errors);
    warnings.extend(build_warnings);

    let can_publish = errors.is_empty();
    let summary = PlanValidationSummary {
        ok: errors.is_empty(),
        error_count: errors.len(),
        warning_count: warnings.len(),
        errors,
        warnings,
    };

    Ok((
        manifest,
        parsed_tasks,
        entities,
        existing_receipt,
        summary,
        can_publish,
    ))
}

fn manifest_validation_messages(
    result: &crate::opensymphony_planning::graph_validate::ManifestValidationResult,
) -> (Vec<PlanValidationMessage>, Vec<PlanValidationMessage>) {
    let mut errors = Vec::new();
    let warnings = Vec::new();
    for m in &result.missing_task_files {
        errors.push(PlanValidationMessage {
            task_id: Some(m.task_id.to_string()),
            field: "file".into(),
            message: format!("missing task file {}", m.file_path),
        });
    }
    for m in &result.invalid_task_files {
        errors.push(PlanValidationMessage {
            task_id: Some(m.task_id.to_string()),
            field: "file".into(),
            message: format!("invalid task file {}: {}", m.file_path, m.reason),
        });
    }
    for m in &result.unknown_milestones {
        errors.push(PlanValidationMessage {
            task_id: Some(m.task_id.to_string()),
            field: "milestone".into(),
            message: format!("unknown milestone {}", m.declared_milestone),
        });
    }
    for m in &result.unknown_dependencies {
        errors.push(PlanValidationMessage {
            task_id: Some(m.from_task_id.to_string()),
            field: "blockedBy".into(),
            message: format!("unknown dependency {}", m.unknown_dependency),
        });
    }
    for cycle in &result.creation_order_cycles {
        errors.push(PlanValidationMessage {
            task_id: None,
            field: "dependencies".into(),
            message: format!(
                "dependency cycle: {}",
                cycle
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
        });
    }
    for m in &result.self_blocks {
        errors.push(PlanValidationMessage {
            task_id: Some(m.task_id.to_string()),
            field: "blockedBy".into(),
            message: "task blocks itself".into(),
        });
    }
    for id in &result.duplicate_task_ids {
        errors.push(PlanValidationMessage {
            task_id: Some(id.to_string()),
            field: "id".into(),
            message: "duplicate task id in manifest".into(),
        });
    }
    (errors, warnings)
}

async fn read_existing_receipt(
    request: &LinearDraftRequest,
    repo_root: &Path,
) -> Result<YamlPublishReceipt, PublishError> {
    let path = if let Some(existing) = request.existing_receipt_path.as_ref() {
        resolve_path_within_repo(repo_root, existing)?
    } else {
        resolve_path_within_repo(repo_root, &request.publish_receipt_path)?
    };

    if !path.exists() {
        return Ok(YamlPublishReceipt {
            planning_wave: String::new(),
            linear_project: None,
            published_at: None,
            milestones: BTreeMap::new(),
            tasks: BTreeMap::new(),
        });
    }

    let raw = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| PublishError::LoadManifest(format!("receipt {}: {}", path.display(), e)))?;
    serde_yaml::from_str(&raw)
        .map_err(|e| PublishError::LoadManifest(format!("receipt {}: {}", path.display(), e)))
}

fn build_entities(
    request: &LinearDraftRequest,
    manifest: &TaskPackageManifestFile,
    parsed_tasks: &BTreeMap<TaskId, ParsedTask>,
    existing_receipt: &YamlPublishReceipt,
) -> (
    Vec<LinearDraftEntity>,
    Vec<PlanValidationMessage>,
    Vec<PlanValidationMessage>,
) {
    let mut entities = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let correlation_id = &request.correlation_id;
    let project_id = &request.project_id;
    let team_id = &request.team_id;
    let planning_wave = &manifest.planning_wave;
    let milestone_set: BTreeSet<&str> = manifest.milestones.iter().map(String::as_str).collect();

    let existing_milestone_ids: BTreeMap<&str, &str> = existing_receipt
        .milestones
        .iter()
        .filter_map(|(name, ms)| ms.milestone_id.as_deref().map(|id| (name.as_str(), id)))
        .collect();

    // Milestones
    for (idx, name) in manifest.milestones.iter().enumerate() {
        let existing_id = existing_milestone_ids
            .get(name.as_str())
            .copied()
            .map(String::from);
        let op = if existing_id.is_some() {
            LinearDraftOperation::Update
        } else {
            LinearDraftOperation::Create
        };
        let payload = TaskGraphMilestoneRequest {
            schema_version: SchemaVersion::default().to_string(),
            correlation_id: correlation_id.clone(),
            op: match op {
                LinearDraftOperation::Create => MilestoneOp::Create,
                LinearDraftOperation::Update => MilestoneOp::Update,
            },
            idempotency_key: Some(format!("{correlation_id}-milestone-{name}")),
            project_id: project_id.clone(),
            milestone_id: existing_id,
            name: name.clone(),
            description: None,
            target_date: None,
            sort_order: Some(idx as f64),
        };
        entities.push(LinearDraftEntity {
            entity_id: name.clone(),
            kind: LinearDraftEntityKind::Milestone,
            op,
            source_task_id: None,
            source_file: None,
            title: name.clone(),
            milestone: None,
            parent_id: None,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
            warnings: Vec::new(),
            payload: serde_json::to_value(&payload).unwrap_or_default(),
        });
    }

    // Validate tasks and build issue/sub-issue entities.
    let mut ordered_task_ids: Vec<TaskId> = Vec::new();
    for (task_id, task) in parsed_tasks {
        let frontmatter = &task.frontmatter;
        if frontmatter.id.is_none() {
            errors.push(PlanValidationMessage {
                task_id: Some(task_id.to_string()),
                field: "id".into(),
                message: "missing id in task frontmatter".into(),
            });
        }
        let title = frontmatter.title.clone().unwrap_or_default();
        if title.trim().is_empty() {
            errors.push(PlanValidationMessage {
                task_id: Some(task_id.to_string()),
                field: "title".into(),
                message: "missing title in task frontmatter".into(),
            });
        }
        let milestone = frontmatter.milestone.clone().unwrap_or_default();
        if milestone.trim().is_empty() {
            errors.push(PlanValidationMessage {
                task_id: Some(task_id.to_string()),
                field: "milestone".into(),
                message: "missing milestone in task frontmatter".into(),
            });
        } else if !milestone_set.contains(milestone.as_str()) {
            errors.push(PlanValidationMessage {
                task_id: Some(task_id.to_string()),
                field: "milestone".into(),
                message: format!("milestone {milestone} is not declared in the manifest"),
            });
        }

        if frontmatter.priority.is_none() {
            warnings.push(PlanValidationMessage {
                task_id: Some(task_id.to_string()),
                field: "priority".into(),
                message: "missing priority; Linear will default to no priority".into(),
            });
        }
        if frontmatter.estimate.is_none() {
            warnings.push(PlanValidationMessage {
                task_id: Some(task_id.to_string()),
                field: "estimate".into(),
                message: "missing estimate".into(),
            });
        }

        if let Some(parent) = frontmatter.parent.as_ref() {
            let parent_id = TaskId::new(parent.clone());
            if parent == &task_id.0 {
                errors.push(PlanValidationMessage {
                    task_id: Some(task_id.to_string()),
                    field: "parent".into(),
                    message: "task cannot be its own parent".into(),
                });
            } else if !parsed_tasks.contains_key(&parent_id) {
                errors.push(PlanValidationMessage {
                    task_id: Some(task_id.to_string()),
                    field: "parent".into(),
                    message: format!("parent task {parent} not found"),
                });
            } else if parsed_tasks
                .get(&parent_id)
                .is_some_and(|p| p.frontmatter.parent.is_some())
            {
                errors.push(PlanValidationMessage {
                    task_id: Some(task_id.to_string()),
                    field: "parent".into(),
                    message: format!("parent task {parent} is itself a sub-issue"),
                });
            }
        }

        for dep in &frontmatter.blocked_by {
            if dep == &task_id.0 {
                errors.push(PlanValidationMessage {
                    task_id: Some(task_id.to_string()),
                    field: "blockedBy".into(),
                    message: "task cannot block itself".into(),
                });
            } else if !parsed_tasks.contains_key(&TaskId::new(dep.clone())) {
                errors.push(PlanValidationMessage {
                    task_id: Some(task_id.to_string()),
                    field: "blockedBy".into(),
                    message: format!("blockedBy dependency {dep} not found"),
                });
            }
        }

        for blocked in &frontmatter.blocks {
            if blocked == &task_id.0 {
                errors.push(PlanValidationMessage {
                    task_id: Some(task_id.to_string()),
                    field: "blocks".into(),
                    message: "task cannot block itself".into(),
                });
            } else if !parsed_tasks.contains_key(&TaskId::new(blocked.clone())) {
                errors.push(PlanValidationMessage {
                    task_id: Some(task_id.to_string()),
                    field: "blocks".into(),
                    message: format!("blocks target {blocked} not found"),
                });
            }
        }

        ordered_task_ids.push(task_id.clone());
    }

    // Topologically order tasks so parents and blockers are created first.
    ordered_task_ids = topo_sort_tasks(parsed_tasks, &ordered_task_ids);

    for task_id in &ordered_task_ids {
        let task = &parsed_tasks[task_id];
        let frontmatter = &task.frontmatter;
        let title = frontmatter.title.clone().unwrap_or_default();
        let milestone = frontmatter.milestone.clone().unwrap_or_default();
        let description = build_task_description(planning_wave, &task_id.0, &task.body);
        let priority = frontmatter.priority.map(|p| p as f64);
        let estimate = frontmatter.estimate.map(|e| e as f64);
        let existing_issue = existing_receipt
            .tasks
            .get(task_id)
            .and_then(|e| e.issue_id.clone());

        let entity = if let Some(parent) = frontmatter.parent.as_ref() {
            let parent_task_id = TaskId::new(parent.clone());
            let parent_milestone = parsed_tasks
                .get(&parent_task_id)
                .and_then(|p| p.frontmatter.milestone.clone())
                .unwrap_or_default();
            let existing_id = existing_issue.clone();
            let op = if existing_id.is_some() {
                LinearDraftOperation::Update
            } else {
                LinearDraftOperation::Create
            };
            let payload = TaskGraphSubIssueRequest {
                schema_version: SchemaVersion::default().to_string(),
                correlation_id: correlation_id.clone(),
                op: match op {
                    LinearDraftOperation::Create => SubIssueOp::Create,
                    LinearDraftOperation::Update => SubIssueOp::Update,
                },
                idempotency_key: Some(format!("{correlation_id}-sub-issue-{task_id}")),
                team_id: team_id.clone(),
                parent_id: parent.clone(),
                sub_issue_id: existing_id,
                parent_identifier: parent.clone(),
                title: title.clone(),
                description: Some(description.clone()),
                priority,
                estimate,
                assignee_id: None,
                project_id: Some(project_id.clone()),
                project_milestone_id: None,
                label_ids: None,
            };
            LinearDraftEntity {
                entity_id: task_id.to_string(),
                kind: LinearDraftEntityKind::SubIssue,
                op,
                source_task_id: Some(task_id.to_string()),
                source_file: Some(task.file.clone()),
                title,
                milestone: Some(parent_milestone),
                parent_id: Some(parent.clone()),
                blocked_by: frontmatter.blocked_by.clone(),
                blocks: frontmatter.blocks.clone(),
                warnings: Vec::new(),
                payload: serde_json::to_value(&payload).unwrap_or_default(),
            }
        } else {
            let existing_id = existing_issue.clone();
            let op = if existing_id.is_some() {
                LinearDraftOperation::Update
            } else {
                LinearDraftOperation::Create
            };
            let payload = TaskGraphIssueRequest {
                schema_version: SchemaVersion::default().to_string(),
                correlation_id: correlation_id.clone(),
                op: match op {
                    LinearDraftOperation::Create => IssueOp::Create,
                    LinearDraftOperation::Update => IssueOp::Update,
                },
                idempotency_key: Some(format!("{correlation_id}-issue-{task_id}")),
                team_id: team_id.clone(),
                issue_id: existing_id,
                title: title.clone(),
                description: Some(description.clone()),
                priority,
                estimate,
                assignee_id: None,
                project_id: Some(project_id.clone()),
                project_milestone_id: None,
                label_ids: None,
            };
            LinearDraftEntity {
                entity_id: task_id.to_string(),
                kind: LinearDraftEntityKind::Issue,
                op,
                source_task_id: Some(task_id.to_string()),
                source_file: Some(task.file.clone()),
                title,
                milestone: Some(milestone.clone()),
                parent_id: None,
                blocked_by: frontmatter.blocked_by.clone(),
                blocks: frontmatter.blocks.clone(),
                warnings: Vec::new(),
                payload: serde_json::to_value(&payload).unwrap_or_default(),
            }
        };
        entities.push(entity);
    }

    // Relations: for each blocked_by edge create a "blocks" relation from blocker to blocked.
    // If a previous receipt already records the relation, skip it to avoid duplicates on retry.
    for task_id in parsed_tasks.keys() {
        let frontmatter = &parsed_tasks[task_id].frontmatter;
        for blocker in &frontmatter.blocked_by {
            let blocker_id = TaskId::new(blocker.clone());
            if parsed_tasks.contains_key(&blocker_id) {
                let already_exists = existing_receipt
                    .tasks
                    .get(&blocker_id)
                    .and_then(|e| e.relation_ids.get(&task_id.0))
                    .is_some();
                if already_exists {
                    continue;
                }
                let payload = TaskGraphRelationRequest {
                    schema_version: SchemaVersion::default().to_string(),
                    correlation_id: correlation_id.clone(),
                    idempotency_key: Some(format!("{correlation_id}-relation-{blocker}-{task_id}")),
                    relation_type: "blocks".into(),
                    issue_id: blocker.clone(),
                    related_issue_id: task_id.to_string(),
                };
                entities.push(LinearDraftEntity {
                    entity_id: format!("{blocker}-blocks-{task_id}"),
                    kind: LinearDraftEntityKind::Relation,
                    op: LinearDraftOperation::Create,
                    source_task_id: Some(blocker.clone()),
                    source_file: None,
                    title: format!("{blocker} blocks {task_id}"),
                    milestone: None,
                    parent_id: None,
                    blocked_by: Vec::new(),
                    blocks: vec![task_id.to_string()],
                    warnings: Vec::new(),
                    payload: serde_json::to_value(&payload).unwrap_or_default(),
                });
            }
        }
    }

    // Evidence comments: one per task to record provenance. Skip tasks that
    // already have a persisted comment id from a previous publish.
    for task_id in parsed_tasks.keys() {
        let task = &parsed_tasks[task_id];
        let already_has_comment = existing_receipt
            .tasks
            .get(task_id)
            .is_some_and(|e| !e.comment_ids.is_empty());
        if already_has_comment {
            continue;
        }
        let body = format!(
            "Published from planning wave {planning_wave}.\nSource task: {task_id}\nFile: {file}",
            task_id = task_id.0,
            file = task.file,
        );
        let payload = TaskGraphEvidenceRequest {
            schema_version: SchemaVersion::default().to_string(),
            correlation_id: correlation_id.clone(),
            idempotency_key: Some(format!("{correlation_id}-comment-{task_id}")),
            issue_id: task_id.to_string(),
            body,
        };
        entities.push(LinearDraftEntity {
            entity_id: format!("{}-comment", task_id.0),
            kind: LinearDraftEntityKind::Comment,
            op: LinearDraftOperation::Create,
            source_task_id: Some(task_id.to_string()),
            source_file: Some(task.file.clone()),
            title: "Provenance comment".into(),
            milestone: task.frontmatter.milestone.clone(),
            parent_id: None,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
            warnings: Vec::new(),
            payload: serde_json::to_value(&payload).unwrap_or_default(),
        });
    }

    (entities, errors, warnings)
}

fn build_task_description(planning_wave: &str, task_id: &str, body: &str) -> String {
    let provenance = format!(
        "<!-- task-planning-wave: {planning_wave} -->\n<!-- task-source-id: {task_id} -->\n\n"
    );
    if body.trim().is_empty() {
        provenance.trim_end().to_string()
    } else {
        format!("{provenance}{body}")
    }
}

fn topo_sort_tasks(
    parsed_tasks: &BTreeMap<TaskId, ParsedTask>,
    task_ids: &[TaskId],
) -> Vec<TaskId> {
    let mut edges: BTreeMap<TaskId, BTreeSet<TaskId>> = BTreeMap::new();
    let mut in_degree: BTreeMap<TaskId, usize> = BTreeMap::new();
    for id in task_ids {
        in_degree.insert(id.clone(), 0);
        edges.insert(id.clone(), BTreeSet::new());
    }
    for id in task_ids {
        let task = &parsed_tasks[id];
        for dep in &task.frontmatter.blocked_by {
            let dep_id = TaskId::new(dep.clone());
            if task_ids.contains(&dep_id) && edges.get(&dep_id).is_none_or(|s| !s.contains(id)) {
                edges.entry(dep_id.clone()).or_default().insert(id.clone());
                *in_degree.entry(id.clone()).or_insert(0) += 1;
            }
        }
        if let Some(parent) = task.frontmatter.parent.as_ref() {
            let parent_id = TaskId::new(parent.clone());
            if task_ids.contains(&parent_id)
                && edges.get(&parent_id).is_none_or(|s| !s.contains(id))
            {
                edges.entry(parent_id).or_default().insert(id.clone());
                *in_degree.entry(id.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut ready: Vec<TaskId> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut result = Vec::with_capacity(task_ids.len());
    while let Some(next) = ready.pop() {
        result.push(next.clone());
        for child in edges.get(&next).cloned().unwrap_or_default() {
            let deg = in_degree.get_mut(&child).expect("in_degree entry");
            *deg -= 1;
            if *deg == 0 {
                ready.push(child);
            }
        }
    }
    // If a cycle remains, append the leftovers deterministically so the preview
    // still renders; publish will be blocked by validation.
    for id in task_ids {
        if !result.contains(id) {
            result.push(id.clone());
        }
    }
    result
}

async fn execute_publish(
    draft: Draft,
    client: Arc<dyn LinearMutationClient>,
    correlation_id: &str,
) -> Result<LinearPublishResponse, PublishError> {
    let mut milestone_name_to_id: BTreeMap<String, String> = BTreeMap::new();
    let mut task_id_to_issue_id: BTreeMap<String, String> = BTreeMap::new();
    let mut task_id_to_identifier: BTreeMap<String, String> = BTreeMap::new();
    let mut failures: Vec<LinearPublishFailure> = Vec::new();

    let mut running_receipt = draft.existing_receipt.clone();
    running_receipt.planning_wave = draft.manifest.planning_wave.clone();
    running_receipt.linear_project = Some(draft.request.linear_project.clone());

    for entity in &draft.entities {
        match entity.kind {
            LinearDraftEntityKind::Milestone => {
                let mut request: TaskGraphMilestoneRequest =
                    serde_json::from_value(entity.payload.clone()).map_err(|e| {
                        PublishError::PublishFailed(format!("milestone payload: {e}"))
                    })?;
                request.correlation_id = correlation_id.into();
                match client
                    .create_or_update_project_milestone(request, correlation_id)
                    .await
                {
                    Ok(response) => {
                        if let Some(id) = response.milestone_id.clone() {
                            milestone_name_to_id.insert(entity.entity_id.clone(), id.clone());
                            running_receipt.milestones.insert(
                                entity.entity_id.clone(),
                                YamlMilestoneReceipt {
                                    name: entity.entity_id.clone(),
                                    milestone_id: Some(id),
                                    linked_issues: milestone_linked_tasks(
                                        &entity.entity_id,
                                        &draft.parsed_tasks,
                                    ),
                                },
                            );
                        }
                    }
                    Err(e) => failures.push(LinearPublishFailure {
                        entity_id: entity.entity_id.clone(),
                        kind: LinearDraftEntityKind::Milestone,
                        source_task_id: None,
                        error: e.as_reason(),
                    }),
                }
            }
            LinearDraftEntityKind::Issue => {
                let mut request: TaskGraphIssueRequest =
                    serde_json::from_value(entity.payload.clone())
                        .map_err(|e| PublishError::PublishFailed(format!("issue payload: {e}")))?;
                request.correlation_id = correlation_id.into();
                if let Some(milestone_name) = entity.milestone.as_ref()
                    && let Some(id) = milestone_name_to_id.get(milestone_name)
                {
                    request.project_milestone_id = Some(id.clone());
                }
                match client.create_or_update_issue(request, correlation_id).await {
                    Ok(response) => {
                        if let Some(id) = response.issue_id {
                            task_id_to_issue_id.insert(entity.entity_id.clone(), id.clone());
                            if let Some(identifier) = response.issue_identifier {
                                task_id_to_identifier
                                    .insert(entity.entity_id.clone(), identifier.clone());
                                update_running_receipt(
                                    &mut running_receipt,
                                    entity,
                                    &id,
                                    &identifier,
                                    &draft.manifest,
                                    &draft.parsed_tasks,
                                );
                            }
                        }
                    }
                    Err(e) => failures.push(LinearPublishFailure {
                        entity_id: entity.entity_id.clone(),
                        kind: LinearDraftEntityKind::Issue,
                        source_task_id: entity.source_task_id.clone(),
                        error: e.as_reason(),
                    }),
                }
            }
            LinearDraftEntityKind::SubIssue => {
                let mut request: TaskGraphSubIssueRequest =
                    serde_json::from_value(entity.payload.clone()).map_err(|e| {
                        PublishError::PublishFailed(format!("sub-issue payload: {e}"))
                    })?;
                request.correlation_id = correlation_id.into();
                if let Some(parent) = entity.parent_id.as_ref() {
                    match task_id_to_issue_id.get(parent) {
                        Some(id) => {
                            request.parent_id = id.clone();
                            request.parent_identifier = task_id_to_identifier
                                .get(parent)
                                .cloned()
                                .unwrap_or_else(|| parent.clone());
                        }
                        None => {
                            failures.push(LinearPublishFailure {
                                entity_id: entity.entity_id.clone(),
                                kind: LinearDraftEntityKind::SubIssue,
                                source_task_id: entity.source_task_id.clone(),
                                error: format!("parent {parent} was not published"),
                            });
                            continue;
                        }
                    }
                }
                if let Some(milestone_name) = entity.milestone.as_ref()
                    && let Some(id) = milestone_name_to_id.get(milestone_name)
                {
                    request.project_milestone_id = Some(id.clone());
                }
                match client
                    .create_or_update_sub_issue(request, correlation_id)
                    .await
                {
                    Ok(response) => {
                        if let Some(id) = response.sub_issue_id {
                            task_id_to_issue_id.insert(entity.entity_id.clone(), id.clone());
                            if let Some(identifier) = response.sub_issue_identifier {
                                task_id_to_identifier
                                    .insert(entity.entity_id.clone(), identifier.clone());
                                update_running_receipt(
                                    &mut running_receipt,
                                    entity,
                                    &id,
                                    &identifier,
                                    &draft.manifest,
                                    &draft.parsed_tasks,
                                );
                            }
                        }
                    }
                    Err(e) => failures.push(LinearPublishFailure {
                        entity_id: entity.entity_id.clone(),
                        kind: LinearDraftEntityKind::SubIssue,
                        source_task_id: entity.source_task_id.clone(),
                        error: e.as_reason(),
                    }),
                }
            }
            LinearDraftEntityKind::Relation => {
                let mut request: TaskGraphRelationRequest =
                    serde_json::from_value(entity.payload.clone()).map_err(|e| {
                        PublishError::PublishFailed(format!("relation payload: {e}"))
                    })?;
                request.correlation_id = correlation_id.into();
                let blocker = request.issue_id.clone();
                let blocked = request.related_issue_id.clone();
                match (
                    task_id_to_issue_id.get(&blocker),
                    task_id_to_issue_id.get(&blocked),
                ) {
                    (Some(from), Some(to)) => {
                        request.issue_id = from.clone();
                        request.related_issue_id = to.clone();
                    }
                    _ => {
                        failures.push(LinearPublishFailure {
                            entity_id: entity.entity_id.clone(),
                            kind: LinearDraftEntityKind::Relation,
                            source_task_id: entity.source_task_id.clone(),
                            error: format!(
                                "missing Linear ids for relation {blocker} -> {blocked}"
                            ),
                        });
                        continue;
                    }
                }
                match client.create_issue_relation(request, correlation_id).await {
                    Ok(response) => {
                        if let (Some(blocker_source_id), Some(relation_id)) = (
                            entity.source_task_id.as_ref(),
                            response.relation_id.as_ref(),
                        ) {
                            update_running_receipt_relation(
                                &mut running_receipt,
                                blocker_source_id,
                                &blocked,
                                relation_id,
                            );
                        }
                    }
                    Err(e) => failures.push(LinearPublishFailure {
                        entity_id: entity.entity_id.clone(),
                        kind: LinearDraftEntityKind::Relation,
                        source_task_id: entity.source_task_id.clone(),
                        error: e.as_reason(),
                    }),
                }
            }
            LinearDraftEntityKind::Comment => {
                let mut request: TaskGraphEvidenceRequest =
                    serde_json::from_value(entity.payload.clone()).map_err(|e| {
                        PublishError::PublishFailed(format!("comment payload: {e}"))
                    })?;
                request.correlation_id = correlation_id.into();
                let task_id = request.issue_id.clone();
                match task_id_to_issue_id.get(&task_id) {
                    Some(id) => request.issue_id = id.clone(),
                    None => {
                        failures.push(LinearPublishFailure {
                            entity_id: entity.entity_id.clone(),
                            kind: LinearDraftEntityKind::Comment,
                            source_task_id: entity.source_task_id.clone(),
                            error: format!("missing Linear id for comment on {task_id}"),
                        });
                        continue;
                    }
                }
                match client
                    .create_evidence_comment(request, correlation_id)
                    .await
                {
                    Ok(response) => {
                        if let (Some(task_source_id), Some(comment_id)) =
                            (entity.source_task_id.as_ref(), response.comment_id.as_ref())
                        {
                            update_running_receipt_comment(
                                &mut running_receipt,
                                task_source_id,
                                comment_id,
                            );
                        }
                    }
                    Err(e) => failures.push(LinearPublishFailure {
                        entity_id: entity.entity_id.clone(),
                        kind: LinearDraftEntityKind::Comment,
                        source_task_id: entity.source_task_id.clone(),
                        error: e.as_reason(),
                    }),
                }
            }
        }
    }

    running_receipt.published_at = Some(Utc::now());
    let receipt_path = resolve_path_within_repo(
        PathBuf::from(&draft.request.repo_root).as_path(),
        &draft.request.publish_receipt_path,
    )?;
    let yaml = serde_yaml::to_string(&running_receipt)
        .map_err(|e| PublishError::ReceiptWrite(e.to_string()))?;
    atomic_write_file(&receipt_path, &yaml).await?;

    let status = if failures.is_empty() {
        "published"
    } else {
        "partial"
    };
    let receipt = convert_to_api_receipt(&running_receipt);
    let results = build_publish_results(&running_receipt, &draft, &failures);

    Ok(LinearPublishResponse {
        schema_version: SchemaVersion::default(),
        draft_id: draft.draft_id.clone(),
        correlation_id: correlation_id.into(),
        status: status.into(),
        receipt,
        failures,
        results,
    })
}

fn update_running_receipt(
    receipt: &mut YamlPublishReceipt,
    entity: &LinearDraftEntity,
    issue_id: &str,
    identifier: &str,
    _manifest: &TaskPackageManifestFile,
    parsed_tasks: &BTreeMap<TaskId, ParsedTask>,
) {
    let Some(task_id_str) = entity.source_task_id.as_ref() else {
        return;
    };
    let task_id = TaskId::new(task_id_str.clone());
    let Some(task) = parsed_tasks.get(&task_id) else {
        return;
    };
    let kind = match entity.kind {
        LinearDraftEntityKind::Issue => TaskKind::Issue,
        LinearDraftEntityKind::SubIssue => TaskKind::SubIssue,
        _ => return,
    };
    let milestone = entity
        .milestone
        .clone()
        .or_else(|| task.frontmatter.milestone.clone())
        .unwrap_or_default();
    let parent = if kind == TaskKind::SubIssue {
        task.frontmatter.parent.clone().map(TaskId::new)
    } else {
        None
    };
    let url = format!(
        "https://linear.app/{project}/issue/{identifier}/{slug}",
        project = receipt.linear_project.as_deref().unwrap_or("project"),
        identifier = identifier,
        slug = slugify(task.frontmatter.title.as_deref().unwrap_or(identifier))
    );
    let existing = receipt.tasks.get(&task_id).cloned();
    let entry = YamlPublishEntity {
        source_task_id: task_id.clone(),
        source_file: task.file.clone(),
        linear_kind: kind,
        linear_milestone: milestone.clone(),
        parent_task_id: parent,
        blocked_by: task
            .frontmatter
            .blocked_by
            .clone()
            .into_iter()
            .map(TaskId::new)
            .collect(),
        blocks: task
            .frontmatter
            .blocks
            .clone()
            .into_iter()
            .map(TaskId::new)
            .collect(),
        review_comments: Vec::new(),
        issue: Some(identifier.into()),
        issue_id: Some(issue_id.into()),
        url: Some(url),
        comment_ids: existing
            .as_ref()
            .map(|e| e.comment_ids.clone())
            .unwrap_or_default(),
        relation_ids: existing
            .as_ref()
            .map(|e| e.relation_ids.clone())
            .unwrap_or_default(),
    };
    receipt.tasks.insert(task_id.clone(), entry);

    let ms = receipt
        .milestones
        .entry(milestone.clone())
        .or_insert(YamlMilestoneReceipt {
            name: milestone.clone(),
            milestone_id: None,
            linked_issues: Vec::new(),
        });
    if !ms.linked_issues.contains(&task_id) {
        ms.linked_issues.push(task_id);
    }
}
fn update_running_receipt_relation(
    receipt: &mut YamlPublishReceipt,
    blocker_source_id: &str,
    blocked_source_id: &str,
    relation_id: &str,
) {
    let task_id = TaskId::new(blocker_source_id.to_owned());
    if let Some(entry) = receipt.tasks.get_mut(&task_id) {
        entry
            .relation_ids
            .insert(blocked_source_id.to_owned(), relation_id.to_owned());
    }
}

fn update_running_receipt_comment(
    receipt: &mut YamlPublishReceipt,
    task_source_id: &str,
    comment_id: &str,
) {
    let task_id = TaskId::new(task_source_id.to_owned());
    if let Some(entry) = receipt.tasks.get_mut(&task_id) {
        entry.comment_ids.push(comment_id.to_owned());
    }
}

fn convert_to_api_receipt(receipt: &YamlPublishReceipt) -> LinearPublishReceipt {
    LinearPublishReceipt {
        planning_wave: receipt.planning_wave.clone(),
        linear_project: receipt.linear_project.clone().unwrap_or_default(),
        published_at: receipt.published_at.unwrap_or_else(Utc::now),
        milestones: receipt
            .milestones
            .values()
            .map(|ms| PublishedMilestone {
                name: ms.name.clone(),
                milestone_id: ms.milestone_id.clone().unwrap_or_default(),
            })
            .collect(),
        tasks: receipt
            .tasks
            .values()
            .map(|t| PublishedTask {
                task_id: t.source_task_id.to_string(),
                issue: t.issue.clone().unwrap_or_default(),
                issue_id: t.issue_id.clone().unwrap_or_default(),
                url: t.url.clone().unwrap_or_default(),
                file: t.source_file.clone(),
            })
            .collect(),
    }
}

fn build_publish_results(
    receipt: &YamlPublishReceipt,
    draft: &Draft,
    failures: &[LinearPublishFailure],
) -> Vec<LinearPublishResult> {
    let mut results = Vec::new();
    let failure_by_id: BTreeMap<&str, &LinearPublishFailure> =
        failures.iter().map(|f| (f.entity_id.as_str(), f)).collect();
    for task_id in draft
        .manifest
        .tasks
        .iter()
        .map(|e| TaskId::new(e.id.clone()))
    {
        let entry = receipt.tasks.get(&task_id);
        let failed = failure_by_id.get(task_id.to_string().as_str()).copied();
        let status = if failed.is_some() {
            "failed"
        } else if entry.is_some() {
            "published"
        } else {
            "skipped"
        };
        results.push(LinearPublishResult {
            task_id: task_id.to_string(),
            issue: entry.and_then(|e| e.issue.clone()),
            issue_id: entry.and_then(|e| e.issue_id.clone()),
            url: entry.and_then(|e| e.url.clone()),
            file: draft
                .manifest
                .tasks
                .iter()
                .find(|e| e.id == task_id.0)
                .map(|e| e.file.clone())
                .unwrap_or_default(),
            status: status.into(),
            error: failed.map(|f| f.error.clone()),
        });
    }
    results
}

fn rejected_response(
    draft: &Draft,
    request: &LinearPublishRequest,
    reason: &str,
) -> LinearPublishResponse {
    LinearPublishResponse {
        schema_version: SchemaVersion::default(),
        draft_id: request.draft_id.clone(),
        correlation_id: request.correlation_id.clone(),
        status: "rejected".into(),
        receipt: convert_to_api_receipt(&draft.existing_receipt),
        failures: vec![LinearPublishFailure {
            entity_id: "validation".into(),
            kind: LinearDraftEntityKind::Issue,
            source_task_id: None,
            error: reason.into(),
        }],
        results: Vec::new(),
    }
}

fn slugify(title: &str) -> String {
    title
        .to_ascii_lowercase()
        .replace(|c: char| !c.is_alphanumeric(), "-")
        .trim_matches('-')
        .to_string()
}
