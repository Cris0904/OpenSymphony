//! Domain types for implementation plan generation.
//!
//! These types represent the structured artifacts produced by the plan generator:
//! milestones, issues, sub-issues, task packages, and acceptance criteria.
//! They use Linear-native terminology (milestone, issue, sub-issue) to maintain
//! consistency with the task tracking system of record.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Unique identifier for a generated task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A criterion that must be satisfied for a task to be considered complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_command: Option<String>,
}

/// Priority level compatible with Linear's numeric priority system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    /// Urgent - immediate attention required
    Urgent = 1,
    /// High - should be addressed soon
    High = 2,
    /// Normal - standard priority
    #[default]
    Normal = 3,
    /// Low - can be deferred
    Low = 4,
}

impl TaskPriority {
    /// Returns the numeric Linear priority value used in task frontmatter.
    pub fn as_linear_priority(self) -> u8 {
        match self {
            Self::Urgent => 1,
            Self::High => 2,
            Self::Normal => 3,
            Self::Low => 4,
        }
    }
}

/// Shared routing metadata for publishable task shapes (LOC-25, H4).
///
/// The planning skill `.md` and this crate agree that:
///
/// * Leaf tasks (top-level issues without sub-issues and every sub-issue)
///   MUST carry exactly one `repo` slug that matches a project-set
///   inventory key character-for-character. The slug is **not**
///   lowercased, slugified, or otherwise coerced.
/// * Parent/review nodes (top-level issues with at least one sub-issue)
///   MUST NOT carry `repo` because their work is decomposed into the
///   leaves that own the repo routing.
///
/// The `TaskRoutingMetadata` type is the single place this contract lives
/// in the Rust crate so the H4 "dual source of truth" requirement is
/// maintained. Changing the rules here means changing the planning skill
/// `.md` in lockstep (and vice versa).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRoutingMetadata {
    /// Repository slug (must match a project-set inventory key exactly).
    /// `Some(slug)` on a leaf; `None` on a parent/review node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

impl TaskRoutingMetadata {
    /// Returns true when the routing metadata is consistent with a leaf
    /// task (i.e. it carries exactly one repo slug).
    pub fn is_valid_for_leaf(&self) -> bool {
        self.repo
            .as_ref()
            .map(|slug| !slug.is_empty())
            .unwrap_or(false)
    }

    /// Returns true when the routing metadata is consistent with a
    /// parent/review node (i.e. it carries no repo slug).
    pub fn is_valid_for_parent(&self) -> bool {
        self.repo.is_none()
    }

    /// Returns the trimmed repo slug when one is present, or `None` if no
    /// slug is set. Trims the underlying value without lowercasing or
    /// slugifying it; the result is still the exact inventory key (or
    /// empty, which downstream validators reject).
    pub fn trimmed_repo(&self) -> Option<&str> {
        self.repo
            .as_ref()
            .map(|slug| slug.trim())
            .filter(|slug| !slug.is_empty())
    }
}

/// A sub-issue represents a bounded implementation, validation, documentation,
/// or cleanup unit small enough for one agent run or one bounded sequence of runs.
///
/// Sub-issues are always leaves in the task graph (a parent task is the
/// top-level issue that owns them). Their `routing.repo` MUST be `Some`
/// when the wave requests repo seeding; the manifest validator enforces
/// the leaf contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedSubIssue {
    pub id: TaskId,
    pub title: String,
    pub summary: String,
    pub scope_in: Vec<String>,
    #[serde(default)]
    pub scope_out: Vec<String>,
    pub deliverables: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    #[serde(default)]
    pub verification_steps: Vec<String>,
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default)]
    pub definition_of_ready: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub priority: TaskPriority,
    #[serde(default)]
    pub estimate: Option<u8>,
    #[serde(default)]
    pub blocked_by: Vec<TaskId>,
    #[serde(default)]
    pub blocks: Vec<TaskId>,
    /// Routing metadata; sub-issues are leaves so they must carry a repo.
    #[serde(default)]
    pub routing: TaskRoutingMetadata,
    /// Relative path to the task file within the tasks directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_file: Option<String>,
}

