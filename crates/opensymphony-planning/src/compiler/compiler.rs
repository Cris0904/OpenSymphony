//! Plan compiler body.
//!
//! `PlanCompiler` validates the [`PlanArtifacts`] produced by
//! `opensymphony_planning::generator::PlanGenerator` against Linear-native
//! taxonomy rules and emits the manifest and publish-receipt projections
//! that feed `convert-tasks-to-linear`.
//!
//! The compiler does not perform any Linear GraphQL calls. The Linear
//! entity ids, urls, and publish timestamps are intentionally left blank
//! so the publish stage can fill them.

use std::collections::BTreeMap;

use serde::Serialize;

use super::super::generator::domain::{
    PlanArtifacts, PlannedIssue, PlannedMilestone, PlannedSubIssue, TaskId,
    TaskPackageManifest as GeneratedManifest,
};
use super::domain::{
    AppliedHierarchy, CompilationResult, CompiledMilestone, DependencyEdge, DependencyMetadata,
    DependencyRelation, LinearPublishEntity, LinearPublishReceipt, MilestoneReceipt, TaskKind,
    TaxonomyViolation, UnderspecifiedSubIssue, ValidationMessage, issue_to_compiled,
};

/// Manifest yaml format emitted for `docs/tasks/task-package.yaml`. The
/// schema mirrors the YAML the downstream publish flow already consumes so
/// the compiled output can be persisted without further transformation.
#[derive(Debug, Serialize)]
pub struct CompiledManifestYaml<'a> {
    #[serde(rename = "planningWave")]
    pub planning_wave: &'a str,
    #[serde(rename = "tasksDir")]
    pub tasks_dir: &'a str,
    pub milestones: Vec<&'a str>,
    pub tasks: Vec<CompiledManifestTaskYaml<'a>>,
}

#[derive(Debug, Serialize)]
pub struct CompiledManifestTaskYaml<'a> {
    pub id: &'a str,
    pub file: &'a str,
}

/// The plan compiler turns a [`PlanArtifacts`] into a [`CompilationResult`].
/// The compiler is stateless; one instance can be reused for any number of
/// compilations as long as the caller supplies the artifact.
#[derive(Debug, Default, Clone)]
pub struct PlanCompiler;

impl PlanCompiler {
    pub fn new() -> Self {
        Self
    }

    /// Compile the supplied planning artifacts. The output is always
    /// returned, even when validation fails: callers can surface the
    /// diagnostics for review and choose to publish or roll back.
    pub fn compile(&self, artifacts: &PlanArtifacts) -> CompilationResult {
        let mut taxonomy_violations = Vec::new();
        let mut validation_messages = Vec::new();
        let mut underspecified_sub_issues = Vec::new();

        validate_taxonomy(
            &artifacts.milestones,
            &mut taxonomy_violations,
            &mut validation_messages,
        );

        let milestones = &artifacts.milestones;
        let manifest = &artifacts.manifest;
        let planning_wave = artifacts.planning_wave.as_str();
        let tasks_dir = artifacts.manifest.tasks_dir.as_str();

        let mut compiled_milestones: Vec<CompiledMilestone> =
            Vec::with_capacity(artifacts.milestones.len());
        let mut dependency_edges: Vec<DependencyEdge> = Vec::new();
        let mut sub_issue_count = 0usize;
        let mut issue_count_total = 0usize;

        for milestone in milestones {
            for issue in &milestone.issues {
                issue_count_total += 1;
                collect_issue_dependency_edges(issue, &milestone.name, &mut dependency_edges);
                validate_issue(issue, &milestone.name, &mut validation_messages);
                for sub_issue in &issue.sub_issues {
                    sub_issue_count += 1;
                    collect_sub_issue_dependency_edges(
                        sub_issue,
                        issue,
                        &milestone.name,
                        &mut dependency_edges,
                    );
                    validate_sub_issue(
                        sub_issue,
                        issue,
                        &milestone.name,
                        &mut validation_messages,
                        &mut underspecified_sub_issues,
                    );
                }
            }
            compiled_milestones.push(compile_milestone(milestone));
        }

        let manifest_yaml = render_manifest_yaml(planning_wave, tasks_dir, milestones);

        let applied_hierarchy = AppliedHierarchy {
            planning_wave: artifacts.planning_wave.clone(),
            milestones: compiled_milestones.clone(),
        };

        dependency_edges.sort_by(|a, b| {
            a.milestone
                .cmp(&b.milestone)
                .then_with(|| a.relation.cmp(&b.relation))
                .then_with(|| a.source.cmp(&b.source))
                .then_with(|| a.target.cmp(&b.target))
        });

        let milestone_count = artifacts.milestones.len();
        let dependency_metadata = DependencyMetadata {
            planning_wave: artifacts.planning_wave.clone(),
            total_nodes: milestone_count + issue_count_total + sub_issue_count,
            milestone_count,
            issue_count: issue_count_total,
            sub_issue_count,
            edges: dependency_edges,
        };

        sort_messages(&mut taxonomy_violations, &mut validation_messages);
        underspecified_sub_issues.sort_by(|a, b| a.sub_issue_id.cmp(&b.sub_issue_id));

        // Cross-check manifest milestones against compiled milestones.
        validate_manifest_consistency(manifest, &compiled_milestones, &mut validation_messages);

        let receipt_struct = build_publish_receipt(
            planning_wave,
            &compiled_milestones,
            tasks_dir,
            manifest,
            &applied_hierarchy,
        );
        let publish_receipt_yaml =
            serde_yaml::to_string(&receipt_struct).expect("publish receipt yaml should serialize");

        CompilationResult {
            planning_wave: artifacts.planning_wave.clone(),
            manifest_yaml,
            publish_receipt_yaml,
            applied_hierarchy,
            taxonomy_violations,
            validation_messages,
            underspecified_sub_issues,
            dependency_metadata,
        }
    }
}

