//! On-disk validator for `docs/tasks/task-package.yaml`.
//!
//! The validator reads the task package manifest plus each declared task
//! file and emits a [`ManifestValidationResult`] that captures the same
//! five error classes the legacy Python converter already exposes:
//!
//! - missing task files
//! - unknown milestones (declared on a task but absent from the manifest)
//! - unknown dependencies (declared in `blockedBy` but absent from the
//!   manifest's `tasks` list)
//! - creation-order cycles (Kahn-style topological check)
//! - self-blocks (a task declaring itself in `blockedBy`)
//! - duplicate task IDs in the manifest
//!
//! Findings are surfaced as separate vector fields so the planning-session
//! API can render each class in its own section. The result supports
//! `is_ok()` so callers can use it as a fast-fail predicate.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::opensymphony_planning::generator::domain::TaskId;

use super::domain::{
    ManifestValidationResult, MissingTaskFile, SelfBlock, UnknownDependency, UnknownMilestone,
};
use super::frontmatter::{TaskFrontmatter, TaskFrontmatterError, parse_task_file};

/// Raw representation of `docs/tasks/task-package.yaml`.
///
/// The on-disk schema uses camelCase keys (`planningWave`, `tasksDir`)
/// matching the existing fixture files. Fields are decoded with explicit
/// `#[serde(rename = ...)]` so the validator works without a custom
/// `serde` adapter.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskPackageManifestFile {
    #[serde(rename = "planningWave")]
    pub planning_wave: String,
    #[serde(rename = "tasksDir", default)]
    pub tasks_dir: String,
    #[serde(default)]
    pub milestones: Vec<String>,
    pub tasks: Vec<ManifestTaskEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestTaskEntry {
    pub id: String,
    pub file: String,
}