/// An issue represents a demoable vertical capability or deliverable unit.
///
/// A `PlannedIssue` is a leaf (top-level task) when it has no
/// `sub_issues`; in that case `routing.repo` MUST be `Some`.
///
/// A `PlannedIssue` with at least one `sub_issue` is a parent/review
/// node; `routing.repo` MUST be `None` for those tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedIssue {
    pub id: TaskId,
    pub title: String,
    pub summary: String,
    pub scope_in: Vec<String>,
    #[serde(default)]
    pub scope_out: Vec<String>,
    pub deliverables: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    #[serde(default)]
    pub verification_steps: Vec<String>,
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default)]
    pub definition_of_ready: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub priority: TaskPriority,
    #[serde(default)]
    pub estimate: Option<u8>,
    #[serde(default)]
    pub blocked_by: Vec<TaskId>,
    #[serde(default)]
    pub blocks: Vec<TaskId>,
    pub sub_issues: Vec<PlannedSubIssue>,
    /// Routing metadata; see [`TaskRoutingMetadata`] for the contract.
    #[serde(default)]
    pub routing: TaskRoutingMetadata,
    /// Relative path to the task file within the tasks directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_file: Option<String>,
}

impl PlannedIssue {
    /// Returns true when the issue is a leaf in the task graph.
    ///
    /// A leaf has no sub-issues, which means the parent task itself is
    /// the work-unit that the agent executes; only leaves carry
    /// `routing.repo` per LOC-25.
    pub fn is_leaf(&self) -> bool {
        self.sub_issues.is_empty()
    }
}

/// A milestone represents a major delivery stage or checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedMilestone {
    pub id: TaskId,
    /// Exact Linear milestone name (e.g., "M9: Collaborative Planning Alpha")
    pub name: String,
    pub goal: String,
    pub issues: Vec<PlannedIssue>,
    #[serde(default)]
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    #[serde(default)]
    pub verification_steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// A single task entry in the task package manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestTask {
    pub id: TaskId,
    pub file: String,
}

/// The task package manifest is the canonical machine-readable input for
/// downstream Linear conversion. It uses exact Linear milestone names and
/// an explicit task file list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPackageManifest {
    /// Stable string identifier for this planning round.
    pub planning_wave: String,
    /// Directory containing task files (e.g., "docs/tasks").
    pub tasks_dir: String,
    /// Exact Linear milestone names.
    pub milestones: Vec<String>,
    /// Complete list of task file references.
    pub tasks: Vec<ManifestTask>,
}

/// Complete set of generated artifacts from a planning session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanArtifacts {
    pub generated_at: DateTime<Utc>,
    pub planning_wave: String,
    pub milestones: Vec<PlannedMilestone>,
    pub manifest: TaskPackageManifest,
    /// Human-readable milestone index (milestones.md equivalent).
    pub milestone_index: String,
    /// Map of task ID to task file content (for file generation).
    pub task_files: BTreeMap<TaskId, String>,
}

/// Scopes which artifacts should be regenerated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegenerationScope {
    /// Regenerate everything.
    Full,
    /// Regenerate only milestones.
    Milestones,
    /// Regenerate only issues (within specified milestones).
    Issues { milestone_ids: Option<Vec<TaskId>> },
    /// Regenerate only sub-issues (within specified issues).
    SubIssues { issue_ids: Option<Vec<TaskId>> },
    /// Regenerate only the task package manifest.
    Manifest,
    /// Regenerate only the human-readable milestone index.
    MilestoneIndex,
}

impl RegenerationScope {
    /// Returns true if milestones should be regenerated.
    pub fn includes_milestones(&self) -> bool {
        matches!(self, Self::Full | Self::Milestones)
    }

    /// Returns true if issues should be regenerated.
    pub fn includes_issues(&self) -> bool {
        matches!(self, Self::Full | Self::Milestones) || matches!(self, Self::Issues { .. })
    }

    /// Returns true if sub-issues should be regenerated.
    pub fn includes_sub_issues(&self) -> bool {
        matches!(self, Self::Full | Self::Milestones) || matches!(self, Self::SubIssues { .. })
    }

    /// Returns true if the manifest should be regenerated.
    pub fn includes_manifest(&self) -> bool {
        matches!(self, Self::Full | Self::Manifest)
            || self.includes_milestones()
            || self.includes_issues()
            || self.includes_sub_issues()
    }

    /// Returns true if the milestone index should be regenerated.
    pub fn includes_milestone_index(&self) -> bool {
        matches!(self, Self::Full | Self::MilestoneIndex)
            || self.includes_milestones()
            || self.includes_issues()
            || self.includes_sub_issues()
    }

    /// Returns true if task files should be regenerated.
    pub fn includes_task_files(&self) -> bool {
        matches!(self, Self::Full | Self::Milestones)
            || self.includes_issues()
            || self.includes_sub_issues()
    }
}