fn compile_milestone(milestone: &PlannedMilestone) -> CompiledMilestone {
    CompiledMilestone {
        name: milestone.name.clone(),
        goal: milestone.goal.clone(),
        notes: milestone.notes.clone(),
        issues: milestone
            .issues
            .iter()
            .map(|i| issue_to_compiled(i, &milestone.name))
            .collect(),
    }
}

fn collect_issue_dependency_edges(
    issue: &PlannedIssue,
    milestone_name: &str,
    edges: &mut Vec<DependencyEdge>,
) {
    for blocker in &issue.blocked_by {
        edges.push(DependencyEdge {
            source: blocker.clone(),
            target: issue.id.clone(),
            milestone: milestone_name.to_string(),
            relation: DependencyRelation::Blocks,
        });
    }
    for blocked in &issue.blocks {
        edges.push(DependencyEdge {
            source: issue.id.clone(),
            target: blocked.clone(),
            milestone: milestone_name.to_string(),
            relation: DependencyRelation::Blocks,
        });
    }
}

fn collect_sub_issue_dependency_edges(
    sub_issue: &PlannedSubIssue,
    parent: &PlannedIssue,
    milestone_name: &str,
    edges: &mut Vec<DependencyEdge>,
) {
    edges.push(DependencyEdge {
        source: parent.id.clone(),
        target: sub_issue.id.clone(),
        milestone: milestone_name.to_string(),
        relation: DependencyRelation::ParentOf,
    });
    for blocker in &sub_issue.blocked_by {
        edges.push(DependencyEdge {
            source: blocker.clone(),
            target: sub_issue.id.clone(),
            milestone: milestone_name.to_string(),
            relation: DependencyRelation::Blocks,
        });
    }
    for blocked in &sub_issue.blocks {
        edges.push(DependencyEdge {
            source: sub_issue.id.clone(),
            target: blocked.clone(),
            milestone: milestone_name.to_string(),
            relation: DependencyRelation::Blocks,
        });
    }
}

