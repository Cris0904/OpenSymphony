//! Existing-repo project-set migration for `opensymphony update`.
//!
//! Implements [LOC-20](https://linear.app/localgputokenscrazy/issue/LOC-20/existing-repo-project-set-migration):
//! detect a legacy single-repo `WORKFLOW.md` + `config.yaml` shape that lacks
//! `.opensymphony/project-set.yaml` (or still carries project-set-owned global
//! fields in `WORKFLOW.md`), build a complete migration plan, and rewrite the
//! repo so it can run under the strict project-set runtime boundary from
//! [LOC-18](https://linear.app/localgputokenscrazy/issue/LOC-18/strict-project-set-runtime-boundary).
//!
//! The migration is atomic: the plan is built and validated before any file is
//! written, and either every planned change lands or the on-disk repo is left
//! untouched. Any unsafe auth value, missing/ambiguous remote, or existing
//! project-set conflict aborts the migration with a clear, actionable error.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

use super::project_set_writer::{
    ProjectSetAppliedOutcome, ProjectSetUpsertError, ProjectSetUpsertPlan, project_set_path,
    upsert_project_set_yaml_with_path,
};
use super::repo_detection::{
    GitRemoteDetection, derive_repo_slug_from_dir, derive_repo_slug_from_remote,
    detect_git_default_branch, detect_git_remote_url,
};
use super::util::trimmed_non_empty;
use crate::opensymphony_workflow::{IntegerLike, STALE_MOVED_FIELDS, WorkflowDefinition};

/// Fallback Linear `api_key_env` written when the legacy `tracker.api_key`
/// is omitted from `WORKFLOW.md`.
const DEFAULT_LINEAR_API_KEY_ENV: &str = "LINEAR_API_KEY";

/// Result of classifying a legacy `tracker.api_key` value for migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigratedApiKey {
    /// `tracker.api_key` was omitted in the legacy `WORKFLOW.md`; the
    /// migration writes `project_set.linear.api_key_env: LINEAR_API_KEY`.
    Omitted,
    /// `tracker.api_key` was an exact `$VAR` or `${VAR}` reference; the
    /// migration writes `project_set.linear.api_key_env: VAR`.
    EnvVar(String),
}

/// Outcome of applying a migration plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// The migration wrote at least one new file (project-set and/or
    /// `WORKFLOW.md`).
    Applied {
        project_set: ProjectSetAppliedOutcome,
        workflow_changed: bool,
    },
    /// The repo was already migrated (project-set exists with the same repo
    /// entry and `WORKFLOW.md` carries no project-set-owned global fields).
    /// `WORKFLOW.md` is left untouched.
    NoChangesNeeded {
        project_set_path: PathBuf,
        workflow_path: PathBuf,
    },
}

/// Result of building a migration plan. Pure: no disk writes happen until
/// [`apply_migration_plan`] is called.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub repo_slug: String,
    pub repo_url: String,
    pub default_branch: Option<String>,
    pub project_set_slug: String,
    pub project_slug: String,
    pub linear_project_slug: String,
    pub api_key: MigratedApiKey,
    pub linear_endpoint: Option<String>,
    pub linear_active_states: Option<Vec<String>>,
    pub linear_terminal_states: Option<Vec<String>>,
    pub polling_interval_ms: Option<u64>,
    pub agent_max_concurrent_agents: Option<u64>,
    pub workflow_before: String,
    pub workflow_after: String,
    pub project_set_path: PathBuf,
    pub workflow_path: PathBuf,
    #[allow(dead_code)] // Reserved for future operator-facing summaries and external callers.
    pub config_path: PathBuf,
}

