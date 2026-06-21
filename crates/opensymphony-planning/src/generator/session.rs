//! Planning session context that holds all inputs for plan generation.
//!
//! A PlanningSession aggregates intake requirements, research findings,
//! codebase analysis, and Linear graph context into a single context
//! that the generator uses to produce structured artifacts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::super::codebase::CodebaseAnalysis;
use super::super::linear_graph::LinearGraphAnalysis;
use super::super::research::ResearchArtifactStore;

/// Intake captures the initial requirements, constraints, and goals
/// gathered from human-AI collaboration sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakeContext {
    /// Planning wave identifier (e.g., "rich-client-hosted-mode").
    pub planning_wave: String,
    /// Project description and mission.
    pub project_description: String,
    /// Success criteria defined by stakeholders.
    pub success_criteria: Vec<String>,
    /// Key requirements and feature needs.
    pub requirements: Vec<String>,
    /// Technical constraints and preferences.
    pub constraints: Vec<String>,
    /// Open questions that need research or clarification.
    pub open_questions: Vec<String>,
    /// Existing PRDs, architecture notes, or design documents.
    pub reference_docs: Vec<String>,
}

/// Complete context for a planning session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningSession {
    pub intake: IntakeContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codebase_analysis: Option<CodebaseAnalysis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_graph_analysis: Option<LinearGraphAnalysis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research: Option<ResearchArtifactStore>,
    /// Lightweight available-repo inventory (LOC-25). Maps the exact
    /// `project_set.projects[].repos[].slug` key to a clone-source URL
    /// (kept as metadata only; the resolver does not consume it).
    ///
    /// When this map contains exactly one entry and a leaf task's
    /// `routing.repo` is unset, the generator may auto-fill the obvious
    /// one-repo slug. When the map has zero or multiple entries, the
    /// generator leaves the routing decision to the planning skill / LLM
    /// and the manifest validator.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub available_repos: BTreeMap<String, String>,
    /// Directory where task files should be generated (e.g., "docs/tasks").
    pub tasks_dir: String,
}

impl PlanningSession {
    /// Creates a new planning session with the given intake context.
    pub fn new(intake: IntakeContext, tasks_dir: impl Into<String>) -> Self {
        Self {
            intake,
            codebase_analysis: None,
            linear_graph_analysis: None,
            research: None,
            available_repos: BTreeMap::new(),
            tasks_dir: tasks_dir.into(),
        }
    }

    /// Sets the codebase analysis for this session.
    pub fn with_codebase_analysis(mut self, analysis: CodebaseAnalysis) -> Self {
        self.codebase_analysis = Some(analysis);
        self
    }

    /// Sets the Linear graph analysis for this session.
    pub fn with_linear_graph_analysis(mut self, analysis: LinearGraphAnalysis) -> Self {
        self.linear_graph_analysis = Some(analysis);
        self
    }

    /// Sets the research artifact store for this session.
    pub fn with_research(mut self, research: ResearchArtifactStore) -> Self {
        self.research = Some(research);
        self
    }

    /// Sets the available-repo inventory for this session.
    ///
    /// The map keys MUST be the exact project-set repo slugs (no
    /// lowercasing or slugification). The values are clone-source URLs
    /// kept as metadata for diagnostics; the resolver does not consume
    /// them.
    pub fn with_available_repos(
        mut self,
        repos: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.available_repos = repos.into_iter().collect();
        self
    }

    /// Returns true if all optional analyses have been provided.
    ///
    /// Note: `available_repos` is intentionally not part of the
    /// "complete" gate. A planning session may legitimately run without
    /// a known inventory (e.g. when the project-set has not been
    /// onboarded yet); the manifest validator then operates in a
    /// "shape only" mode that rejects parent-with-repo and missing
    /// leaf repo but skips the inventory-membership check.
    pub fn is_complete(&self) -> bool {
        self.codebase_analysis.is_some()
            && self.linear_graph_analysis.is_some()
            && self.research.is_some()
    }

