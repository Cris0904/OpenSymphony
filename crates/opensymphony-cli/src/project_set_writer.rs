//! Reusable raw YAML writer/upsert for `<config_root>/.opensymphony/project-set.yaml`.
//!
//! Used by [`crate::opensymphony_cli::init_repo`] for fresh-bootstrap
//! onboarding ([LOC-19](https://linear.app/localgputokenscrazy/issue/LOC-19/init-multi-repo-onboarding))
//! and intended to be reused by the
//! [LOC-20](https://linear.app/localgputokenscrazy/issue/LOC-20/existing-repo-project-set-migration)
//! migration path. Only operates on raw
//! [`opensymphony_workflow::ProjectSetFrontMatter`] — never on
//! [`opensymphony_workflow::ResolvedProjectSet`] — so we never round-trip
//! resolved Linear secrets through serialization.

use std::{
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

use serde::Serialize;

use super::util::trimmed_non_empty;
use crate::opensymphony_workflow::{
    IntegerLike, PROJECT_SET_SCHEMA_VERSION, ProjectEntry, ProjectSetAgentFrontMatter,
    ProjectSetBody, ProjectSetFrontMatter, ProjectSetLinearFrontMatter,
    ProjectSetPollingFrontMatter,
};

#[derive(Debug, Error)]
pub enum ProjectSetUpsertError {
    #[error("failed to read project-set config at {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse project-set config at {path}: {source}")]
    ParseFile {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error(
        "project-set inventory already has repo slug `{slug}` mapped to a different URL ({existing}); refusing to remap to {incoming}"
    )]
    ConflictingRepoUrl {
        slug: String,
        existing: String,
        incoming: String,
    },
    #[error(
        "project-set inventory already has repo URL `{url}` mapped to a different slug ({existing}); refusing to remap to {incoming}"
    )]
    ConflictingRepoSlug {
        url: String,
        existing: String,
        incoming: String,
    },
    #[error("failed to serialize project-set config: {0}")]
    Serialize(serde_yaml::Error),
    #[error("failed to write project-set config at {path}: {source}")]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create parent directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Inputs for upserting a single repo entry into the project-set inventory.
///
/// All fields are trimmed by [`upsert_project_set_yaml`] before being
/// applied; leading/trailing whitespace is never significant.
#[derive(Debug, Clone)]
pub struct ProjectSetUpsertPlan {
    /// Repo slug to register (e.g. `opensymphony`, `kumanday/OpenSymphony`).
    pub repo_slug: String,
    /// Clone URL for the repo (e.g. `https://github.com/org/repo.git`).
    pub repo_url: String,
    /// Default branch when it can be confidently determined. `None` is
    /// always safe: the resolver omits the field rather than writing a
    /// placeholder.
    pub default_branch: Option<String>,
    /// Slug for the `project_set:` block when creating a missing
    /// project-set file. When the file already exists, this is ignored.
    pub project_set_slug: String,
    /// Slug for the inner `project_set.projects[]` entry when creating a
    /// missing project-set file. When the file already exists, this is
    /// ignored.
    pub project_slug: String,
    /// `project_set.linear.project_slug` value used when the file is
    /// created from scratch (typically the same `project_slug` above).
    pub linear_project_slug: String,
    /// `project_set.linear.api_key_env` value used when the file is
    /// created from scratch. Defaults to `LINEAR_API_KEY` when omitted.
    pub linear_api_key_env: Option<String>,
    /// `polling.interval_ms` value used when the file is created from
    /// scratch. Defaults to [`DEFAULT_POLL_INTERVAL_MS`] when omitted.
    pub polling_interval_ms: Option<u64>,
    /// `agent.max_concurrent_agents` value used when the file is created
    /// from scratch. Defaults to [`DEFAULT_MAX_CONCURRENT_AGENTS`] when
    /// omitted.
    pub max_concurrent_agents: Option<u64>,
    /// `linear.active_states` value used when the file is created from
    /// scratch. Defaults to [`DEFAULT_ACTIVE_STATES`] when omitted.
    pub linear_active_states: Option<Vec<String>>,
    /// `linear.terminal_states` value used when the file is created from
    /// scratch. Defaults to [`DEFAULT_TERMINAL_STATES`] when omitted.
    pub linear_terminal_states: Option<Vec<String>>,
}

/// Outcome of an upsert. Useful for tests and operator-facing summaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectSetUpsertOutcome {
    /// The project-set file did not exist; a fresh one was written.
    Created,
    /// The project-set file already existed; the repo entry was inserted
    /// or updated without changing unrelated content.
    Updated,
    /// The project-set file already existed with an identical repo entry;
    /// no write was performed.
    NoChange,
}