fn validate_taxonomy(
    milestones: &[PlannedMilestone],
    taxonomy_violations: &mut Vec<TaxonomyViolation>,
    validation_messages: &mut Vec<ValidationMessage>,
) {
    if milestones.is_empty() {
        taxonomy_violations.push(TaxonomyViolation {
            task_id: None,
            task_kind: None,
            reason: "no milestones produced".to_string(),
            actionable: "Generator must produce at least one Linear milestone".to_string(),
        });
        validation_messages.push(ValidationMessage::error(
            None,
            "milestones",
            "Plan contains no milestones; expected at least one Linear milestone",
        ));
        return;
    }

    for milestone in milestones {
        if milestone.name.trim().is_empty() {
            taxonomy_violations.push(TaxonomyViolation {
                task_id: Some(milestone.id.clone()),
                task_kind: Some(TaskKind::Milestone),
                reason: "milestone has empty name".to_string(),
                actionable: format!(
                    "Provide a non-empty Linear milestone name for task {}",
                    milestone.id
                ),
            });
            validation_messages.push(ValidationMessage::error(
                Some(milestone.id.clone()),
                "name",
                "Linear milestone name is required",
            ));
        }
    }
}

fn validate_issue(
    issue: &PlannedIssue,
    _milestone_name: &str,
    validation_messages: &mut Vec<ValidationMessage>,
) {
    if issue.acceptance_criteria.is_empty() {
        validation_messages.push(ValidationMessage::error(
            Some(issue.id.clone()),
            "acceptanceCriteria",
            "Linear issue requires at least one acceptance criterion",
        ));
    }
    for (idx, criterion) in issue.acceptance_criteria.iter().enumerate() {
        if criterion.description.trim().is_empty() {
            validation_messages.push(ValidationMessage::error(
                Some(issue.id.clone()),
                "acceptanceCriteria",
                format!(
                    "Acceptance criterion {} on issue {} has empty description",
                    idx + 1,
                    issue.id
                ),
            ));
        }
    }
    if issue.title.trim().is_empty() {
        validation_messages.push(ValidationMessage::error(
            Some(issue.id.clone()),
            "title",
            "Linear issue requires a non-empty title",
        ));
    }
}

fn validate_sub_issue(
    sub_issue: &PlannedSubIssue,
    parent: &PlannedIssue,
    _milestone_name: &str,
    validation_messages: &mut Vec<ValidationMessage>,
    underspecified: &mut Vec<UnderspecifiedSubIssue>,
) {
    if sub_issue.verification_steps.is_empty() {
        validation_messages.push(ValidationMessage::error(
            Some(sub_issue.id.clone()),
            "verificationExpectations",
            format!(
                "Linear sub-issue {} requires at least one verification expectation",
                sub_issue.id
            ),
        ));
    }
    for (idx, step) in sub_issue.verification_steps.iter().enumerate() {
        if step.trim().is_empty() {
            validation_messages.push(ValidationMessage::error(
                Some(sub_issue.id.clone()),
                "verificationExpectations",
                format!(
                    "Verification step {} on sub-issue {} is empty",
                    idx + 1,
                    sub_issue.id
                ),
            ));
        }
    }
    if sub_issue.title.trim().is_empty() {
        validation_messages.push(ValidationMessage::error(
            Some(sub_issue.id.clone()),
            "title",
            "Linear sub-issue requires a non-empty title",
        ));
    }

    let reasons = super::domain::classify_underspecified_sub_issue(sub_issue);
    if !reasons.is_empty() {
        underspecified.push(UnderspecifiedSubIssue {
            sub_issue_id: sub_issue.id.clone(),
            parent_issue_id: parent.id.clone(),
            acceptance_criteria_count: sub_issue.acceptance_criteria.len(),
            verification_steps_count: sub_issue.verification_steps.len(),
            deliverables_count: sub_issue.deliverables.len(),
            scope_in_count: sub_issue.scope_in.len(),
            reasons,
        });
        validation_messages.push(ValidationMessage::warning(
            Some(sub_issue.id.clone()),
            "readiness",
            format!(
                "Sub-issue {} is underspecified: must add deliverables, scope, acceptance criteria, or verification expectations before publish",
                sub_issue.id
            ),
        ));
    }
}

fn validate_manifest_consistency(
    manifest: &GeneratedManifest,
    compiled_milestones: &[CompiledMilestone],
    validation_messages: &mut Vec<ValidationMessage>,
) {
    let compiled_milestone_names: std::collections::BTreeSet<&str> = compiled_milestones
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    for name in &manifest.milestones {
        if !compiled_milestone_names.contains(name.as_str()) {
            validation_messages.push(ValidationMessage::error(
                None,
                "milestones",
                format!(
                    "Manifest milestone '{}' is not present in compiled hierarchy",
                    name
                ),
            ));
        }
    }
    for milestone in compiled_milestones {
        if !manifest.milestones.contains(&milestone.name) {
            validation_messages.push(ValidationMessage::error(
                None,
                "milestones",
                format!(
                    "Compiled milestone '{}' is missing from manifest milestone list",
                    milestone.name
                ),
            ));
        }
    }
}