    /// Returns the unique slug of the only available repo when the
    /// inventory contains exactly one entry, or `None` otherwise.
    ///
    /// This is the "obvious one-repo case" the LOC-25 description calls
    /// out: when the project set has a single repo, the generator can
    /// auto-fill `routing.repo` for every leaf task without forcing the
    /// planner to repeat itself.
    pub fn single_repo_slug(&self) -> Option<&str> {
        if self.available_repos.len() == 1 {
            self.available_repos.keys().next().map(String::as_str)
        } else {
            None
        }
    }

    /// Returns a summary of available context for debugging/logging.
    pub fn context_summary(&self) -> String {
        let mut summary = format!("Planning wave: {}\n", self.intake.planning_wave);
        summary.push_str(&format!(
            "Requirements: {} items\n",
            self.intake.requirements.len()
        ));
        summary.push_str(&format!(
            "Constraints: {} items\n",
            self.intake.constraints.len()
        ));
        summary.push_str(&format!(
            "Codebase analysis: {}\n",
            if self.codebase_analysis.is_some() {
                "available"
            } else {
                "missing"
            }
        ));
        summary.push_str(&format!(
            "Linear graph analysis: {}\n",
            if self.linear_graph_analysis.is_some() {
                "available"
            } else {
                "missing"
            }
        ));
        summary.push_str(&format!(
            "Research artifacts: {}\n",
            self.research.as_ref().map(|r| r.len()).unwrap_or(0)
        ));
        summary.push_str(&format!(
            "Available repos: {} slug(s)\n",
            self.available_repos.len()
        ));
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample_intake() -> IntakeContext {
        IntakeContext {
            planning_wave: "test-wave".to_string(),
            project_description: "Test project for planning".to_string(),
            success_criteria: vec!["All tests pass".to_string()],
            requirements: vec!["Feature A".to_string(), "Feature B".to_string()],
            constraints: vec!["Must use Rust".to_string()],
            open_questions: vec!["How to handle auth?".to_string()],
            reference_docs: vec!["docs/architecture.md".to_string()],
        }
    }

    #[test]
    fn planning_session_starts_with_empty_analyses() {
        let session = PlanningSession::new(make_sample_intake(), "docs/tasks");
        assert!(session.codebase_analysis.is_none());
        assert!(session.linear_graph_analysis.is_none());
        assert!(session.research.is_none());
        assert!(!session.is_complete());
    }

    #[test]
    fn planning_session_can_be_completed_with_all_analyses() {
        let intake = make_sample_intake();
        let session = PlanningSession::new(intake.clone(), "docs/tasks")
            .with_codebase_analysis(CodebaseAnalysis {
                root_path: ".".to_string(),
                languages: vec![],
                packages: vec![],
                build_systems: vec![],
                ownership_files: vec![],
                integration_points: vec![],
                conventions: vec![],
                risks: vec![],
                total_files: 0,
                total_rust_files: 0,
                total_typescript_files: 0,
            })
            .with_linear_graph_analysis(LinearGraphAnalysis {
                project_name: "Test".to_string(),
                project_id: "test-1".to_string(),
                analyzed_at: chrono::Utc::now(),
                total_issues: 0,
                issues_by_state: Default::default(),
                issues_by_priority: Default::default(),
                milestones: vec![],
                blocker_chains: vec![],
                unblocked_issues: vec![],
                blocked_issues: vec![],
                terminal_issues: vec![],
                active_issues: vec![],
                label_distribution: Default::default(),
                parent_child_relationships: vec![],
                constraints_summary: "None".to_string(),
            })
            .with_research(ResearchArtifactStore::new());

        assert!(session.is_complete());
        assert!(session.context_summary().contains("test-wave"));
    }

    #[test]
    fn context_summary_includes_all_sections() {
        let session = PlanningSession::new(make_sample_intake(), "docs/tasks");
        let summary = session.context_summary();

        assert!(summary.contains("Planning wave"));
        assert!(summary.contains("Requirements"));
        assert!(summary.contains("Constraints"));
        assert!(summary.contains("Codebase analysis"));
        assert!(summary.contains("Linear graph analysis"));
        assert!(summary.contains("Research artifacts"));
    }
}
