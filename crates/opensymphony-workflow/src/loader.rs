use std::{fs, path::Path};

use super::{
    WorkflowDefinition, WorkflowFrontMatter, error::WorkflowLoadError, model::ProjectSetFrontMatter,
};

pub(crate) fn load_workflow_from_path(
    path: &Path,
) -> Result<WorkflowDefinition, WorkflowLoadError> {
    let contents = fs::read_to_string(path).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => WorkflowLoadError::MissingWorkflowFile {
            path: path.to_path_buf(),
        },
        _ => WorkflowLoadError::ReadWorkflowFile {
            path: path.to_path_buf(),
            source,
        },
    })?;

    parse_workflow(&contents)
}

pub(crate) fn parse_workflow(source: &str) -> Result<WorkflowDefinition, WorkflowLoadError> {
    let Some(split) = split_front_matter(source) else {
        return Ok(WorkflowDefinition {
            front_matter: WorkflowFrontMatter::default(),
            prompt_template: source.to_owned(),
        });
    };

    let Some(front_matter) = parse_front_matter(split.front_matter)? else {
        return Ok(WorkflowDefinition {
            front_matter: WorkflowFrontMatter::default(),
            prompt_template: source.to_owned(),
        });
    };

    Ok(WorkflowDefinition {
        front_matter,
        prompt_template: split.body.to_owned(),
    })
}

fn parse_front_matter(
    front_matter: &str,
) -> Result<Option<WorkflowFrontMatter>, WorkflowLoadError> {
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(front_matter)
        .map_err(|source| WorkflowLoadError::WorkflowParseError { source })?;

    match parsed {
        serde_yaml::Value::Null if front_matter.trim().is_empty() => {
            Ok(Some(WorkflowFrontMatter::default()))
        }
        serde_yaml::Value::Null => Ok(None),
        serde_yaml::Value::Mapping(_) => {
            let parsed: WorkflowFrontMatter = serde_yaml::from_value(parsed)
                .map_err(|source| WorkflowLoadError::WorkflowParseError { source })?;

            match parsed.extensions.keys().next() {
                Some(namespace) => Err(WorkflowLoadError::UnknownTopLevelNamespace {
                    namespace: namespace.clone(),
                }),
                None => Ok(Some(parsed)),
            }
        }
        _ => Ok(None),
    }
}

/// Result of [`split_front_matter`].
///
/// Carries all four slices of a fenced front-matter document so callers can
/// reconstruct the original bytes without doing pointer arithmetic on
/// intermediate string views. The struct is the canonical output of the
/// shared front-matter parser (see `LOC-19` AI review feedback on
/// `init_repo.rs::strip_project_set_owned_fields` using `as_ptr().addr()`
/// to splice back together the original marker lines).
#[derive(Debug, Clone, Copy)]
pub struct FrontMatterSplit<'a> {
    /// The opening `---\n` (or trailing-CR variant) marker line, verbatim.
    pub head: &'a str,
    /// The YAML front matter, i.e. everything between the two `---`
    /// marker lines. Callers parse and re-serialize this slice.
    pub front_matter: &'a str,
    /// The closing `---\n` marker line, verbatim.
    pub trailer: &'a str,
    /// Everything that follows the closing `---` line.
    pub body: &'a str,
}

/// Splits a `WORKFLOW.md`-style document into its YAML front matter and the
/// body that follows it.
///
/// The function is the canonical parser used by [`parse_workflow`] and is
/// re-exported so other crates (e.g. the `init` command) can reuse the same
/// implementation instead of reinventing it (see `LOC-19` AI review feedback
/// on the divergent `split_front_matter` parser in `init_repo.rs`).
///
/// Returns a [`FrontMatterSplit`] with the opening marker line, the YAML
/// front matter, the closing marker line, and the trailing body. Returns
/// `None` when the document has no leading `---` marker or when no closing
/// `---` marker is found.
///
/// The parser walks the document line-by-line, so a YAML scalar that
/// contains `---` (e.g. `description: "use --- as separator"`) is correctly
/// preserved instead of being mistaken for the closing delimiter.
pub fn split_front_matter(source: &str) -> Option<FrontMatterSplit<'_>> {
    let mut lines = source.split_inclusive('\n');
    let first_line = lines.next()?;

    if trim_line(first_line) != "---" {
        return None;
    }

    let mut offset = first_line.len();
    for line in lines {
        let line_length = line.len();
        if trim_line(line) == "---" {
            let body_start = offset + line_length;
            return Some(FrontMatterSplit {
                head: &source[..first_line.len()],
                front_matter: &source[first_line.len()..offset],
                trailer: &source[offset..body_start],
                body: &source[body_start..],
            });
        }

        offset += line_length;
    }

    None
}

fn trim_line(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

// ---------------------------------------------------------------------------
// Project-set config loader
// ---------------------------------------------------------------------------

/// Loads and parses `.opensymphony/project-set.yaml` from `config_root`.
///
/// Returns `Ok(None)` when the file does not exist (legacy single-repo flow).
pub(crate) fn load_project_set_from_config_root(
    config_root: &Path,
) -> Result<Option<ProjectSetFrontMatter>, WorkflowLoadError> {
    let path = config_root.join(".opensymphony").join("project-set.yaml");
    load_project_set_from_path(&path)
}

/// Loads and parses the project-set config from an explicit path.
///
/// Returns `Ok(None)` when the file does not exist.
pub(crate) fn load_project_set_from_path(
    path: &Path,
) -> Result<Option<ProjectSetFrontMatter>, WorkflowLoadError> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(WorkflowLoadError::ReadWorkflowFile {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    parse_project_set(&contents).map(Some)
}

/// Parses a project-set YAML document.
pub(crate) fn parse_project_set(source: &str) -> Result<ProjectSetFrontMatter, WorkflowLoadError> {
    serde_yaml::from_str(source).map_err(|source| WorkflowLoadError::WorkflowParseError { source })
}
