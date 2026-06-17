use std::{fs, path::Path};

use super::{
    WorkflowDefinition, WorkflowFrontMatter,
    error::WorkflowLoadError,
    model::ProjectSetFrontMatter,
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
    let Some((front_matter_source, prompt_source)) = split_front_matter(source) else {
        return Ok(WorkflowDefinition {
            front_matter: WorkflowFrontMatter::default(),
            prompt_template: source.to_owned(),
        });
    };

    let Some(front_matter) = parse_front_matter(front_matter_source)? else {
        return Ok(WorkflowDefinition {
            front_matter: WorkflowFrontMatter::default(),
            prompt_template: source.to_owned(),
        });
    };

    Ok(WorkflowDefinition {
        front_matter,
        prompt_template: prompt_source.to_owned(),
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

fn split_front_matter(source: &str) -> Option<(&str, &str)> {
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
            return Some((&source[first_line.len()..offset], &source[body_start..]));
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
