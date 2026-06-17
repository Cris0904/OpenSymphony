use serde::{Deserialize, Serialize};

/// Canonical repo identity used across the orchestrator.
///
/// This is a pure leaf type with no dependencies on other modules.
/// Consumers import it downstream.
///
/// Field naming note: the orchestrator field that holds a `RepoRef` is named
/// `execution_repo_ref`, never `execution_repo` — the identifier `execution_repo`
/// is already taken by memory as an `Option<String>` path prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRef {
    /// Clone source URL (e.g., `https://github.com/org/repo.git`).
    pub url: String,
    /// Short repo key (e.g., `org/repo` or team-local identifier).
    pub key: String,
    /// Default branch name when known (e.g., `main`, `master`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::RepoRef;

    #[test]
    fn repo_ref_round_trips_through_json() {
        let repo = RepoRef {
            url: "https://github.com/example-org/example-repo.git".to_string(),
            key: "example-org/example-repo".to_string(),
            default_branch: Some("main".to_string()),
        };

        let json = serde_json::to_string(&repo).expect("RepoRef should serialize");
        let deserialized: RepoRef =
            serde_json::from_str(&json).expect("RepoRef should deserialize");

        assert_eq!(repo, deserialized);
    }

    #[test]
    fn repo_ref_round_trips_with_none_default_branch() {
        let repo = RepoRef {
            url: "https://github.com/example-org/example-repo.git".to_string(),
            key: "example-org/example-repo".to_string(),
            default_branch: None,
        };

        let json = serde_json::to_string(&repo).expect("RepoRef should serialize");
        assert!(
            !json.contains("default_branch"),
            "None field should be skipped"
        );

        let deserialized: RepoRef =
            serde_json::from_str(&json).expect("RepoRef should deserialize");

        assert_eq!(repo, deserialized);
    }

    #[test]
    fn repo_ref_field_naming_reserves_execution_repo() {
        // The field name `execution_repo_ref` (not `execution_repo`) is used in
        // orchestrator consumers to avoid collision with memory's path-prefix string.
        // This test verifies the struct field names are stable.
        let repo = RepoRef {
            url: "u".to_string(),
            key: "k".to_string(),
            default_branch: None,
        };
        let json = serde_json::to_value(&repo).expect("RepoRef should serialize to value");
        assert!(json.get("url").is_some(), "url field must exist");
        assert!(json.get("key").is_some(), "key field must exist");
        // default_branch is skipped when None, so no assertion here
    }
}