/// Outcome of an upsert that also carries the absolute file path so callers
/// (e.g. `opensymphony init`) can show operators exactly where the
/// inventory lives (LOC-19 / D9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectSetAppliedOutcome {
    Created(PathBuf),
    Updated(PathBuf),
    Unchanged(PathBuf),
}

impl ProjectSetAppliedOutcome {
    /// Returns the absolute file path of the upserted project-set file.
    #[allow(dead_code)] // Reserved for callers that need to display/log the project-set path.
    pub fn path(&self) -> &Path {
        match self {
            Self::Created(path) | Self::Updated(path) | Self::Unchanged(path) => path,
        }
    }
}

/// Default polling interval (ms) used when bootstrapping a project-set
/// file from scratch.
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 30_000;

/// Default `max_concurrent_agents` used when bootstrapping a project-set
/// file from scratch.
pub const DEFAULT_MAX_CONCURRENT_AGENTS: u64 = 4;

/// Default Linear state names used when bootstrapping a project-set file
/// from scratch.
pub const DEFAULT_ACTIVE_STATES: &[&str] =
    &["Todo", "In Progress", "Human Review", "Merging", "Rework"];

pub const DEFAULT_TERMINAL_STATES: &[&str] =
    &["Done", "Closed", "Cancelled", "Canceled", "Duplicate"];

/// Computes the canonical project-set file path for `config_root`.
pub fn project_set_path(config_root: impl AsRef<Path>) -> PathBuf {
    config_root
        .as_ref()
        .join(".opensymphony")
        .join("project-set.yaml")
}