/// Errors that abort the migration before any file is written.
#[derive(Debug, Error)]
pub enum MigrationError {
    #[error(
        "no `WORKFLOW.md` or `config.yaml` in {root}; this directory does not look like an OpenSymphony target repo"
    )]
    NotTargetRepo { root: PathBuf },
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to load workflow at {path}: {source}")]
    LoadWorkflow {
        path: PathBuf,
        #[source]
        source: crate::opensymphony_workflow::WorkflowLoadError,
    },
    #[error("failed to serialize migrated `WORKFLOW.md` front matter: {0}")]
    SerializeWorkflowFrontMatter(String),
    #[error(
        "refusing to migrate: legacy `tracker.api_key` in `WORKFLOW.md` is a literal token rather than an env-var reference. Move the token to an environment variable (e.g. `export LINEAR_API_KEY=...`) and replace `tracker.api_key` with `$LINEAR_API_KEY` or omit it entirely before re-running `opensymphony update`."
    )]
    LiteralApiKey,
    #[error(
        "could not determine the repo remote URL for the project-set inventory. Configure a git remote (`git remote add origin <url>`) and re-run `opensymphony update`, or pass a project-set slug the operator can edit manually before committing."
    )]
    AmbiguousRemote,
    #[error("project-set upsert failed: {0}")]
    ProjectSetUpsert(#[from] ProjectSetUpsertError),
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Classifies a legacy `tracker.api_key` value for migration.
///
/// Rules (LOC-20):
/// - absent / empty -> [`MigratedApiKey::Omitted`] (migrates to `LINEAR_API_KEY`).
/// - exact `$VAR` or `${VAR}` reference -> [`MigratedApiKey::EnvVar`] (migrates to `VAR`).
/// - any other value -> `Err(MigrationError::LiteralApiKey)`; the migration
///   refuses to write the literal token to `.opensymphony/project-set.yaml`.
pub fn classify_legacy_api_key(value: Option<&str>) -> Result<MigratedApiKey, MigrationError> {
    let trimmed = trimmed_non_empty(value);
    let Some(value) = trimmed else {
        return Ok(MigratedApiKey::Omitted);
    };
    let bytes = value.as_bytes();
    if let Some(rest) = bytes.strip_prefix(b"${") {
        // Reject `${...}` shapes that are not an exact `${VAR}` reference:
        // anything after the closing `}` (including whitespace) means the
        // value is a literal token, not an env-var reference.
        if let Some(close) = rest.iter().position(|byte| *byte == b'}') {
            let var_bytes = &rest[..close];
            let after_brace = &rest[close + 1..];
            if after_brace.is_empty() && is_valid_env_var(var_bytes) {
                let var =
                    std::str::from_utf8(var_bytes).map_err(|_| MigrationError::LiteralApiKey)?;
                return Ok(MigratedApiKey::EnvVar(var.to_owned()));
            }
        }
        return Err(MigrationError::LiteralApiKey);
    }
    if let Some(rest) = bytes.strip_prefix(b"$") {
        if is_valid_env_var(rest) {
            let var = std::str::from_utf8(rest).map_err(|_| MigrationError::LiteralApiKey)?;
            return Ok(MigratedApiKey::EnvVar(var.to_owned()));
        }
        return Err(MigrationError::LiteralApiKey);
    }
    Err(MigrationError::LiteralApiKey)
}

/// Returns `true` when `bytes` is a non-empty POSIX-style environment
/// variable name: it must start with an ASCII letter or underscore, and the
/// remaining characters must be ASCII letters, digits, or underscores.
fn is_valid_env_var(bytes: &[u8]) -> bool {
    let Some((first, rest)) = bytes.split_first() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || *first == b'_') {
        return false;
    }
    rest.iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn api_key_env_from_classifier(api_key: &MigratedApiKey) -> String {
    match api_key {
        MigratedApiKey::Omitted => DEFAULT_LINEAR_API_KEY_ENV.to_owned(),
        MigratedApiKey::EnvVar(var) => var.clone(),
    }
}

fn detect_legacy_active_states(workflow: &WorkflowDefinition) -> Option<Vec<String>> {
    workflow.front_matter.tracker.active_states.clone()
}

fn detect_legacy_terminal_states(workflow: &WorkflowDefinition) -> Option<Vec<String>> {
    workflow.front_matter.tracker.terminal_states.clone()
}

fn detect_legacy_polling_interval_ms(workflow: &WorkflowDefinition) -> Option<u64> {
    match workflow.front_matter.polling.interval_ms.as_ref() {
        Some(IntegerLike::Integer(value)) if *value > 0 => Some(*value as u64),
        Some(IntegerLike::String(value)) => value.parse::<u64>().ok(),
        _ => None,
    }
}

fn detect_legacy_max_concurrent_agents(workflow: &WorkflowDefinition) -> Option<u64> {
    match workflow.front_matter.agent.max_concurrent_agents.as_ref() {
        Some(IntegerLike::Integer(value)) if *value > 0 => Some(*value as u64),
        Some(IntegerLike::String(value)) => value.parse::<u64>().ok(),
        _ => None,
    }
}

fn tracker_endpoint(workflow: &WorkflowDefinition) -> Option<String> {
    workflow.front_matter.tracker.endpoint.clone()
}

fn tracker_project_slug(workflow: &WorkflowDefinition) -> Option<String> {
    trimmed_non_empty(workflow.front_matter.tracker.project_slug.as_deref())
}

/// Rewrites the legacy `WORKFLOW.md` front matter to drop the
/// [`STALE_MOVED_FIELDS`] entries while keeping every other field and the
/// trailing prompt body byte-identical.
///
/// Returns the source unchanged when the parsed front matter carries none of
/// the canonical stale fields; this avoids an unnecessary `serde_yaml`
/// round-trip that would otherwise add a trailing newline to an already
/// clean document and lets the migration report `workflow_changed: false`.
///
/// Uses `serde_yaml::Value` for the in-place manipulation so we never have to
/// serialize `WorkflowFrontMatter` (which intentionally does not implement
/// `Serialize`) and so unrelated extensions are preserved (LOC-20).
fn build_legacy_workflow_after(
    source: &str,
    workflow_definition: &WorkflowDefinition,
) -> Result<String, MigrationError> {
    if !workflow_needs_rewrite(&workflow_definition.front_matter) {
        return Ok(source.to_owned());
    }

    let Some(split) = crate::opensymphony_workflow::split_front_matter(source) else {
        // No front-matter markers: nothing to migrate.
        return Ok(source.to_owned());
    };

    let mut value = serde_yaml::from_str::<serde_yaml::Value>(split.front_matter)
        .map_err(|source| MigrationError::SerializeWorkflowFrontMatter(source.to_string()))?;
    let Some(mapping) = value.as_mapping_mut() else {
        // Front matter is not a YAML map: refuse to rewrite so we don't
        // damage an unusual legacy layout. The migration plan was built
        // from this same input and the resolver would already have
        // accepted it; the strict project-set mode will surface the
        // remaining stale fields to the operator.
        return Ok(source.to_owned());
    };

    for entry in STALE_MOVED_FIELDS {
        match entry.field.split_once('.') {
            None => {
                mapping.remove(entry.field);
            }
            Some((parent, child)) => {
                if let Some(parent_mapping) =
                    mapping.get_mut(parent).and_then(|v| v.as_mapping_mut())
                {
                    parent_mapping.remove(child);
                }
            }
        }
    }

    // After removing stale sub-fields, drop empty parent keys so the
    // migrated `WORKFLOW.md` is already valid under the strict project-set
    // boundary (no empty `tracker:` / `polling:` / `agent:` mappings).
    let stale_parents: BTreeSet<&str> = STALE_MOVED_FIELDS
        .iter()
        .filter_map(|entry| entry.field.split_once('.').map(|(parent, _)| parent))
        .collect();
    for parent in &stale_parents {
        let is_empty_mapping = mapping
            .get(*parent)
            .and_then(|value| value.as_mapping())
            .is_some_and(|m| m.is_empty());
        if is_empty_mapping {
            mapping.remove(*parent);
        }
    }

    let serialized_front_matter = serde_yaml::to_string(&value)
        .map_err(|source| MigrationError::SerializeWorkflowFrontMatter(source.to_string()))?;

    Ok(format!(
        "{head}{serialized}{trailer}{body}",
        head = split.head,
        serialized = serialized_front_matter,
        trailer = split.trailer,
        body = split.body
    ))
}

/// Returns `true` when the on-disk `WORKFLOW.md` still carries any of the
/// canonical [`STALE_MOVED_FIELDS`]; the migration then needs to rewrite it.
fn workflow_needs_rewrite(workflow: &crate::opensymphony_workflow::WorkflowFrontMatter) -> bool {
    STALE_MOVED_FIELDS
        .iter()
        .any(|entry| workflow.has_stale_field(entry.field).unwrap_or(false))
}

/// Builds a migration plan from the on-disk `WORKFLOW.md`, `config.yaml`, and
/// the local git repository. Pure: reads files, does not write anything.
pub fn plan_migration(target_repo: &Path) -> Result<MigrationPlan, MigrationError> {
    let workflow_path = target_repo.join("WORKFLOW.md");
    let config_path = target_repo.join("config.yaml");
    if !workflow_path.is_file() || !config_path.is_file() {
        return Err(MigrationError::NotTargetRepo {
            root: target_repo.to_path_buf(),
        });
    }

    let workflow_before =
        fs::read_to_string(&workflow_path).map_err(|source| MigrationError::ReadFile {
            path: workflow_path.clone(),
            source,
        })?;

    let workflow_definition = WorkflowDefinition::parse(&workflow_before).map_err(|source| {
        MigrationError::LoadWorkflow {
            path: workflow_path.clone(),
            source,
        }
    })?;

    // Auth classifier runs before any other step so literal tokens abort the
    // migration before we touch disk.
    let api_key =
        classify_legacy_api_key(workflow_definition.front_matter.tracker.api_key.as_deref())?;

    let remote = detect_git_remote_url(target_repo);
    let repo_url = match &remote {
        GitRemoteDetection::Selected { url, .. } => Some(url.clone()),
        GitRemoteDetection::None => None,
        GitRemoteDetection::Ambiguous(_) => {
            return Err(MigrationError::AmbiguousRemote);
        }
    };
    let repo_url = match repo_url {
        Some(url) => url,
        None => {
            // The shared helper distinguishes "no remotes" from "ambiguous
            // remotes"; both surface as `MigrationError::AmbiguousRemote` so
            // operators get one actionable error path.
            return Err(MigrationError::AmbiguousRemote);
        }
    };

    let default_branch = remote
        .remote_name()
        .and_then(|name| detect_git_default_branch(target_repo, name));

    let derived_slug =
        derive_repo_slug_from_remote(&repo_url).or_else(|| derive_repo_slug_from_dir(target_repo));
    let repo_slug = derived_slug.unwrap_or_else(|| "repo".to_owned());

    let project_set_slug = "default-project-set".to_owned();
    // When the project-set file already exists, prefer the project and
    // linear slugs it owns. The legacy `WORKFLOW.md` may have been rewritten
    // by a previous migration pass and no longer carries the source slugs;
    // trusting the project-set keeps the inventory stable on repeat runs.
    let project_set_file = project_set_path(target_repo);
    let existing_project_set =
        super::project_set_writer::read_project_set_front_matter(&project_set_file)
            .map_err(MigrationError::ProjectSetUpsert)?;
    let (project_slug, linear_project_slug) = match &existing_project_set {
        Some(existing) => {
            let first_project_slug = existing
                .project_set
                .projects
                .first()
                .and_then(|p| p.slug.clone())
                .unwrap_or_else(|| repo_slug.clone());
            let existing_linear_slug = existing
                .project_set
                .linear
                .project_slug
                .clone()
                .unwrap_or_else(|| first_project_slug.clone());
            let from_workflow_linear = tracker_project_slug(&workflow_definition);
            (
                first_project_slug,
                from_workflow_linear.unwrap_or(existing_linear_slug),
            )
        }
        None => {
            let linear_project_slug =
                tracker_project_slug(&workflow_definition).unwrap_or_else(|| repo_slug.clone());
            (linear_project_slug.clone(), linear_project_slug)
        }
    };

    let workflow_after = build_legacy_workflow_after(&workflow_before, &workflow_definition)?;

    Ok(MigrationPlan {
        repo_slug,
        repo_url,
        default_branch,
        project_set_slug,
        project_slug,
        linear_project_slug,
        api_key,
        linear_endpoint: tracker_endpoint(&workflow_definition),
        linear_active_states: detect_legacy_active_states(&workflow_definition),
        linear_terminal_states: detect_legacy_terminal_states(&workflow_definition),
        polling_interval_ms: detect_legacy_polling_interval_ms(&workflow_definition),
        agent_max_concurrent_agents: detect_legacy_max_concurrent_agents(&workflow_definition),
        workflow_before,
        workflow_after,
        project_set_path: project_set_path(target_repo),
        workflow_path,
        config_path,
    })
}

/// Applies a migration plan to disk.
///
/// Atomicity: the plan is already validated by [`plan_migration`], so this
/// function performs the writes in an order that lets the operator re-run
/// `opensymphony update` safely if the process is interrupted mid-write:
///
/// 1. Write `.opensymphony/project-set.yaml` (the project-set writer is
///    idempotent and tolerates an already-up-to-date file).
/// 2. Rewrite `WORKFLOW.md` only when the post-migration content differs
///    from the source bytes.
///
/// The function never writes a placeholder repo URL: if the plan was built
/// without a confidently detected git remote, [`plan_migration`] already
/// returned [`MigrationError::AmbiguousRemote`].
pub fn apply_migration_plan(plan: &MigrationPlan) -> Result<MigrationOutcome, MigrationError> {
    let upsert_plan = ProjectSetUpsertPlan {
        repo_slug: plan.repo_slug.clone(),
        repo_url: plan.repo_url.clone(),
        default_branch: plan.default_branch.clone(),
        project_set_slug: plan.project_set_slug.clone(),
        project_slug: plan.project_slug.clone(),
        linear_project_slug: plan.linear_project_slug.clone(),
        linear_api_key_env: Some(api_key_env_from_classifier(&plan.api_key)),
        polling_interval_ms: plan.polling_interval_ms,
        max_concurrent_agents: plan.agent_max_concurrent_agents,
        linear_active_states: plan.linear_active_states.clone(),
        linear_terminal_states: plan.linear_terminal_states.clone(),
        linear_endpoint: plan.linear_endpoint.clone(),
    };

    let project_set_outcome =
        upsert_project_set_yaml_with_path(&plan.project_set_path, &upsert_plan)
            .map_err(MigrationError::ProjectSetUpsert)?;

    let workflow_changed = if plan.workflow_before == plan.workflow_after {
        false
    } else {
        if let Some(parent) = plan.workflow_path.parent() {
            fs::create_dir_all(parent).map_err(|source| MigrationError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&plan.workflow_path, &plan.workflow_after).map_err(|source| {
            MigrationError::WriteFile {
                path: plan.workflow_path.clone(),
                source,
            }
        })?;
        true
    };

    Ok(MigrationOutcome::Applied {
        project_set: project_set_outcome,
        workflow_changed,
    })
}

/// One-shot helper that builds and applies a migration plan against
/// `target_repo`.
///
/// Returns [`MigrationOutcome::Applied`] when at least one file changed, or
/// [`MigrationOutcome::NoChangesNeeded`] when the repo is already migrated
/// (project-set already lists this repo entry and `WORKFLOW.md` carries no
/// project-set-owned global fields).
pub fn run_migration(target_repo: &Path) -> Result<MigrationOutcome, MigrationError> {
    let workflow_path = target_repo.join("WORKFLOW.md");
    let config_path = target_repo.join("config.yaml");
    if !workflow_path.is_file() || !config_path.is_file() {
        return Err(MigrationError::NotTargetRepo {
            root: target_repo.to_path_buf(),
        });
    }
    let project_set_path = project_set_path(target_repo);

    // Pre-flight check: if the repo is already in project-set mode (the
    // project-set file exists with the current repo's slug/URL registered
    // and `WORKFLOW.md` carries no stale moved fields) report
    // `NoChangesNeeded` without re-planning. Without this short-circuit the
    // second pass would re-derive the `project_slug` from a now-clean
    // `WORKFLOW.md` (which no longer has `tracker.project_slug`) and create
    // a duplicate project-set project entry.
    if let Some(outcome) = check_already_migrated(target_repo, &workflow_path, &project_set_path)? {
        return Ok(outcome);
    }

    let plan = plan_migration(target_repo)?;
    let outcome = apply_migration_plan(&plan)?;
    Ok(outcome)
}

/// Returns `Some(MigrationOutcome::NoChangesNeeded { .. })` when the repo
/// is already on the project-set runtime boundary. The repo qualifies when
/// at least one of these holds:
///
/// 1. `WORKFLOW.md` carries no stale moved fields and no project-set file
///    exists yet: there is nothing to migrate yet, so the migration is a
///    safe no-op (this lets `opensymphony update` work on freshly
///    initialized OpenSymphony repos that have not yet configured any
///    git remote).
/// 2. The existing `.opensymphony/project-set.yaml` already lists this
///    repo entry AND `WORKFLOW.md` carries none of the canonical
///    [`STALE_MOVED_FIELDS`].
fn check_already_migrated(
    target_repo: &Path,
    workflow_path: &Path,
    project_set_path: &Path,
) -> Result<Option<MigrationOutcome>, MigrationError> {
    let workflow_source =
        fs::read_to_string(workflow_path).map_err(|source| MigrationError::ReadFile {
            path: workflow_path.to_path_buf(),
            source,
        })?;
    let workflow_definition = WorkflowDefinition::parse(&workflow_source).map_err(|source| {
        MigrationError::LoadWorkflow {
            path: workflow_path.to_path_buf(),
            source,
        }
    })?;
    if workflow_needs_rewrite(&workflow_definition.front_matter) {
        return Ok(None);
    }

    let existing_project_set =
        match super::project_set_writer::read_project_set_front_matter(project_set_path) {
            Ok(Some(front_matter)) => front_matter,
            Ok(None) => {
                // No stale fields in WORKFLOW.md and no project-set file yet:
                // there is nothing to migrate, so treat this as already-migrated
                // so the operator-facing update flow continues cleanly even
                // when no git remote is configured.
                return Ok(Some(MigrationOutcome::NoChangesNeeded {
                    project_set_path: project_set_path.to_path_buf(),
                    workflow_path: workflow_path.to_path_buf(),
                }));
            }
            Err(source) => {
                return Err(MigrationError::ProjectSetUpsert(source));
            }
        };

    // Detect the git remote exactly like `plan_migration` would so the
    // already-migrated detection matches what the migration would do.
    let remote = detect_git_remote_url(target_repo);
    let repo_url = match &remote {
        GitRemoteDetection::Selected { url, .. } => url.clone(),
        _ => return Ok(None),
    };
    let derived_slug =
        derive_repo_slug_from_remote(&repo_url).or_else(|| derive_repo_slug_from_dir(target_repo));
    let Some(repo_slug) = derived_slug else {
        return Ok(None);
    };

    let repo_in_inventory = existing_project_set
        .project_set
        .projects
        .iter()
        .flat_map(|project| project.repos.iter())
        .any(|entry| {
            entry.slug.as_deref() == Some(repo_slug.as_str())
                && entry.url.as_deref() == Some(repo_url.as_str())
        });
    if !repo_in_inventory {
        return Ok(None);
    }

    Ok(Some(MigrationOutcome::NoChangesNeeded {
        project_set_path: project_set_path.to_path_buf(),
        workflow_path: workflow_path.to_path_buf(),
    }))
}

/// Renders the operator-facing summary for a migration outcome. Used by
/// `opensymphony update` to print what changed and what (if anything) needs
/// manual action.
pub fn render_summary(outcome: &MigrationOutcome, workflow_path: &Path) -> String {
    match outcome {
        MigrationOutcome::NoChangesNeeded { .. } => {
            "Migration: repo is already project-set mode; no files changed.".to_owned()
        }
        MigrationOutcome::Applied {
            project_set,
            workflow_changed,
        } => {
            let mut lines = Vec::new();
            match project_set {
                ProjectSetAppliedOutcome::Created(path) => {
                    lines.push(format!("Migration: created {}", path.display()));
                }
                ProjectSetAppliedOutcome::Updated(path) => {
                    lines.push(format!("Migration: updated {}", path.display()));
                }
                ProjectSetAppliedOutcome::Unchanged(path) => {
                    lines.push(format!(
                        "Migration: project-set inventory at {} is already up to date",
                        path.display()
                    ));
                }
            }
            if *workflow_changed {
                lines.push(format!(
                    "Migration: rewrote {} to remove project-set-owned global fields",
                    workflow_path.display()
                ));
            } else {
                lines.push(format!(
                    "Migration: {} was already free of project-set-owned global fields",
                    workflow_path.display()
                ));
            }
            lines.join("\n")
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow_source(front_matter: &str) -> String {
        format!(
            "---\n{front_matter}\n---\n\nYou are working on a Linear ticket `{{{{ issue.identifier }}}}`\n"
        )
    }

    #[test]
    fn classify_legacy_api_key_omits_when_unset() {
        assert_eq!(
            classify_legacy_api_key(None).expect("omitted should classify"),
            MigratedApiKey::Omitted
        );
        assert_eq!(
            classify_legacy_api_key(Some("")).expect("empty should classify"),
            MigratedApiKey::Omitted
        );
        assert_eq!(
            classify_legacy_api_key(Some("   ")).expect("whitespace should classify"),
            MigratedApiKey::Omitted
        );
    }

    #[test]
    fn classify_legacy_api_key_accepts_bare_and_braced_env_vars() {
        assert_eq!(
            classify_legacy_api_key(Some("$LINEAR_API_KEY")).expect("bare env should classify"),
            MigratedApiKey::EnvVar("LINEAR_API_KEY".to_owned())
        );
        assert_eq!(
            classify_legacy_api_key(Some("${LINEAR_API_KEY}")).expect("braced env should classify"),
            MigratedApiKey::EnvVar("LINEAR_API_KEY".to_owned())
        );
        assert_eq!(
            classify_legacy_api_key(Some("$TEAM_TOKEN")).expect("bare custom env should classify"),
            MigratedApiKey::EnvVar("TEAM_TOKEN".to_owned())
        );
        assert_eq!(
            classify_legacy_api_key(Some("${TEAM_TOKEN}"))
                .expect("braced custom env should classify"),
            MigratedApiKey::EnvVar("TEAM_TOKEN".to_owned())
        );
    }

    #[test]
    fn classify_legacy_api_key_rejects_literal_tokens() {
        for value in [
            "lin_api_abcdef0123456789",
            "lin_api_abcdef0123456789$LITERAL",
            "$1LINEAR",
            "${UNCLOSED",
            "${WITH SPACE}",
            "${VAR-EXTRA}",
            "${VAR}extra",
            "${VAR} extra",
            "prefix$LITERAL",
        ] {
            assert!(
                matches!(
                    classify_legacy_api_key(Some(value)),
                    Err(MigrationError::LiteralApiKey)
                ),
                "expected literal-api-key error for value `{value}`, got: {:?}",
                classify_legacy_api_key(Some(value))
            );
        }
    }

    #[test]
    fn classify_legacy_api_key_rejects_braced_env_var_with_suffix() {
        // Regression: `${VAR}extra` and `${VAR} extra` are NOT exact `${VAR}`
        // references — anything after the closing brace means the legacy value
        // is a literal token. The classifier must reject it before any file is
        // written (LOC-20 safe-auth rules).
        for value in ["${VAR}extra", "${VAR} extra"] {
            assert!(
                matches!(
                    classify_legacy_api_key(Some(value)),
                    Err(MigrationError::LiteralApiKey)
                ),
                "expected literal-api-key error for value `{value}`, got: {:?}",
                classify_legacy_api_key(Some(value))
            );
        }
    }

    #[test]
    fn rewrite_workflow_preserves_prompt_body_byte_identical() {
        let source = workflow_source(
            r#"tracker:
  kind: linear
  project_slug: opensymphony-bootstrap
  active_states:
    - Todo
polling:
  interval_ms: 5000
agent:
  max_concurrent_agents: 8
workspace:
  root: ~/.opensymphony/workspaces
hooks:
  after_create: |
    opensymphony workspace clone
openhands:
  transport:
    base_url: http://127.0.0.1:8000
"#,
        );

        let workflow = WorkflowDefinition::parse(&source).expect("parse should succeed");
        let rewritten = build_legacy_workflow_after(&source, &workflow).expect("rewrite");
        assert!(
            rewritten.contains("workspace:\n  root: ~/.opensymphony/workspaces"),
            "workspace block must be preserved: {rewritten}"
        );
        assert!(
            rewritten.contains("hooks:\n  after_create: |"),
            "hooks block must be preserved: {rewritten}"
        );
        assert!(
            rewritten.contains("base_url: http://127.0.0.1:8000"),
            "openhands block must be preserved: {rewritten}"
        );
        assert!(
            !rewritten.contains("kind: linear"),
            "stale tracker.kind must be removed: {rewritten}"
        );
        assert!(
            !rewritten.contains("interval_ms"),
            "stale polling.interval_ms must be removed: {rewritten}"
        );
        assert!(
            !rewritten.contains("max_concurrent_agents"),
            "stale agent.max_concurrent_agents must be removed: {rewritten}"
        );
        let body_start = rewritten
            .find("You are working on a Linear ticket")
            .expect("prompt body must remain");
        let trailing = &rewritten[body_start..];
        assert_eq!(
            trailing, "You are working on a Linear ticket `{{ issue.identifier }}`\n",
            "prompt body must be byte-identical"
        );
    }

    #[test]
    fn rewrite_workflow_is_noop_when_already_clean() {
        let source = workflow_source("workspace:\n  root: ~/.opensymphony/workspaces\n");
        let workflow = WorkflowDefinition::parse(&source).expect("parse should succeed");
        let rewritten = build_legacy_workflow_after(&source, &workflow).expect("rewrite");
        assert_eq!(
            rewritten, source,
            "clean workflow must round-trip unchanged"
        );
    }

    #[test]
    fn rewrite_workflow_drops_empty_parent_mappings() {
        let source = workflow_source(
            r#"tracker:
  kind: linear
workspace:
  root: ~/.opensymphony/workspaces
"#,
        );
        let workflow = WorkflowDefinition::parse(&source).expect("parse should succeed");
        let rewritten = build_legacy_workflow_after(&source, &workflow).expect("rewrite");
        // After removing every `tracker.*` field, the empty `tracker:`
        // mapping must also be dropped so the result is valid under the
        // strict project-set boundary.
        assert!(
            !rewritten.contains("tracker:"),
            "empty tracker mapping must be dropped: {rewritten}"
        );
        assert!(
            rewritten.contains("workspace:"),
            "workspace block must remain: {rewritten}"
        );
    }

    #[test]
    fn fresh_plan_writes_full_project_set_from_legacy_workflow() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path();
        fs::create_dir_all(repo.join(".git")).expect("git dir");
        let workflow = workflow_source(
            r#"tracker:
  kind: linear
  project_slug: opensymphony-bootstrap
  api_key: $LINEAR_API_KEY
  active_states:
    - Todo
    - In Progress
  terminal_states:
    - Done
polling:
  interval_ms: 5000
agent:
  max_concurrent_agents: 8
  max_turns: 40
workspace:
  root: ~/.opensymphony/workspaces
hooks:
  after_create: |
    opensymphony workspace clone
"#,
        );
        fs::write(repo.join("WORKFLOW.md"), &workflow).expect("workflow write");
        fs::write(
            repo.join("config.yaml"),
            "openhands:\n  tool_dir: ~/.opensymphony\n",
        )
        .expect("config write");
        // Fake a git remote so `detect_git_remote_url` finds one.
        let git = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo)
            .output();
        if let Ok(output) = git {
            assert!(output.status.success(), "git init should succeed");
        }
        let _ = std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://example.com/demo.git"])
            .current_dir(repo)
            .output();

        let plan = plan_migration(repo).expect("plan should succeed");
        assert_eq!(plan.repo_url, "https://example.com/demo.git");
        assert_eq!(plan.repo_slug, "demo");
        assert_eq!(
            plan.api_key,
            MigratedApiKey::EnvVar("LINEAR_API_KEY".to_owned())
        );
        assert_eq!(plan.polling_interval_ms, Some(5_000));
        assert_eq!(plan.agent_max_concurrent_agents, Some(8));

        let outcome = apply_migration_plan(&plan).expect("apply should succeed");
        match &outcome {
            MigrationOutcome::Applied {
                project_set,
                workflow_changed,
            } => {
                assert!(matches!(project_set, ProjectSetAppliedOutcome::Created(_)));
                assert!(*workflow_changed);
            }
            _ => panic!("expected Applied outcome, got {outcome:?}"),
        }

        let project_set_yaml =
            fs::read_to_string(&plan.project_set_path).expect("project-set should exist");
        assert!(project_set_yaml.contains("schema_version: 1"));
        assert!(project_set_yaml.contains("slug: default-project-set"));
        assert!(project_set_yaml.contains("project_slug: opensymphony-bootstrap"));
        assert!(project_set_yaml.contains("api_key_env: LINEAR_API_KEY"));
        assert!(project_set_yaml.contains("interval_ms: 5000"));
        assert!(project_set_yaml.contains("max_concurrent_agents: 8"));
        assert!(project_set_yaml.contains("- slug: demo"));
        assert!(project_set_yaml.contains("url: https://example.com/demo.git"));

        let workflow_after =
            fs::read_to_string(&plan.workflow_path).expect("workflow should exist");
        assert!(
            !workflow_after.contains("tracker:"),
            "tracker block must be removed: {workflow_after}"
        );
        assert!(
            !workflow_after.contains("polling:"),
            "polling block must be removed: {workflow_after}"
        );
        assert!(
            !workflow_after.contains("max_concurrent_agents"),
            "agent.max_concurrent_agents must be removed: {workflow_after}"
        );
        assert!(
            workflow_after.contains("workspace:") && workflow_after.contains("hooks:"),
            "workspace and hooks must survive: {workflow_after}"
        );
        assert!(
            workflow_after.contains("max_turns: 40"),
            "agent.max_turns must survive: {workflow_after}"
        );
        let body_start = workflow_after
            .find("You are working on a Linear ticket")
            .expect("prompt body must remain");
        assert_eq!(
            &workflow_after[body_start..],
            "You are working on a Linear ticket `{{ issue.identifier }}`\n",
            "prompt body must be byte-identical"
        );
    }

    #[test]
    fn plan_fails_with_clear_error_for_literal_api_key() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path();
        fs::write(
            repo.join("WORKFLOW.md"),
            workflow_source(
                r#"tracker:
  kind: linear
  api_key: lin_api_abcdef0123456789
"#,
            ),
        )
        .expect("workflow write");
        fs::write(
            repo.join("config.yaml"),
            "openhands:\n  tool_dir: ~/.opensymphony\n",
        )
        .expect("config write");
        let err = plan_migration(repo).expect_err("literal token must abort");
        assert!(
            matches!(err, MigrationError::LiteralApiKey),
            "expected LiteralApiKey, got {err:?}"
        );
        assert!(
            !repo.join(".opensymphony/project-set.yaml").exists(),
            "no project-set file should be written when auth aborts"
        );
        assert_eq!(
            fs::read_to_string(repo.join("WORKFLOW.md")).expect("workflow"),
            workflow_source(
                r#"tracker:
  kind: linear
  api_key: lin_api_abcdef0123456789
"#
            ),
            "WORKFLOW.md must be untouched when auth aborts"
        );
    }

    #[test]
    fn plan_fails_for_missing_remote_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path();
        // No git directory at all -> `detect_git_remote_url` returns None.
        fs::write(
            repo.join("WORKFLOW.md"),
            workflow_source(
                r#"tracker:
  kind: linear
  project_slug: opensymphony-bootstrap
"#,
            ),
        )
        .expect("workflow write");
        fs::write(
            repo.join("config.yaml"),
            "openhands:\n  tool_dir: ~/.opensymphony\n",
        )
        .expect("config write");
        let err = plan_migration(repo).expect_err("missing remote must abort");
        assert!(
            matches!(err, MigrationError::AmbiguousRemote),
            "expected AmbiguousRemote, got {err:?}"
        );
        assert!(
            !repo.join(".opensymphony/project-set.yaml").exists(),
            "no project-set file should be written"
        );
    }

    #[test]
    fn repeated_plan_is_idempotent() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path();
        fs::create_dir_all(repo.join(".git")).expect("git dir");
        let workflow = workflow_source(
            r#"tracker:
  kind: linear
  project_slug: opensymphony-bootstrap
polling:
  interval_ms: 5000
agent:
  max_concurrent_agents: 8
workspace:
  root: ~/.opensymphony/workspaces
hooks:
  after_create: |
    opensymphony workspace clone
"#,
        );
        fs::write(repo.join("WORKFLOW.md"), &workflow).expect("workflow write");
        fs::write(
            repo.join("config.yaml"),
            "openhands:\n  tool_dir: ~/.opensymphony\n",
        )
        .expect("config write");
        let _ = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo)
            .output();
        let _ = std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://example.com/demo.git"])
            .current_dir(repo)
            .output();

        let plan = plan_migration(repo).expect("plan should succeed");
        let outcome1 = apply_migration_plan(&plan).expect("apply should succeed");
        assert!(matches!(
            outcome1,
            MigrationOutcome::Applied {
                project_set: ProjectSetAppliedOutcome::Created(_),
                workflow_changed: true,
            }
        ));

        // Second pass: re-read workflow, build a fresh plan, re-apply.
        let plan2 = plan_migration(repo).expect("second plan should succeed");
        let outcome2 = apply_migration_plan(&plan2).expect("second apply should succeed");
        assert!(
            matches!(
                outcome2,
                MigrationOutcome::Applied {
                    project_set: ProjectSetAppliedOutcome::Unchanged(_),
                    workflow_changed: false,
                }
            ),
            "second migration must be a no-op for inventory + workflow, got {outcome2:?}"
        );
    }

    #[test]
    fn existing_project_set_with_conflicting_slug_fails() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path();
        fs::create_dir_all(repo.join(".git")).expect("git dir");
        let workflow = workflow_source(
            r#"tracker:
  kind: linear
  project_slug: opensymphony-bootstrap
"#,
        );
        fs::write(repo.join("WORKFLOW.md"), &workflow).expect("workflow write");
        fs::write(
            repo.join("config.yaml"),
            "openhands:\n  tool_dir: ~/.opensymphony\n",
        )
        .expect("config write");
        let _ = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo)
            .output();
        let _ = std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://example.com/demo.git"])
            .current_dir(repo)
            .output();

        // Seed an existing project-set file with the same slug but a
        // different URL -> the upsert surfaces a `ConflictingRepoUrl`.
        fs::create_dir_all(repo.join(".opensymphony")).expect("opensymphony dir");
        fs::write(
            repo.join(".opensymphony/project-set.yaml"),
            "schema_version: 1\nproject_set:\n  slug: default-project-set\n  linear:\n    project_slug: opensymphony-bootstrap\n    api_key_env: LINEAR_API_KEY\n  projects:\n    - slug: opensymphony-bootstrap\n      repos:\n        - slug: demo\n          url: https://example.com/other.git\n",
        )
        .expect("seeded project-set");
        let plan = plan_migration(repo).expect("plan should succeed");
        let err = apply_migration_plan(&plan).expect_err("conflict must abort");
        let msg = format!("{err}");
        assert!(
            msg.contains("conflicting") || msg.contains("different URL"),
            "expected conflict message, got: {msg}"
        );
        assert!(
            !repo.join(".opensymphony/project-set.yaml.tmp").exists(),
            "no temp files should leak on conflict"
        );
    }

    #[test]
    fn render_summary_for_applied_outcome_lists_changed_files() {
        let outcome = MigrationOutcome::Applied {
            project_set: ProjectSetAppliedOutcome::Created(PathBuf::from(
                "/tmp/.opensymphony/project-set.yaml",
            )),
            workflow_changed: true,
        };
        let summary = render_summary(&outcome, Path::new("/tmp/WORKFLOW.md"));
        assert!(summary.contains("created /tmp/.opensymphony/project-set.yaml"));
        assert!(summary.contains("rewrote /tmp/WORKFLOW.md"));
    }

    #[test]
    fn render_summary_for_no_changes_needed() {
        let outcome = MigrationOutcome::NoChangesNeeded {
            project_set_path: PathBuf::from("/tmp/.opensymphony/project-set.yaml"),
            workflow_path: PathBuf::from("/tmp/WORKFLOW.md"),
        };
        let summary = render_summary(&outcome, Path::new("/tmp/WORKFLOW.md"));
        assert!(summary.contains("already project-set mode"));
    }

    #[test]
    fn not_target_repo_error_is_distinct() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path();
        fs::write(repo.join("WORKFLOW.md"), "workspace:\n  root: /tmp\n").expect("workflow write");
        // Intentionally omit `config.yaml`.
        let err = plan_migration(repo).expect_err("missing config.yaml");
        assert!(
            matches!(err, MigrationError::NotTargetRepo { .. }),
            "expected NotTargetRepo, got {err:?}"
        );
    }

    // The unused-import lints can complain about the helper types above when
    // the test module is compiled standalone; this no-op assertion is a
    // cheap way to keep them referenced.
    #[test]
    fn helper_types_remain_referenced() {
        let _ = BTreeSet::<String>::new();
    }
}