/// Loads a task-package manifest from disk.
pub fn load_manifest(path: &Path) -> Result<TaskPackageManifestFile, ManifestValidatorError> {
    let raw = fs::read_to_string(path).map_err(|source| ManifestValidatorError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_yaml::from_str(&raw).map_err(|source| ManifestValidatorError::Yaml {
        path: path.display().to_string(),
        source,
    })
}

/// Errors surfaced by manifest loading. Validation paths emit their own
/// non-error `ManifestValidationResult` instead.
#[derive(Debug, thiserror::Error)]
pub enum ManifestValidatorError {
    #[error("failed to read manifest {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse manifest {path}: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

/// Manifest validator.
#[allow(dead_code)]
pub struct ManifestValidator;

impl ManifestValidator {
    /// Validates the supplied manifest file path (the manifest itself) and
    /// each declared task file path. Missing or unreadable task files are
    /// surfaced via `missing_task_files`, not as hard errors.
    #[allow(dead_code)]
    pub fn validate(
        manifest_path: &Path,
    ) -> Result<ManifestValidationResult, ManifestValidatorError> {
        let manifest = load_manifest(manifest_path)?;
        let repo_root = manifest_path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(Self::validate_against_repo_root(&manifest, &repo_root))
    }

    /// Same as [`Self::validate`] but takes a pre-parsed manifest and a
    /// repository root to anchor relative task paths. Useful for unit tests
    /// that exercise the validator with temporary fixtures on disk.
    pub fn validate_against_repo_root(
        manifest: &TaskPackageManifestFile,
        repo_root: &Path,
    ) -> ManifestValidationResult {
        let mut result = ManifestValidationResult {
            planning_wave: manifest.planning_wave.clone(),
            declared_task_ids: Vec::new(),
            missing_task_files: Vec::new(),
            unknown_milestones: Vec::new(),
            unknown_dependencies: Vec::new(),
            creation_order_cycles: Vec::new(),
            self_blocks: Vec::new(),
            duplicate_task_ids: Vec::new(),
        };

        let mut seen_ids: BTreeSet<TaskId> = BTreeSet::new();
        let mut entries: Vec<(TaskId, PathBuf, TaskFrontmatter)> = Vec::new();
        let milestone_set: BTreeSet<String> = manifest.milestones.iter().cloned().collect();

        for entry in &manifest.tasks {
            let id = TaskId::new(entry.id.clone());
            if !seen_ids.insert(id.clone()) {
                result.duplicate_task_ids.push(id);
                continue;
            }
            result.declared_task_ids.push(id.clone());
            let path = repo_root.join(&entry.file);
            match parse_task_file(&path) {
                Ok(parsed) => entries.push((id.clone(), path, parsed.frontmatter)),
                Err(TaskFrontmatterError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    result.missing_task_files.push(MissingTaskFile {
                        task_id: id.clone(),
                        file_path: entry.file.clone(),
                    });
                }
                Err(_) => {
                    // Malformed task file is treated as if it were not loadable.
                    result.missing_task_files.push(MissingTaskFile {
                        task_id: id.clone(),
                        file_path: entry.file.clone(),
                    });
                }
            }
        }

        let id_set: BTreeSet<TaskId> = result.declared_task_ids.iter().cloned().collect();
        let mut adjacency: BTreeMap<TaskId, BTreeSet<TaskId>> = BTreeMap::new();
        for (task_id, _, frontmatter) in &entries {
            if !milestone_set.contains(frontmatter.milestone.as_deref().unwrap_or_default())
                && let Some(declared) = frontmatter.milestone.clone()
            {
                result.unknown_milestones.push(UnknownMilestone {
                    task_id: task_id.clone(),
                    declared_milestone: declared,
                });
            }
            for dep in &frontmatter.blocked_by {
                if dep == &task_id.0 {
                    result.self_blocks.push(SelfBlock {
                        task_id: task_id.clone(),
                    });
                } else if !id_set.contains(&TaskId::new(dep.clone())) {
                    result.unknown_dependencies.push(UnknownDependency {
                        from_task_id: task_id.clone(),
                        unknown_dependency: TaskId::new(dep.clone()),
                    });
                } else {
                    adjacency
                        .entry(task_id.clone())
                        .or_default()
                        .insert(TaskId::new(dep.clone()));
                }
            }
        }

        result.creation_order_cycles = creation_order_cycles(&adjacency, &id_set);
        // Stable order keeps the artefact diff-friendly for tests.
        result
            .missing_task_files
            .sort_by(|a, b| a.task_id.cmp(&b.task_id));
        result
            .unknown_milestones
            .sort_by(|a, b| a.task_id.cmp(&b.task_id));
        result
            .unknown_dependencies
            .sort_by(|a, b| a.from_task_id.cmp(&b.from_task_id));
        result.self_blocks.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        result
    }
}

/// Returns the minimal cycles (one representative cycle path per
/// strongly-connected component) in the directed graph implied by
/// `adjacency`. We use a simple DFS for tasks working in BTreeMap order
/// so the output is deterministic.
fn creation_order_cycles(
    adjacency: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    nodes: &BTreeSet<TaskId>,
) -> Vec<Vec<TaskId>> {
    let mut visited: BTreeSet<TaskId> = BTreeSet::new();
    let mut on_stack: BTreeSet<TaskId> = BTreeSet::new();
    let mut stack: Vec<TaskId> = Vec::new();
    let mut seen_cycles: BTreeSet<Vec<TaskId>> = BTreeSet::new();
    let mut collected: Vec<Vec<TaskId>> = Vec::new();

    for entry in nodes {
        if !visited.contains(entry) {
            dfs_cycle(
                entry,
                adjacency,
                &mut visited,
                &mut on_stack,
                &mut stack,
                &mut seen_cycles,
                &mut collected,
            );
        }
    }
    collected
}

fn dfs_cycle(
    node: &TaskId,
    adjacency: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    visited: &mut BTreeSet<TaskId>,
    on_stack: &mut BTreeSet<TaskId>,
    stack: &mut Vec<TaskId>,
    seen_cycles: &mut BTreeSet<Vec<TaskId>>,
    collected: &mut Vec<Vec<TaskId>>,
) {
    visited.insert(node.clone());
    on_stack.insert(node.clone());
    stack.push(node.clone());
    if let Some(deps) = adjacency.get(node) {
        for dep in deps {
            if !visited.contains(dep) {
                dfs_cycle(
                    dep,
                    adjacency,
                    visited,
                    on_stack,
                    stack,
                    seen_cycles,
                    collected,
                );
            } else if on_stack.contains(dep)
                && let Some(start_idx) = stack.iter().position(|n| n == dep)
            {
                let mut cycle: Vec<TaskId> = stack[start_idx..].to_vec();
                cycle.push(dep.clone());
                cycle.sort();
                if seen_cycles.insert(cycle.clone()) {
                    collected.push(cycle);
                }
            }
        }
    }
    on_stack.remove(node);
    stack.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    fn write(path: &Path, contents: &str) {
        let mut file = fs::File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write file");
    }

    fn fixture_with_manifest(
        tmp: &Path,
        manifest_text: &str,
        files: Vec<(String, String)>,
    ) -> TaskPackageManifestFile {
        write(&tmp.join("task-package.yaml"), manifest_text);
        for (path, contents) in files {
            let full = tmp.join(&path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).expect("mkdir");
            }
            write(&full, &contents);
        }
        load_manifest(&tmp.join("task-package.yaml")).expect("manifest loads")
    }

    fn manifest_with_tasks(tasks: &[(&str, &str)]) -> String {
        let mut s = String::from(
            "planningWave: test\ntasksDir: docs/tasks\nmilestones:\n  - \"M1\"\ntasks:\n",
        );
        for (id, file) in tasks {
            s.push_str(&format!("  - id: {}\n    file: {}\n", id, file));
        }
        s
    }

    fn task_file_text(id: &str, milestone: &str, blocked_by: &[&str], blocks: &[&str]) -> String {
        let mut s = format!(
            "---\nid: {}\ntitle: \"{}\"\nmilestone: \"{}\"\nblockedBy: [",
            id, id, milestone
        );
        s.push_str(
            &blocked_by
                .iter()
                .map(|x| format!("\"{}\"", x))
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push_str("]\nblocks: [");
        s.push_str(
            &blocks
                .iter()
                .map(|x| format!("\"{}\"", x))
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push_str("]\n---\n# Test\n");
        s
    }

    #[test]
    fn validates_clean_manifest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text =
            manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md"), ("TASK-B", "docs/tasks/b.md")]);
        let files = vec![
            (
                "docs/tasks/a.md".to_string(),
                task_file_text("TASK-A", "M1", &[], &["TASK-B"]),
            ),
            (
                "docs/tasks/b.md".to_string(),
                task_file_text("TASK-B", "M1", &["TASK-A"], &[]),
            ),
        ];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert!(result.is_ok(), "unexpected findings: {result:?}");
        assert_eq!(result.error_count(), 0);
    }

    #[test]
    fn missing_task_file_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[
            ("TASK-A", "docs/tasks/a.md"),
            ("TASK-B", "docs/tasks/missing.md"),
        ]);
        let files = vec![(
            "docs/tasks/a.md".to_string(),
            task_file_text("TASK-A", "M1", &[], &[]),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert!(!result.is_ok());
        assert_eq!(result.missing_task_files.len(), 1);
        assert_eq!(result.missing_task_files[0].task_id, TaskId::new("TASK-B"));
    }

    #[test]
    fn unknown_milestone_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md")]);
        let files = vec![(
            "docs/tasks/a.md".to_string(),
            task_file_text("TASK-A", "M9", &[], &[]),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.unknown_milestones.len(), 1);
        assert_eq!(result.unknown_milestones[0].declared_milestone, "M9");
    }

    #[test]
    fn unknown_dependency_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md")]);
        let files = vec![(
            "docs/tasks/a.md".to_string(),
            task_file_text("TASK-A", "M1", &["TASK-GHOST"], &[]),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.unknown_dependencies.len(), 1);
        assert_eq!(
            result.unknown_dependencies[0].unknown_dependency,
            TaskId::new("TASK-GHOST")
        );
    }

    #[test]
    fn creation_order_cycle_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text =
            manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md"), ("TASK-B", "docs/tasks/b.md")]);
        let files = vec![
            (
                "docs/tasks/a.md".to_string(),
                task_file_text("TASK-A", "M1", &["TASK-B"], &[]),
            ),
            (
                "docs/tasks/b.md".to_string(),
                task_file_text("TASK-B", "M1", &["TASK-A"], &[]),
            ),
        ];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.creation_order_cycles.len(), 1);
    }

    #[test]
    fn self_block_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md")]);
        let files = vec![(
            "docs/tasks/a.md".to_string(),
            task_file_text("TASK-A", "M1", &["TASK-A"], &[]),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.self_blocks.len(), 1);
        assert_eq!(result.self_blocks[0].task_id, TaskId::new("TASK-A"));
    }

    #[test]
    fn duplicate_task_id_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[
            ("TASK-A", "docs/tasks/a.md"),
            ("TASK-A", "docs/tasks/copy.md"),
        ]);
        let files = vec![
            (
                "docs/tasks/a.md".to_string(),
                task_file_text("TASK-A", "M1", &[], &[]),
            ),
            (
                "docs/tasks/copy.md".to_string(),
                task_file_text("TASK-A", "M1", &[], &[]),
            ),
        ];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.duplicate_task_ids, vec![TaskId::new("TASK-A")]);
    }
}