fn render_manifest_yaml(
    planning_wave: &str,
    tasks_dir: &str,
    milestones: &[PlannedMilestone],
) -> String {
    let milestone_refs: Vec<&str> = milestones.iter().map(|m| m.name.as_str()).collect();
    let mut tasks: Vec<CompiledManifestTaskYaml<'_>> = Vec::new();
    for milestone in milestones {
        for issue in &milestone.issues {
            if let Some(file) = issue.task_file.as_ref() {
                tasks.push(CompiledManifestTaskYaml {
                    id: issue.id.0.as_str(),
                    file: file.as_str(),
                });
            }
            for sub_issue in &issue.sub_issues {
                if let Some(file) = sub_issue.task_file.as_ref() {
                    tasks.push(CompiledManifestTaskYaml {
                        id: sub_issue.id.0.as_str(),
                        file: file.as_str(),
                    });
                }
            }
        }
    }
    let yaml_struct = CompiledManifestYaml {
        planning_wave,
        tasks_dir,
        milestones: milestone_refs,
        tasks,
    };
    serde_yaml::to_string(&yaml_struct).expect("manifest yaml should serialize")
}

fn build_publish_receipt(
    planning_wave: &str,
    compiled_milestones: &[CompiledMilestone],
    _tasks_dir: &str,
    manifest: &GeneratedManifest,
    _hierarchy: &AppliedHierarchy,
) -> LinearPublishReceipt {
    let mut milestones: BTreeMap<String, MilestoneReceipt> = BTreeMap::new();
    let mut tasks: BTreeMap<TaskId, LinearPublishEntity> = BTreeMap::new();
    let mut manifest_lookup: BTreeMap<&str, &str> = BTreeMap::new();
    for task in &manifest.tasks {
        manifest_lookup.insert(task.id.0.as_str(), task.file.as_str());
    }

    for milestone in compiled_milestones {
        let mut linked_issues: Vec<TaskId> = Vec::new();
        for issue in &milestone.issues {
            linked_issues.push(issue.task_id.clone());
            let file = if issue.source_file.is_empty() {
                manifest_lookup
                    .get(issue.task_id.0.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            } else {
                issue.source_file.clone()
            };
            let review_comments = Vec::new();
            tasks.insert(
                issue.task_id.clone(),
                LinearPublishEntity {
                    source_task_id: issue.task_id.clone(),
                    source_file: file,
                    linear_kind: TaskKind::Issue,
                    linear_milestone: milestone.name.clone(),
                    parent_task_id: None,
                    blocked_by: issue.blocked_by.clone(),
                    blocks: issue.blocks.clone(),
                    review_comments,
                    issue: None,
                    issue_id: None,
                    url: None,
                },
            );
            for sub in &issue.sub_issues {
                tasks.insert(
                    sub.task_id.clone(),
                    LinearPublishEntity {
                        source_task_id: sub.task_id.clone(),
                        source_file: sub.source_file.clone(),
                        linear_kind: TaskKind::SubIssue,
                        linear_milestone: milestone.name.clone(),
                        parent_task_id: Some(issue.task_id.clone()),
                        blocked_by: sub.blocked_by.clone(),
                        blocks: sub.blocks.clone(),
                        review_comments: Vec::new(),
                        issue: None,
                        issue_id: None,
                        url: None,
                    },
                );
            }
        }
        milestones.insert(
            milestone.name.clone(),
            MilestoneReceipt {
                name: milestone.name.clone(),
                milestone_id: None,
                linked_issues,
            },
        );
    }

    LinearPublishReceipt {
        planning_wave: planning_wave.to_string(),
        linear_project: None,
        published_at: None,
        milestones,
        tasks,
    }
}

// Review-comment extraction is intentionally absent today: the planning
// generator does not yet collect review comment lanes, so `LinearPublishEntity`
// stores an empty `review_comments: Vec<String>` at both issue and sub-issue
// insertion sites. A future change can add a `Vec<&ReviewComment>` pull from
// `PlanArtifacts` and feed it directly into the field without resurrecting
// this function.

fn sort_messages(taxonomy: &mut [TaxonomyViolation], messages: &mut [ValidationMessage]) {
    taxonomy.sort_by(|a, b| {
        let a_key = (
            a.task_kind,
            a.task_id.as_ref().map(|t| t.0.clone()).unwrap_or_default(),
        );
        let b_key = (
            b.task_kind,
            b.task_id.as_ref().map(|t| t.0.clone()).unwrap_or_default(),
        );
        a_key
            .0
            .cmp(&b_key.0)
            .then_with(|| a_key.1.cmp(&b_key.1))
            .then_with(|| a.reason.cmp(&b.reason))
    });
    messages.sort_by(|a, b| {
        let a_key = (
            a.severity,
            a.task_id.as_ref().map(|t| t.0.clone()).unwrap_or_default(),
            a.field.clone(),
        );
        let b_key = (
            b.severity,
            b.task_id.as_ref().map(|t| t.0.clone()).unwrap_or_default(),
            b.field.clone(),
        );
        a_key
            .0
            .cmp(&b_key.0)
            .then_with(|| a_key.1.cmp(&b_key.1))
            .then_with(|| a_key.2.cmp(&b_key.2))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_planning::generator::domain::{
        AcceptanceCriterion, PlanArtifacts, TaskPackageManifest as GeneratedManifest, TaskPriority,
    };
    use chrono::Utc;
    use std::collections::BTreeMap;

    fn sample_artifact(planning_wave: &str) -> PlanArtifacts {
        let issue_id = TaskId("OSYM-733".to_string());
        let sub_impl = TaskId("OSYM-733-IMPL".to_string());
        let sub_val = TaskId("OSYM-733-VAL".to_string());

        let issue = PlannedIssue {
            id: issue_id.clone(),
            title: "Milestone, issue, sub-issue compiler".to_string(),
            summary: "Compile planning artifacts into Linear hierarchy".to_string(),
            scope_in: vec!["Compile planner".to_string()],
            scope_out: vec!["Linear mutation".to_string()],
            deliverables: vec!["Plan compiler".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion {
                description: "Compiler emits manifest-driven task package".to_string(),
                verification_command: Some("cargo test".to_string()),
            }],
            verification_steps: vec!["cargo test -p opensymphony".to_string()],
            context: vec!["PRD 4.6.3".to_string()],
            definition_of_ready: vec!["Spec is referenced".to_string()],
            notes: None,
            priority: TaskPriority::Urgent,
            estimate: Some(5),
            blocked_by: vec![],
            blocks: vec![],
            sub_issues: vec![
                PlannedSubIssue {
                    id: sub_impl.clone(),
                    title: "Implement milestone/issue/sub-issue compiler".to_string(),
                    summary: "Implementation unit for compiler".to_string(),
                    scope_in: vec!["Compiler body".to_string()],
                    scope_out: vec!["Publish flow".to_string()],
                    deliverables: vec!["Compiler module".to_string()],
                    acceptance_criteria: vec![AcceptanceCriterion {
                        description: "Compiler module compiles".to_string(),
                        verification_command: Some("cargo check".to_string()),
                    }],
                    verification_steps: vec!["cargo test -p opensymphony compiler".to_string()],
                    context: vec!["PRD 4.6.3".to_string()],
                    definition_of_ready: vec!["Spec referenced".to_string()],
                    notes: None,
                    priority: TaskPriority::Urgent,
                    estimate: Some(3),
                    blocked_by: vec![],
                    blocks: vec![sub_val.clone()],
                    task_file: Some("docs/tasks/osym-733-impl.md".to_string()),
                },
                PlannedSubIssue {
                    id: sub_val.clone(),
                    title: "Validate compiler output".to_string(),
                    summary: "Validation sub-issue".to_string(),
                    scope_in: vec!["Tests".to_string()],
                    scope_out: Vec::new(),
                    deliverables: vec!["Validation tests".to_string()],
                    acceptance_criteria: vec![AcceptanceCriterion {
                        description: "Tests pass".to_string(),
                        verification_command: None,
                    }],
                    verification_steps: vec!["cargo test".to_string()],
                    context: vec!["PRD 4.6.3".to_string()],
                    definition_of_ready: vec!["Implementation done".to_string()],
                    notes: None,
                    priority: TaskPriority::Urgent,
                    estimate: Some(2),
                    blocked_by: vec![sub_impl.clone()],
                    blocks: vec![],
                    task_file: Some("docs/tasks/osym-733-val.md".to_string()),
                },
            ],
            task_file: Some(
                "docs/tasks/osym-733-milestone-issue-and-sub-issue-compiler.md".to_string(),
            ),
        };

        let mut tasks = Vec::new();
        tasks.push(
            crate::opensymphony_planning::generator::domain::ManifestTask {
                id: issue_id.clone(),
                file: "docs/tasks/osym-733-milestone-issue-and-sub-issue-compiler.md".to_string(),
            },
        );
        for sub in &issue.sub_issues {
            if let Some(file) = sub.task_file.as_ref() {
                tasks.push(
                    crate::opensymphony_planning::generator::domain::ManifestTask {
                        id: sub.id.clone(),
                        file: file.clone(),
                    },
                );
            }
        }

        let manifest = GeneratedManifest {
            planning_wave: planning_wave.to_string(),
            tasks_dir: "docs/tasks".to_string(),
            milestones: vec!["M9: Collaborative Planning Alpha".to_string()],
            tasks,
        };

        PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: planning_wave.to_string(),
            milestones: vec![PlannedMilestone {
                id: TaskId("OSYM-MS-9".to_string()),
                name: "M9: Collaborative Planning Alpha".to_string(),
                goal: "Deliver compiler layer".to_string(),
                issues: vec![issue],
                acceptance_criteria: vec![],
                verification_steps: vec![],
                notes: None,
            }],
            manifest,
            milestone_index: String::new(),
            task_files: BTreeMap::new(),
        }
    }

    #[test]
    fn compile_complete_plan_is_publishable() {
        let compiler = PlanCompiler::new();
        let result = compiler.compile(&sample_artifact("rich-client-hosted-mode"));

        assert!(
            result.is_publishable(),
            "violations: {:?}",
            result.taxonomy_violations
        );
        assert_eq!(result.taxonomy_violations, vec![]);
        assert_eq!(result.planning_wave, "rich-client-hosted-mode");
        assert!(
            result
                .manifest_yaml
                .contains("planningWave: rich-client-hosted-mode")
        );
        assert!(
            result
                .publish_receipt_yaml
                .contains("planningWave: rich-client-hosted-mode")
        );
    }

    #[test]
    fn compile_flags_missing_acceptance_criteria() {
        let mut artifact = sample_artifact("rich-client-hosted-mode");
        artifact.milestones[0].issues[0].acceptance_criteria.clear();

        let compiler = PlanCompiler::new();
        let result = compiler.compile(&artifact);

        assert!(!result.is_publishable());
        let hit = result
            .validation_messages
            .iter()
            .find(|m| m.field == "acceptanceCriteria");
        assert!(
            hit.is_some(),
            "expected missing acceptanceCriteria message, got: {:?}",
            result.validation_messages
        );
    }

    #[test]
    fn compile_flags_missing_sub_issue_verification_expectations() {
        let mut artifact = sample_artifact("rich-client-hosted-mode");
        for sub in artifact.milestones[0].issues[0].sub_issues.iter_mut() {
            sub.verification_steps.clear();
        }

        let compiler = PlanCompiler::new();
        let result = compiler.compile(&artifact);

        assert!(
            !result.is_publishable(),
            "missing verification must block publish"
        );
        let miss = result
            .validation_messages
            .iter()
            .find(|m| m.field == "verificationExpectations");
        assert!(miss.is_some());
    }

    #[test]
    fn compile_flags_underspecified_sub_issues() {
        let mut artifact = sample_artifact("rich-client-hosted-mode");
        let sub = &mut artifact.milestones[0].issues[0].sub_issues[0];
        sub.deliverables.clear();
        sub.scope_in.clear();

        let compiler = PlanCompiler::new();
        let result = compiler.compile(&artifact);

        assert!(
            result
                .underspecified_sub_issues
                .iter()
                .any(|u| u.sub_issue_id.0 == "OSYM-733-IMPL"),
            "expected OSYM-733-IMPL flagged as underspecified"
        );
    }

    #[test]
    fn compile_manifest_references_issue_and_sub_issue_only() {
        let compiler = PlanCompiler::new();
        let result = compiler.compile(&sample_artifact("rich-client-hosted-mode"));

        // Manifest contains exact milestone names and references each
        // issue + sub-issue task file, never milestone ids. `serde_yaml`
        // emits either single or double quotes for strings containing `:`,
        // so assert the substring flexibly.
        assert!(
            result
                .manifest_yaml
                .contains("M9: Collaborative Planning Alpha")
        );
        assert!(result.manifest_yaml.contains("- id: OSYM-733"));
        assert!(result.manifest_yaml.contains("- id: OSYM-733-IMPL"));
        assert!(result.manifest_yaml.contains("- id: OSYM-733-VAL"));
    }

    #[test]
    fn compile_dependency_metadata_records_parent_and_blocks_edges() {
        let compiler = PlanCompiler::new();
        let result = compiler.compile(&sample_artifact("rich-client-hosted-mode"));

        assert!(result.dependency_metadata.edges.iter().any(|e| matches!(
            e.relation,
            DependencyRelation::ParentOf
        ) && e.source.0 == "OSYM-733"
            && e.target.0 == "OSYM-733-IMPL"));
        assert!(result.dependency_metadata.edges.iter().any(|e| matches!(
            e.relation,
            DependencyRelation::Blocks
        ) && e.source.0
            == "OSYM-733-IMPL"
            && e.target.0 == "OSYM-733-VAL"));
    }

    #[test]
    fn compile_publish_receipt_carries_planning_wave_and_milestone_entries() {
        let compiler = PlanCompiler::new();
        let result = compiler.compile(&sample_artifact("rich-client-hosted-mode"));

        assert!(
            result
                .publish_receipt_yaml
                .contains("planningWave: rich-client-hosted-mode")
        );
        assert!(
            result
                .publish_receipt_yaml
                .contains("M9: Collaborative Planning Alpha")
        );
        assert!(result.publish_receipt_yaml.contains("OSYM-733"));
    }

    #[test]
    fn compile_handles_invalid_taxonomy_marker() {
        let mut artifact = sample_artifact("rich-client-hosted-mode");
        artifact.milestones[0].name = "  ".to_string();

        let compiler = PlanCompiler::new();
        let result = compiler.compile(&artifact);

        assert!(!result.taxonomy_violations.is_empty());
        let violation = result.taxonomy_violations.first().expect("violation");
        assert!(matches!(violation.task_kind, Some(TaskKind::Milestone)));
    }

    #[test]
    fn compile_emits_validation_message_for_missing_in_scope_sub_issue() {
        let mut artifact = sample_artifact("rich-client-hosted-mode");
        artifact.milestones[0].issues[0].sub_issues[0]
            .scope_in
            .clear();
        artifact.milestones[0].issues[0].sub_issues[0]
            .deliverables
            .clear();
        artifact.milestones[0].issues[0].sub_issues[0]
            .verification_steps
            .clear();
        artifact.milestones[0].issues[0].sub_issues[0]
            .acceptance_criteria
            .clear();

        let compiler = PlanCompiler::new();
        let result = compiler.compile(&artifact);

        let underspecified = result
            .underspecified_sub_issues
            .iter()
            .find(|u| u.sub_issue_id.0 == "OSYM-733-IMPL")
            .expect("underspecified record present");
        assert!(!underspecified.reasons.is_empty());
    }

    #[test]
    fn compile_dependency_metadata_totals_match_hierarchy() {
        let compiler = PlanCompiler::new();
        let result = compiler.compile(&sample_artifact("rich-client-hosted-mode"));

        assert_eq!(
            result.dependency_metadata.sub_issue_count,
            result.applied_hierarchy.milestones[0].issues[0]
                .sub_issues
                .len()
        );
        assert_eq!(result.dependency_metadata.issue_count, 1);
        assert_eq!(result.dependency_metadata.milestone_count, 1);
        assert_eq!(result.dependency_metadata.total_nodes, 1 + 1 + 2);
    }
}