/// Reads the existing project-set file at `path`, returning `Ok(None)` when
/// the file does not exist. The result is the raw front matter; callers
/// must use [`upsert_project_set_yaml`] (or its sibling helpers) to apply
/// edits.
pub fn read_project_set_front_matter(
    path: impl AsRef<Path>,
) -> Result<Option<ProjectSetFrontMatter>, ProjectSetUpsertError> {
    let path = path.as_ref();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ProjectSetUpsertError::ReadFile {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let front_matter: ProjectSetFrontMatter =
        serde_yaml::from_str(&source).map_err(|source| ProjectSetUpsertError::ParseFile {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(Some(front_matter))
}

/// Result of applying a [`ProjectSetUpsertPlan`] against an already-parsed
/// [`ProjectSetFrontMatter`] in memory. Does not write to disk.
#[derive(Debug, Clone)]
pub struct ProjectSetUpsertDiff {
    pub front_matter: ProjectSetFrontMatter,
    pub outcome: ProjectSetUpsertOutcome,
}

/// Applies the plan against the supplied raw front matter (or a freshly
/// built default body when `existing` is `None`) and returns the resulting
/// raw front matter + a [`ProjectSetUpsertOutcome`] describing whether a
/// write would be required.
///
/// When `existing` is `Some`, the existing `project_set:` body is preserved
/// as much as possible: only the targeted inventory entry is inserted or
/// updated, and any conflict on slug/URL mappings surfaces as a
/// [`ProjectSetUpsertError`].
pub fn apply_upsert_plan(
    existing: Option<&ProjectSetFrontMatter>,
    plan: &ProjectSetUpsertPlan,
) -> Result<ProjectSetUpsertDiff, ProjectSetUpsertError> {
    let trimmed_repo_slug =
        trimmed_non_empty(Some(&plan.repo_slug)).ok_or_else(|| missing_field_error("repo_slug"))?;
    let trimmed_repo_url =
        trimmed_non_empty(Some(&plan.repo_url)).ok_or_else(|| missing_field_error("repo_url"))?;
    let trimmed_default_branch = trimmed_non_empty(plan.default_branch.as_deref());

    let Some(existing) = existing else {
        let front_matter = build_fresh_front_matter(plan)?;
        return Ok(ProjectSetUpsertDiff {
            front_matter,
            outcome: ProjectSetUpsertOutcome::Created,
        });
    };

    let mut body = existing.project_set.clone();
    let project_slug = trimmed_non_empty(Some(&plan.project_slug))
        .ok_or_else(|| missing_field_error("project_slug"))?;

    let project = ensure_project_entry(&mut body, &project_slug);
    let outcome = upsert_repo_entry(
        project,
        &trimmed_repo_slug,
        &trimmed_repo_url,
        trimmed_default_branch.as_deref(),
    )?;

    let front_matter = ProjectSetFrontMatter {
        schema_version: existing.schema_version,
        project_set: body,
    };
    Ok(ProjectSetUpsertDiff {
        front_matter,
        outcome,
    })
}

/// Convenience: reads, applies, writes, and returns the upsert outcome.
///
/// Use [`apply_upsert_plan`] when you need to inspect the diff before
/// committing it to disk.
pub fn upsert_project_set_yaml(
    path: impl AsRef<Path>,
    plan: &ProjectSetUpsertPlan,
) -> Result<ProjectSetUpsertOutcome, ProjectSetUpsertError> {
    let path = path.as_ref();
    let existing = read_project_set_front_matter(path)?;
    let diff = apply_upsert_plan(existing.as_ref(), plan)?;
    if matches!(diff.outcome, ProjectSetUpsertOutcome::NoChange) {
        return Ok(diff.outcome);
    }
    let serialized = serialize_front_matter(&diff.front_matter)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ProjectSetUpsertError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, serialized).map_err(|source| ProjectSetUpsertError::WriteFile {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(diff.outcome)
}

/// Convenience wrapper around [`upsert_project_set_yaml`] that returns the
/// absolute project-set file path alongside the outcome. Useful for
/// operator-facing summaries (LOC-19) and tests.
pub fn upsert_project_set_yaml_with_path(
    path: impl AsRef<Path>,
    plan: &ProjectSetUpsertPlan,
) -> Result<ProjectSetAppliedOutcome, ProjectSetUpsertError> {
    let path = path.as_ref().to_path_buf();
    let outcome = upsert_project_set_yaml(&path, plan)?;
    let applied = match outcome {
        ProjectSetUpsertOutcome::Created => ProjectSetAppliedOutcome::Created(path),
        ProjectSetUpsertOutcome::Updated => ProjectSetAppliedOutcome::Updated(path),
        ProjectSetUpsertOutcome::NoChange => ProjectSetAppliedOutcome::Unchanged(path),
    };
    Ok(applied)
}

/// Serializes a [`ProjectSetFrontMatter`] to a deterministic YAML document.
///
/// The output uses [`PROJECT_SET_SCHEMA_VERSION`] and omits default-valued
/// fields where the writer controls serialization, keeping the file diffable.
pub fn serialize_front_matter(
    front_matter: &ProjectSetFrontMatter,
) -> Result<String, ProjectSetUpsertError> {
    #[derive(Serialize)]
    struct Out<'a> {
        schema_version: u64,
        project_set: &'a ProjectSetBody,
    }
    let schema_version = front_matter
        .schema_version
        .unwrap_or(PROJECT_SET_SCHEMA_VERSION);
    let payload = Out {
        schema_version,
        project_set: &front_matter.project_set,
    };
    serde_yaml::to_string(&payload).map_err(ProjectSetUpsertError::Serialize)
}

fn build_fresh_front_matter(
    plan: &ProjectSetUpsertPlan,
) -> Result<ProjectSetFrontMatter, ProjectSetUpsertError> {
    let repo_slug =
        trimmed_non_empty(Some(&plan.repo_slug)).ok_or_else(|| missing_field_error("repo_slug"))?;
    let repo_url =
        trimmed_non_empty(Some(&plan.repo_url)).ok_or_else(|| missing_field_error("repo_url"))?;
    let project_set_slug = trimmed_non_empty(Some(&plan.project_set_slug))
        .ok_or_else(|| missing_field_error("project_set_slug"))?;
    let project_slug = trimmed_non_empty(Some(&plan.project_slug))
        .ok_or_else(|| missing_field_error("project_slug"))?;
    let linear_project_slug = trimmed_non_empty(Some(&plan.linear_project_slug))
        .ok_or_else(|| missing_field_error("linear_project_slug"))?;
    let api_key_env = trimmed_non_empty(plan.linear_api_key_env.as_deref())
        .unwrap_or_else(|| "LINEAR_API_KEY".to_string());
    let polling_interval_ms = plan.polling_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);
    let max_concurrent_agents = plan
        .max_concurrent_agents
        .unwrap_or(DEFAULT_MAX_CONCURRENT_AGENTS);
    let active_states = plan.linear_active_states.clone().unwrap_or_else(|| {
        DEFAULT_ACTIVE_STATES
            .iter()
            .map(|s| s.to_string())
            .collect()
    });
    let terminal_states = plan.linear_terminal_states.clone().unwrap_or_else(|| {
        DEFAULT_TERMINAL_STATES
            .iter()
            .map(|s| s.to_string())
            .collect()
    });

    let body = ProjectSetBody {
        slug: Some(project_set_slug.clone()),
        name: None,
        linear: ProjectSetLinearFrontMatter {
            endpoint: None,
            project_slug: Some(linear_project_slug),
            api_key_env: Some(api_key_env),
            active_states: Some(active_states),
            terminal_states: Some(terminal_states),
        },
        polling: ProjectSetPollingFrontMatter {
            interval_ms: Some(IntegerLike::Integer(polling_interval_ms as i64)),
        },
        agent: ProjectSetAgentFrontMatter {
            max_concurrent_agents: Some(IntegerLike::Integer(max_concurrent_agents as i64)),
        },
        projects: vec![ProjectEntry {
            slug: Some(project_slug.clone()),
            name: None,
            repos: vec![crate::opensymphony_workflow::RepoEntry {
                slug: Some(repo_slug),
                url: Some(repo_url),
                default_branch: trimmed_non_empty(plan.default_branch.as_deref()),
                path: None,
            }],
        }],
    };

    Ok(ProjectSetFrontMatter {
        schema_version: Some(PROJECT_SET_SCHEMA_VERSION),
        project_set: body,
    })
}

fn ensure_project_entry<'a>(
    body: &'a mut ProjectSetBody,
    project_slug: &str,
) -> &'a mut ProjectEntry {
    if let Some(index) = body.projects.iter().position(|project| {
        project
            .slug
            .as_deref()
            .map(str::trim)
            .map(|slug| slug == project_slug)
            .unwrap_or(false)
    }) {
        &mut body.projects[index]
    } else {
        body.projects.push(ProjectEntry {
            slug: Some(project_slug.to_owned()),
            name: None,
            repos: Vec::new(),
        });
        body.projects
            .last_mut()
            .expect("just pushed an entry above")
    }
}

