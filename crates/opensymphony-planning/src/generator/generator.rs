//! Implementation plan generator that produces structured artifacts.
//!
//! This module takes a PlanningSession containing intake, research, codebase
//! analysis, and Linear graph context and produces:
//!
//! - Planned milestones with issues and sub-issues
//! - Task package manifest (docs/tasks/task-package.yaml equivalent)
//! - Human-readable milestone index
//! - Individual task file contents
//! - Acceptance criteria, verification steps, and dependencies

use std::collections::BTreeMap;

use chrono::Utc;

use super::domain::*;
use super::session::{IntakeContext, PlanningSession};

/// Error type for plan generation operations.
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    #[error("planning session is incomplete: missing {0}")]
    IncompleteSession(String),
    #[error("invalid task ID: {0}")]
    InvalidTaskId(String),
    #[error("circular dependency detected: {0}")]
    CircularDependency(String),
    #[error("task file content generation failed: {0}")]
    TaskFileGeneration(String),
}

/// Escapes a string for safe use in YAML frontmatter double-quoted values.
fn yaml_escape(s: &str) -> String {
    // Replace backslashes first, then quotes, then other special chars
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// The generator produces structured plan artifacts from a planning session.
pub struct PlanGenerator {
    session: PlanningSession,
    task_counter: usize,
}

impl PlanGenerator {
    /// Creates a new generator from a planning session.
    pub fn new(session: PlanningSession) -> Self {
        Self {
            session,
            task_counter: 0,
        }
    }

    /// Generates the complete set of plan artifacts.
    pub fn generate(&mut self) -> Result<PlanArtifacts, GenerationError> {
        self.validate_session()?;

        let milestones = self.generate_milestones();
        let manifest = self.generate_manifest(&milestones);
        let milestone_index = self.render_milestone_index(&milestones);
        let task_files = self.generate_task_files(&milestones);

        Ok(PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: self.session.intake.planning_wave.clone(),
            milestones,
            manifest,
            milestone_index,
            task_files,
        })
    }

    /// Regenerates only the artifacts specified in the scope, preserving others.
    pub fn regenerate(
        &mut self,
        existing: &PlanArtifacts,
        scope: &RegenerationScope,
    ) -> Result<PlanArtifacts, GenerationError> {
        self.validate_session()?;

        let milestones = if scope.includes_milestones() {
            self.generate_milestones()
        } else {
            existing.milestones.clone()
        };

        let manifest = if scope.includes_manifest() {
            self.generate_manifest(&milestones)
        } else {
            existing.manifest.clone()
        };

        let milestone_index = if scope.includes_milestone_index() {
            self.render_milestone_index(&milestones)
        } else {
            existing.milestone_index.clone()
        };

        let task_files = if scope.includes_task_files() {
            self.generate_task_files(&milestones)
        } else {
            existing.task_files.clone()
        };

        Ok(PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: self.session.intake.planning_wave.clone(),
            milestones,
            manifest,
            milestone_index,
            task_files,
        })
    }

    fn validate_session(&self) -> Result<(), GenerationError> {
        if self.session.intake.planning_wave.is_empty() {
            return Err(GenerationError::IncompleteSession(
                "planning_wave".to_string(),
            ));
        }
        if self.session.intake.requirements.is_empty() {
            return Err(GenerationError::IncompleteSession(
                "requirements".to_string(),
            ));
        }
        Ok(())
    }

    fn next_task_id(&mut self) -> TaskId {
        self.task_counter += 1;
        TaskId(format!("TASK-{:03}", self.task_counter))
    }

    fn generate_milestones(&mut self) -> Vec<PlannedMilestone> {
        let planning_wave = self.session.intake.planning_wave.clone();
        let project_description = self.session.intake.project_description.clone();
        let success_criteria = self.session.intake.success_criteria.clone();
        let requirements = self.session.intake.requirements.clone();
        let open_questions = self.session.intake.open_questions.clone();
        let reference_docs = self.session.intake.reference_docs.clone();
        let constraints = self.session.intake.constraints.clone();

        let intake = IntakeContext {
            planning_wave,
            project_description,
            success_criteria,
            requirements: requirements.clone(),
            constraints,
            open_questions,
            reference_docs,
        };

        // Extract milestone structure from Linear analysis if available
        let linear_milestones = self
            .session
            .linear_graph_analysis
            .as_ref()
            .map(|a| a.milestones.clone())
            .unwrap_or_default();

        let mut milestones = Vec::new();

        if linear_milestones.is_empty() {
            // Create a single milestone from intake requirements
            let milestone_id = self.next_task_id();
            let milestone_name = format!(
                "M1: {}",
                intake
                    .project_description
                    .split_whitespace()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ")
            );

            let issues = self.generate_issues_for_milestone(&milestone_id, &intake);

            milestones.push(PlannedMilestone {
                id: milestone_id,
                name: milestone_name,
                goal: intake.project_description.clone(),
                issues,
                acceptance_criteria: intake
                    .success_criteria
                    .iter()
                    .map(|c| AcceptanceCriterion {
                        description: c.clone(),
                        verification_command: None,
                    })
                    .collect(),
                verification_steps: Vec::new(),
                notes: None,
            });
        } else {
            // Distribute requirements across Linear milestones using round-robin
            // to ensure all requirements are assigned without dropping any
            let mut milestone_requirements: Vec<Vec<&String>> =
                vec![Vec::new(); linear_milestones.len()];

            for (req_idx, req) in requirements.iter().enumerate() {
                milestone_requirements[req_idx % linear_milestones.len()].push(req);
            }

            for (ms_idx, ms) in linear_milestones.iter().enumerate() {
                let milestone_id = self.next_task_id();

                // Skip milestones with no assigned requirements
                if milestone_requirements[ms_idx].is_empty() {
                    continue;
                }

                let mut milestone_intake = intake.clone();
                milestone_intake.requirements = milestone_requirements[ms_idx]
                    .iter()
                    .map(|r| (**r).clone())
                    .collect();

                let issues = self.generate_issues_for_milestone(&milestone_id, &milestone_intake);

                milestones.push(PlannedMilestone {
                    id: milestone_id,
                    name: ms.milestone_name.clone(),
                    goal: format!("Deliver {} capabilities", ms.milestone_name),
                    issues,
                    acceptance_criteria: Vec::new(),
                    verification_steps: Vec::new(),
                    notes: None,
                });
            }
        }

        milestones
    }

    fn generate_issues_for_milestone(
        &mut self,
        _milestone_id: &TaskId,
        intake: &IntakeContext,
    ) -> Vec<PlannedIssue> {
        let mut issues: Vec<PlannedIssue> = Vec::new();

        // Generate one issue per requirement as a starting point
        for (idx, requirement) in intake.requirements.iter().enumerate() {
            let issue_id = self.next_task_id();

            // Each issue gets sub-issues for implementation
            let sub_issues = self.generate_sub_issues_for_issue(&issue_id, requirement, intake);

            let blocked_by: Vec<TaskId> = if idx > 0 {
                issues
                    .last()
                    .map(|i| vec![i.id.clone()])
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            // Populate blocks symmetrically: if this issue is blocked by the previous,
            // the previous issue blocks this one
            if !blocked_by.is_empty()
                && let Some(prev_issue) = issues.last_mut()
            {
                prev_issue.blocks.push(issue_id.clone());
            }

            issues.push(PlannedIssue {
                id: issue_id.clone(),
                title: requirement.clone(),
                summary: format!(
                    "Implement {} as a vertical deliverable for the {} planning wave.",
                    requirement, intake.planning_wave
                ),
                scope_in: vec![requirement.clone()],
                scope_out: Vec::new(),
                deliverables: vec![format!("Working {} implementation", requirement)],
                acceptance_criteria: vec![AcceptanceCriterion {
                    description: format!("{} meets acceptance standards", requirement),
                    verification_command: None,
                }],
                verification_steps: vec![format!("Test {} functionality", requirement)],
                context: vec![
                    format!("Planning wave: {}", intake.planning_wave),
                    format!("Requirement {} of {}", idx + 1, intake.requirements.len()),
                ],
                definition_of_ready: vec![
                    "Hidden assumptions from prior discussion are written down.".to_string(),
                    "Required files, docs, and dependencies are explicitly referenced.".to_string(),
                    "A coding agent could begin execution without additional planning context."
                        .to_string(),
                ],
                notes: None,
                priority: TaskPriority::default(),
                estimate: None,
                blocked_by,
                blocks: Vec::new(),
                sub_issues,
                task_file: Some(format!("{}/{}.md", self.session.tasks_dir, issue_id)),
            });
        }

        issues
    }

    fn generate_sub_issues_for_issue(
        &mut self,
        _issue_id: &TaskId,
        requirement: &str,
        _intake: &IntakeContext,
    ) -> Vec<PlannedSubIssue> {
        let mut sub_issues = Vec::new();

        // Generate implementation sub-issue
        let impl_id = self.next_task_id();

        // Generate validation sub-issue (needs impl_id for blocked_by)
        let val_id = self.next_task_id();

        // Implementation sub-issue blocks the validation sub-issue
        sub_issues.push(PlannedSubIssue {
            id: impl_id.clone(),
            title: format!("Implement {}", requirement),
            summary: format!("Implementation unit for {}", requirement),
            scope_in: vec![format!("Core implementation of {}", requirement)],
            scope_out: vec![format!("Testing and validation of {}", requirement)],
            deliverables: vec!["Implementation code".to_string(), "Unit tests".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion {
                description: format!(
                    "Implementation of {} compiles and passes tests",
                    requirement
                ),
                verification_command: Some("cargo test".to_string()),
            }],
            verification_steps: vec![
                "Run unit tests".to_string(),
                "Verify code style".to_string(),
            ],
            context: vec![format!("Sub-issue of {}", requirement)],
            definition_of_ready: vec![
                "Requirements are clear and understood.".to_string(),
                "Dependencies are available.".to_string(),
            ],
            notes: None,
            priority: TaskPriority::default(),
            estimate: Some(3),
            blocked_by: Vec::new(),
            blocks: vec![val_id.clone()],
            task_file: Some(format!("{}/{}.md", self.session.tasks_dir, impl_id)),
        });

        // Validation sub-issue is blocked by the implementation sub-issue
        sub_issues.push(PlannedSubIssue {
            id: val_id.clone(),
            title: format!("Validate {}", requirement),
            summary: format!("Validation and testing for {}", requirement),
            scope_in: vec![
                "Integration testing".to_string(),
                "Acceptance criteria verification".to_string(),
            ],
            scope_out: vec!["Implementation changes".to_string()],
            deliverables: vec!["Test report".to_string(), "Validation evidence".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion {
                description: format!("All acceptance criteria for {} are met", requirement),
                verification_command: Some("cargo test --all".to_string()),
            }],
            verification_steps: vec![
                "Run integration tests".to_string(),
                "Verify acceptance criteria".to_string(),
                "Generate validation report".to_string(),
            ],
            context: vec![format!("Validates implementation of {}", requirement)],
            definition_of_ready: vec![
                "Implementation is complete.".to_string(),
                "Test environment is configured.".to_string(),
            ],
            notes: None,
            priority: TaskPriority::default(),
            estimate: Some(2),
            blocked_by: vec![impl_id],
            blocks: Vec::new(),
            task_file: Some(format!("{}/{}.md", self.session.tasks_dir, val_id)),
        });

        sub_issues
    }

    fn generate_manifest(&self, milestones: &[PlannedMilestone]) -> TaskPackageManifest {
        let mut tasks = Vec::new();
        let mut milestone_names = Vec::new();

        for milestone in milestones {
            milestone_names.push(milestone.name.clone());

            for issue in &milestone.issues {
                if let Some(ref task_file) = issue.task_file {
                    tasks.push(ManifestTask {
                        id: issue.id.clone(),
                        file: task_file.clone(),
                    });
                }

                for sub_issue in &issue.sub_issues {
                    if let Some(ref task_file) = sub_issue.task_file {
                        tasks.push(ManifestTask {
                            id: sub_issue.id.clone(),
                            file: task_file.clone(),
                        });
                    }
                }
            }
        }

        TaskPackageManifest {
            planning_wave: self.session.intake.planning_wave.clone(),
            tasks_dir: self.session.tasks_dir.clone(),
            milestones: milestone_names,
            tasks,
        }
    }

    fn render_milestone_index(&self, milestones: &[PlannedMilestone]) -> String {
        let mut md = String::from("# Project Milestones\n\n");

        for milestone in milestones {
            md.push_str(&format!("## {}\n\n", milestone.name));
            md.push_str(&format!("Goal: {}\n\n", milestone.goal));

            if !milestone.issues.is_empty() {
                md.push_str("Tasks:\n\n");
                for issue in &milestone.issues {
                    md.push_str(&format!("- {} {}\n", issue.id, issue.title));
                    for sub_issue in &issue.sub_issues {
                        md.push_str(&format!("  - {} {}\n", sub_issue.id, sub_issue.title));
                    }
                }
            }
            md.push('\n');
        }

        md
    }

    fn generate_task_files(&self, milestones: &[PlannedMilestone]) -> BTreeMap<TaskId, String> {
        let mut task_files = BTreeMap::new();

        for milestone in milestones {
            for issue in &milestone.issues {
                let content = self.render_issue_task_file(issue, milestone);
                task_files.insert(issue.id.clone(), content);

                for sub_issue in &issue.sub_issues {
                    let content = self.render_sub_issue_task_file(sub_issue, issue, milestone);
                    task_files.insert(sub_issue.id.clone(), content);
                }
            }
        }

        task_files
    }

    fn render_issue_task_file(&self, issue: &PlannedIssue, milestone: &PlannedMilestone) -> String {
        let mut content = format!(
            r#"---
id: {}
title: "{}"
milestone: "{}"
priority: {}
estimate: {}
blockedBy: [{}]
blocks: [{}]
parent: null
---

## Summary

{}

## Scope

### In scope

{}

### Out of scope

{}

## Deliverables

{}

## Acceptance Criteria

{}

## Test Plan

{}

## Context

{}

## Definition of Ready

{}

## Notes

{}
"#,
            issue.id,
            yaml_escape(&issue.title),
            yaml_escape(&milestone.name),
            issue.priority as u8,
            issue
                .estimate
                .map(|e| e.to_string())
                .unwrap_or_else(|| "null".to_string()),
            issue
                .blocked_by
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            issue
                .blocks
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            issue.summary,
            issue
                .scope_in
                .iter()
                .map(|s| format!("- {}", s))
                .collect::<Vec<_>>()
                .join("\n"),
            if issue.scope_out.is_empty() {
                "- None".to_string()
            } else {
                issue
                    .scope_out
                    .iter()
                    .map(|s| format!("- {}", s))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            issue
                .deliverables
                .iter()
                .map(|d| format!("- {}", d))
                .collect::<Vec<_>>()
                .join("\n"),
            issue
                .acceptance_criteria
                .iter()
                .map(|c| format!("- [ ] {}", c.description))
                .collect::<Vec<_>>()
                .join("\n"),
            issue
                .verification_steps
                .iter()
                .map(|v| format!("- {}", v))
                .collect::<Vec<_>>()
                .join("\n"),
            issue
                .context
                .iter()
                .map(|c| format!("- {}", c))
                .collect::<Vec<_>>()
                .join("\n"),
            issue
                .definition_of_ready
                .iter()
                .map(|d| format!("- [ ] {}", d))
                .collect::<Vec<_>>()
                .join("\n"),
            issue.notes.as_deref().unwrap_or("None"),
        );

        // Include sub-issues as part of the issue content
        if !issue.sub_issues.is_empty() {
            content.push_str("\n## Sub-issues\n\n");
            for sub_issue in &issue.sub_issues {
                content.push_str(&format!("- {} {}\n", sub_issue.id, sub_issue.title));
            }
        }

        content
    }

    fn render_sub_issue_task_file(
        &self,
        sub_issue: &PlannedSubIssue,
        parent_issue: &PlannedIssue,
        milestone: &PlannedMilestone,
    ) -> String {
        let content = format!(
            r#"---
id: {}
title: "{}"
milestone: "{}"
priority: {}
estimate: {}
blockedBy: [{}]
blocks: [{}]
parent: {}
---

## Summary

{}

## Scope

### In scope

{}

### Out of scope

{}

## Deliverables

{}

## Acceptance Criteria

{}

## Test Plan

{}

## Context

{}

## Definition of Ready

{}

## Notes

{}
"#,
            sub_issue.id,
            yaml_escape(&sub_issue.title),
            yaml_escape(&milestone.name),
            sub_issue.priority as u8,
            sub_issue
                .estimate
                .map(|e| e.to_string())
                .unwrap_or_else(|| "null".to_string()),
            sub_issue
                .blocked_by
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            sub_issue
                .blocks
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            parent_issue.id,
            sub_issue.summary,
            sub_issue
                .scope_in
                .iter()
                .map(|s| format!("- {}", s))
                .collect::<Vec<_>>()
                .join("\n"),
            if sub_issue.scope_out.is_empty() {
                "- None".to_string()
            } else {
                sub_issue
                    .scope_out
                    .iter()
                    .map(|s| format!("- {}", s))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            sub_issue
                .deliverables
                .iter()
                .map(|d| format!("- {}", d))
                .collect::<Vec<_>>()
                .join("\n"),
            sub_issue
                .acceptance_criteria
                .iter()
                .map(|c| format!("- [ ] {}", c.description))
                .collect::<Vec<_>>()
                .join("\n"),
            sub_issue
                .verification_steps
                .iter()
                .map(|v| format!("- {}", v))
                .collect::<Vec<_>>()
                .join("\n"),
            sub_issue
                .context
                .iter()
                .map(|c| format!("- {}", c))
                .collect::<Vec<_>>()
                .join("\n"),
            sub_issue
                .definition_of_ready
                .iter()
                .map(|d| format!("- [ ] {}", d))
                .collect::<Vec<_>>()
                .join("\n"),
            sub_issue.notes.as_deref().unwrap_or("None"),
        );

        content
    }
}

/// Validates that a dependency graph has no cycles.
pub fn validate_dependency_graph(artifacts: &PlanArtifacts) -> Result<(), GenerationError> {
    // Build adjacency map once for O(1) lookups instead of O(N) linear scans
    let dep_map = build_dependency_map(artifacts);
    let mut visited = BTreeMap::new();

    for milestone in &artifacts.milestones {
        for issue in &milestone.issues {
            validate_task_dependencies_with_map(&issue.id, &dep_map, &mut visited)?;

            for sub_issue in &issue.sub_issues {
                validate_task_dependencies_with_map(&sub_issue.id, &dep_map, &mut visited)?;
            }
        }
    }

    Ok(())
}

/// Builds a lookup map from task ID to its blocked_by dependencies.
/// Note: blocks field is the inverse of blocked_by and represents the same
/// dependency relationship from the other direction. We only need one direction
/// for cycle detection - blocked_by is the canonical source.
fn build_dependency_map(artifacts: &PlanArtifacts) -> BTreeMap<TaskId, Vec<TaskId>> {
    let mut map = BTreeMap::new();
    for milestone in &artifacts.milestones {
        for issue in &milestone.issues {
            map.insert(issue.id.clone(), issue.blocked_by.clone());
            for sub_issue in &issue.sub_issues {
                map.insert(sub_issue.id.clone(), sub_issue.blocked_by.clone());
            }
        }
    }
    map
}

fn validate_task_dependencies_with_map(
    task_id: &TaskId,
    dep_map: &BTreeMap<TaskId, Vec<TaskId>>,
    visited: &mut BTreeMap<TaskId, bool>,
) -> Result<(), GenerationError> {
    if let Some(&in_progress) = visited.get(task_id) {
        if in_progress {
            return Err(GenerationError::CircularDependency(format!(
                "Cycle detected involving task {}",
                task_id
            )));
        }
        return Ok(());
    }

    visited.insert(task_id.clone(), true);

    // O(1) lookup instead of linear scan
    if let Some(deps) = dep_map.get(task_id) {
        for dep in deps {
            validate_task_dependencies_with_map(dep, dep_map, visited)?;
        }
    }

    visited.insert(task_id.clone(), false);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample_session() -> PlanningSession {
        PlanningSession::new(
            IntakeContext {
                planning_wave: "test-wave".to_string(),
                project_description: "Test project for unit testing".to_string(),
                success_criteria: vec!["All tests pass".to_string()],
                requirements: vec!["Feature A".to_string(), "Feature B".to_string()],
                constraints: vec!["Must use Rust".to_string()],
                open_questions: vec![],
                reference_docs: vec![],
            },
            "docs/tasks",
        )
    }

    #[test]
    fn generator_produces_milestones_with_issues_and_subissues() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert!(!artifacts.milestones.is_empty());

        // Each requirement should produce at least one issue
        let total_issues: usize = artifacts.milestones.iter().map(|m| m.issues.len()).sum();
        assert!(total_issues > 0);

        // Each issue should have sub-issues
        for milestone in &artifacts.milestones {
            for issue in &milestone.issues {
                assert!(!issue.sub_issues.is_empty());
            }
        }
    }

    #[test]
    fn generator_produces_valid_manifest() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert_eq!(artifacts.manifest.planning_wave, "test-wave");
        assert_eq!(artifacts.manifest.tasks_dir, "docs/tasks");
        assert!(!artifacts.manifest.milestones.is_empty());
        assert!(!artifacts.manifest.tasks.is_empty());

        // Each milestone in the manifest should have a matching entry
        for milestone_name in &artifacts.manifest.milestones {
            assert!(
                artifacts
                    .milestones
                    .iter()
                    .any(|m| &m.name == milestone_name),
                "Milestone {} not found in artifacts",
                milestone_name
            );
        }
    }

    #[test]
    fn generator_produces_milestone_index() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert!(artifacts.milestone_index.contains("# Project Milestones"));

        for milestone in &artifacts.milestones {
            assert!(artifacts.milestone_index.contains(&milestone.name));
        }
    }

    #[test]
    fn generator_produces_task_files() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert!(!artifacts.task_files.is_empty());

        // Each issue and sub-issue should have a task file
        for milestone in &artifacts.milestones {
            for issue in &milestone.issues {
                assert!(artifacts.task_files.contains_key(&issue.id));
                for sub_issue in &issue.sub_issues {
                    assert!(artifacts.task_files.contains_key(&sub_issue.id));
                }
            }
        }
    }

    #[test]
    fn generator_fails_without_requirements() {
        let mut session = make_sample_session();
        session.intake.requirements.clear();
        let mut generator = PlanGenerator::new(session);
        let result = generator.generate();

        assert!(result.is_err());
        match result.expect_err("expected error should be returned") {
            GenerationError::IncompleteSession(field) => {
                assert_eq!(field, "requirements");
            }
            other => panic!("expected IncompleteSession error, got {:?}", other),
        }
    }

    #[test]
    fn regeneration_preserves_unselected_artifacts() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let original = generator.generate().expect("generation should succeed");

        // Regenerate only the manifest
        let regenerated = generator
            .regenerate(&original, &RegenerationScope::Manifest)
            .expect("regeneration should succeed");

        // Milestones should be preserved
        assert_eq!(original.milestones.len(), regenerated.milestones.len());

        // Milestone index should be preserved
        assert_eq!(original.milestone_index, regenerated.milestone_index);

        // Task files should be preserved
        assert_eq!(original.task_files.len(), regenerated.task_files.len());
    }

    #[test]
    fn dependency_graph_validation_passes_for_valid_graph() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert!(validate_dependency_graph(&artifacts).is_ok());
    }

    #[test]
    fn task_ids_are_unique() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        use std::collections::BTreeSet;
        let mut all_ids = BTreeSet::new();

        // Count total expected unique IDs
        let mut total_expected = 0;
        for milestone in &artifacts.milestones {
            total_expected += 1;
            assert!(
                all_ids.insert(milestone.id.0.clone()),
                "duplicate milestone id: {}",
                milestone.id
            );
            for issue in &milestone.issues {
                total_expected += 1;
                assert!(
                    all_ids.insert(issue.id.0.clone()),
                    "duplicate issue id: {}",
                    issue.id
                );
                for sub_issue in &issue.sub_issues {
                    total_expected += 1;
                    assert!(
                        all_ids.insert(sub_issue.id.0.clone()),
                        "duplicate sub-issue id: {}",
                        sub_issue.id
                    );
                }
            }
        }

        assert_eq!(all_ids.len(), total_expected, "all ids should be unique");

        // Each manifest task should reference a known id
        for task in &artifacts.manifest.tasks {
            assert!(
                all_ids.contains(&task.id.0),
                "Task ID {} not found in milestone/issue/sub-issue structure",
                task.id.0
            );
        }
    }

    #[test]
    fn dependency_graph_validation_detects_cycle() {
        // Build artifacts with a cycle: A → B → C → A
        // All three tasks must exist as issues/sub-issues for the graph traversal to find the cycle.
        let cycle_a = TaskId("TASK-001".to_string());
        let cycle_b = TaskId("TASK-002".to_string());
        let cycle_c = TaskId("TASK-003".to_string());

        let artifacts = PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "test".to_string(),
            milestones: vec![PlannedMilestone {
                id: TaskId("MS-1".to_string()),
                name: "M1: Test".to_string(),
                goal: "Test goal".to_string(),
                issues: vec![
                    PlannedIssue {
                        id: cycle_a.clone(),
                        title: "Task A".to_string(),
                        summary: "A".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_c.clone()], // A blocked by C (cycle)
                        blocks: vec![],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_b.clone(),
                        title: "Task B".to_string(),
                        summary: "B".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_a.clone()], // B blocked by A
                        blocks: vec![],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_c.clone(),
                        title: "Task C".to_string(),
                        summary: "C".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_b.clone()], // C blocked by B
                        blocks: vec![],
                        sub_issues: vec![],
                        task_file: None,
                    },
                ],
                acceptance_criteria: vec![],
                verification_steps: vec![],
                notes: None,
            }],
            manifest: TaskPackageManifest {
                planning_wave: "test".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec!["M1: Test".to_string()],
                tasks: vec![
                    ManifestTask {
                        id: cycle_a.clone(),
                        file: "docs/tasks/a.md".to_string(),
                    },
                    ManifestTask {
                        id: cycle_b.clone(),
                        file: "docs/tasks/b.md".to_string(),
                    },
                    ManifestTask {
                        id: cycle_c.clone(),
                        file: "docs/tasks/c.md".to_string(),
                    },
                ],
            },
            milestone_index: String::new(),
            task_files: BTreeMap::new(),
        };

        let result = validate_dependency_graph(&artifacts);
        assert!(result.is_err(), "cycle should be detected");
        match result.expect_err("expected error should be returned") {
            GenerationError::CircularDependency(msg) => {
                assert!(msg.contains("Cycle"));
            }
            other => panic!("expected CircularDependency, got {:?}", other),
        }
    }

    #[test]
    fn dependency_graph_validation_detects_deep_cycle() {
        // Build a 3-node cycle: A blocks B, B blocks C, C blocks A
        // The old buggy implementation (passing &[] for deps) would NOT detect this.
        let cycle_a = TaskId("TASK-001".to_string());
        let cycle_b = TaskId("TASK-002".to_string());
        let cycle_c = TaskId("TASK-003".to_string());

        let artifacts = PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "test".to_string(),
            milestones: vec![PlannedMilestone {
                id: TaskId("MS-1".to_string()),
                name: "M1: Test".to_string(),
                goal: "Test goal".to_string(),
                issues: vec![
                    PlannedIssue {
                        id: cycle_a.clone(),
                        title: "Task A".to_string(),
                        summary: "A".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_c.clone()],
                        blocks: vec![cycle_b.clone()],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_b.clone(),
                        title: "Task B".to_string(),
                        summary: "B".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_a.clone()],
                        blocks: vec![cycle_c.clone()],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_c.clone(),
                        title: "Task C".to_string(),
                        summary: "C".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_b.clone()],
                        blocks: vec![],
                        sub_issues: vec![],
                        task_file: None,
                    },
                ],
                acceptance_criteria: vec![],
                verification_steps: vec![],
                notes: None,
            }],
            manifest: TaskPackageManifest {
                planning_wave: "test".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec!["M1: Test".to_string()],
                tasks: vec![
                    ManifestTask {
                        id: cycle_a.clone(),
                        file: "docs/tasks/a.md".to_string(),
                    },
                    ManifestTask {
                        id: cycle_b.clone(),
                        file: "docs/tasks/b.md".to_string(),
                    },
                    ManifestTask {
                        id: cycle_c.clone(),
                        file: "docs/tasks/c.md".to_string(),
                    },
                ],
            },
            milestone_index: String::new(),
            task_files: BTreeMap::new(),
        };

        let result = validate_dependency_graph(&artifacts);
        assert!(
            result.is_err(),
            "deep 3-node cycle should be detected (old bug passed &[] for deps)"
        );
        match result.expect_err("expected error should be returned") {
            GenerationError::CircularDependency(msg) => {
                assert!(msg.contains("Cycle"));
            }
            other => panic!("expected CircularDependency, got {:?}", other),
        }
    }
}
