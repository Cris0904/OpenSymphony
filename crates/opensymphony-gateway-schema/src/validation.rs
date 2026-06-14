use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{approval::ApprovalStatus, run::RunAction, version::SchemaVersion};

/// Validation summary for `/api/v1/runs/{run_id}/validation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunValidationSummary {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub generated_at: DateTime<Utc>,
    pub overall_status: ApprovalStatus,
    pub commands: Vec<ValidationCommand>,
    pub evidence: Vec<ValidationEvidenceItem>,
}

/// A validation command that ran against the run's workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationCommand {
    pub command_id: String,
    pub command: String,
    pub status: ApprovalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_summary: Option<String>,
}

/// A single piece of validation evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationEvidenceItem {
    pub evidence_id: String,
    pub label: String,
    pub status: ApprovalStatus,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_triggered: Option<RunAction>,
}