fn upsert_repo_entry(
    project: &mut ProjectEntry,
    repo_slug: &str,
    repo_url: &str,
    default_branch: Option<&str>,
) -> Result<ProjectSetUpsertOutcome, ProjectSetUpsertError> {
    // Detect conflicts against any existing entry first.
    for entry in project.repos.iter() {
        let Some(existing_slug) = entry
            .slug
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(existing_url) = entry
            .url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if existing_slug == repo_slug && existing_url != repo_url {
            return Err(ProjectSetUpsertError::ConflictingRepoUrl {
                slug: repo_slug.to_string(),
                existing: existing_url.to_string(),
                incoming: repo_url.to_string(),
            });
        }
        if existing_url == repo_url && existing_slug != repo_slug {
            return Err(ProjectSetUpsertError::ConflictingRepoSlug {
                url: repo_url.to_string(),
                existing: existing_slug.to_string(),
                incoming: repo_slug.to_string(),
            });
        }
    }

    if let Some(index) = project.repos.iter().position(|entry| {
        entry
            .slug
            .as_deref()
            .map(str::trim)
            .map(|slug| slug == repo_slug)
            .unwrap_or(false)
    }) {
        let entry = &mut project.repos[index];
        let prior_url = entry.url.as_deref().map(str::trim).unwrap_or("");
        let prior_branch = entry.default_branch.as_deref().map(str::trim);
        let mut changed = false;
        if prior_url != repo_url {
            entry.url = Some(repo_url.to_string());
            changed = true;
        }
        let desired_branch = default_branch
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .map(|branch| branch.to_owned());
        match (prior_branch, desired_branch.as_deref()) {
            (None, Some(_)) => {
                entry.default_branch = desired_branch;
                changed = true;
            }
            (Some(prior), Some(desired)) if prior != desired => {
                entry.default_branch = desired_branch;
                changed = true;
            }
            _ => {}
        }
        if changed {
            Ok(ProjectSetUpsertOutcome::Updated)
        } else {
            Ok(ProjectSetUpsertOutcome::NoChange)
        }
    } else {
        project.repos.push(crate::opensymphony_workflow::RepoEntry {
            slug: Some(repo_slug.to_string()),
            url: Some(repo_url.to_string()),
            default_branch: default_branch
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .map(str::to_owned),
            path: None,
        });
        Ok(ProjectSetUpsertOutcome::Updated)
    }
}

fn missing_field_error(field: &str) -> ProjectSetUpsertError {
    // We do not have a dedicated `MissingField` variant; surface the failure
    // as a serialization error so the existing error enum stays narrow.
    let message = format!("project-set upsert plan missing required field `{field}`");
    ProjectSetUpsertError::Serialize(<serde_yaml::Error as serde::de::Error>::custom(message))
}
