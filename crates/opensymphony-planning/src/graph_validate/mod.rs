//! Dependency-graph generator and plan-quality checks.
//!
//! This module owns the planning-session validation artefacts that the
//! downstream planning workspace UI (OSYM-735) and Linear draft preview
//! (OSYM-736) consume. It exposes three first-class types:
//!
//! - [`DependencyGraphBuilder`] in [`graph`] emits a deterministic graph
//!   artefact from in-memory [`crate::opensymphony_planning::generator::domain::PlanArtifacts`].
//! - [`PlanQualityChecker`] in [`checks`] runs cycle detection (delegating
//!   to the existing in-memory validator), missing-blocker detection,
//!   parallelizable-work grouping, and the full plan-check category matrix.
//! - [`ManifestValidator`] in [`manifest`] reads
//!   `docs/tasks/task-package.yaml` plus the declared task files and
//!   returns the same five error classes as the Python
//!   `convert-tasks-to-linear.py` validator.
//!
//! All three produce values of [`domain`], so the planning session API can
//! combine them into a single [`PlanValidationReport`] without further
//! translation.

pub mod checks;
pub mod domain;
pub mod frontmatter;
pub mod graph;
pub mod manifest;

pub use checks::{PlanQualityChecker, build_blocker_inverse, creation_order_waves};
pub use domain::{
    DependencyGraph, GraphEdge, GraphEdgeReason, GraphNode, GraphNodeKind,
    ManifestValidationResult, MissingTaskFile, PlanCheckCategory, PlanCheckFinding,
    PlanCheckSeverity, PlanValidationReport, SelfBlock, UnknownDependency, UnknownMilestone,
};
pub use frontmatter::{
    ParsedTaskFile, TaskFrontmatter, TaskFrontmatterError, parse_task_file, parse_task_text,
};
pub use graph::DependencyGraphBuilder;
pub use manifest::{
    ManifestTaskEntry, ManifestValidator, ManifestValidatorError, TaskPackageManifestFile,
    load_manifest,
};

use chrono::Utc;

use crate::opensymphony_planning::generator::domain::PlanArtifacts;

use super::codebase::CodebaseAnalysis;
use super::codebase::RiskSeverity;
use super::research::ResearchBrief;

/// Convenience helper that runs the graph builder and the plan-quality
/// checker together. The manifest validator produces an independent
/// report and is not invoked from this helper because it reads from
/// disk; callers typically run it in a separate planning-session step.
#[allow(dead_code)]
pub fn build_in_memory_report(
    artifacts: &PlanArtifacts,
    research: Option<&ResearchBrief>,
    codebase: Option<&CodebaseAnalysis>,
) -> PlanValidationReport {
    let dependency_graph = DependencyGraphBuilder::build(artifacts);
    let mut checker = PlanQualityChecker::new(artifacts);
    if let Some(brief) = research {
        checker = checker.with_research(brief.findings.len());
    }
    if let Some(analysis) = codebase {
        let risk_count: usize = analysis
            .risks
            .iter()
            .filter(|risk| matches!(risk.severity, RiskSeverity::High))
            .count();
        checker = checker.with_codebase(risk_count);
    }
    let plan_checks = checker.run();
    PlanValidationReport {
        planning_wave: artifacts.planning_wave.clone(),
        generated_at: Utc::now(),
        dependency_graph: Some(dependency_graph),
        plan_checks,
        manifest_validation: None,
    }
}

/// Convenience helper to attach a manifest-validation result onto an
/// existing in-memory report. The helper takes ownership of the supplied
/// manifest result so callers can move it into the report after the
/// on-disk validation step completes.
#[allow(dead_code)]
pub fn attach_manifest_validation(
    report: &mut PlanValidationReport,
    result: ManifestValidationResult,
) {
    report.manifest_validation = Some(result);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::opensymphony_planning::generator::generator::validate_dependency_graph;

    #[test]
    fn plan_validation_report_round_trips_through_json() {
        // A minimal-but-valid set of artefacts that the in-memory report
        // helper can consume. Confirms the planning-session API can
        // serialize and re-deserialize the report without losing fields.
        use crate::opensymphony_planning::generator::domain::{PlanArtifacts, TaskPackageManifest};
        let artifacts = PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "rich-client-hosted-mode".to_string(),
            milestones: vec![],
            manifest: TaskPackageManifest {
                planning_wave: "rich-client-hosted-mode".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec![],
                tasks: vec![],
            },
            milestone_index: String::new(),
            task_files: Default::default(),
        };
        validate_dependency_graph(&artifacts).expect("no cycles in empty artifacts");
        let report = build_in_memory_report(&artifacts, None, None);
        let json = serde_json::to_string(&report).expect("serializable");
        assert!(json.contains("rich-client-hosted-mode"));
        assert!(json.contains("dependency_graph"));
        let parsed: PlanValidationReport = serde_json::from_str(&json).expect("deserializable");
        assert_eq!(parsed.planning_wave, "rich-client-hosted-mode");
        assert!(parsed.dependency_graph.is_some());
    }
}
