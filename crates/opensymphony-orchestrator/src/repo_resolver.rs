//! Single label-only resolver for the per-issue execution repo (LOC-13).
//!
//! This module is the **only** place that knows about the `repo:<slug>` Linear
//! label format. The label stores a short slug; the URL/branch/key come from
//! the project-set inventory (see LOC-12). The resolver is intentionally
//! decoupled from dispatch behavior (LOC-14): it only returns facts.
//!
//! Resolution rules (per D3 / D5):
//!
//! * Only **leaf** issues (no sub-issues) may resolve to a repo.
//! * Leaf issues must carry **exactly one** `repo:<slug>` label that maps to
//!   the inventory. Anything else (`None`, empty, unknown slug, multiple
//!   `repo:` labels) resolves to `None`.
//! * Parent issues (issues with sub-issues) always resolve to `None`, even
//!   when they carry `repo:` labels — those labels are silently ignored.
//! * `repo:` is a reserved prefix. `area:<slug>`, `repo` (no colon), or any
//!   other label format are NOT treated as repo labels.

use std::collections::BTreeMap;

use crate::opensymphony_domain::{NormalizedIssue, RepoRef};

/// Resolves the per-issue execution repo from `issue.labels` against the
/// project-set inventory.
///
/// Returns `Some(RepoRef)` only when `issue` is a leaf (`sub_issues.is_empty()`)
/// and carries exactly one `repo:<slug>` label whose slug is a key in
/// `project_set_inventory`. All other inputs — parent issues, unlabeled
/// leaves, unknown slugs, multiple `repo:` labels, non-`repo:` labels —
/// resolve to `None`.
///
/// The resolver never panics; it only inspects the labels and inventory.
pub fn repo_for_issue(
    issue: &NormalizedIssue,
    project_set_inventory: &BTreeMap<String, RepoRef>,
) -> Option<RepoRef> {
    // D3: parents (issues with sub-issues) never carry a repo. Any `repo:`
    // label found on a parent is intentionally ignored at this layer.
    if !issue.sub_issues.is_empty() {
        return None;
    }

    let mut found: Option<&str> = None;
    for raw in &issue.labels {
        let Some(slug) = canonical_repo_label_slug(raw) else {
            continue;
        };
        // D5: a terminal (leaf) issue with multiple `repo:` labels is invalid.
        match found {
            None => found = Some(slug),
            Some(_) => return None,
        }
    }

    let slug = found?;
    project_set_inventory.get(slug).cloned()
}

/// Prefix parser for the `repo:<slug>` Linear label format.
///
/// Analogous to `canonical_area_label_slug` in
/// `crates/opensymphony-memory/src/capture_render.rs`. Whitespace around the
/// label is trimmed; the prefix is matched case-insensitively and only when
/// followed by exactly one colon and a non-empty slug. The slug is returned
/// verbatim (no slugification) because the project-set inventory is keyed by
/// the literal slug, and we want the lookup to fail loudly on slug mismatch
/// rather than silently coerce it.
fn canonical_repo_label_slug(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    let (prefix, suffix) = trimmed.split_once(':')?;
    if !prefix.eq_ignore_ascii_case("repo") {
        return None;
    }
    let slug = suffix.trim();
    if slug.is_empty() { None } else { Some(slug) }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::opensymphony_domain::{IssueId, IssueIdentifier, IssueRef, IssueState, RepoRef};

    use super::repo_for_issue;

    fn repo(url: &str, key: &str) -> RepoRef {
        RepoRef {
            url: url.to_string(),
            key: key.to_string(),
            default_branch: Some("main".to_string()),
        }
    }

    fn leaf_issue(labels: &[&str]) -> crate::opensymphony_domain::NormalizedIssue {
        crate::opensymphony_domain::NormalizedIssue {
            id: IssueId::new("lin_1").expect("issue id should be valid"),
            identifier: IssueIdentifier::new("LOC-1").expect("identifier should be valid"),
            title: "leaf".to_string(),
            description: None,
            priority: None,
            state: IssueState {
                id: None,
                name: "Todo".to_string(),
                category: crate::opensymphony_domain::IssueStateCategory::NonActive,
            },
            branch_name: None,
            url: None,
            labels: labels.iter().map(|label| label.to_string()).collect(),
            parent_id: None,
            blocked_by: Vec::new(),
            sub_issues: Vec::new(),
            created_at: None,
            updated_at: None,
            execution_repo_ref: None,
        }
    }

    fn parent_issue(labels: &[&str]) -> crate::opensymphony_domain::NormalizedIssue {
        let mut issue = leaf_issue(labels);
        issue.sub_issues = vec![IssueRef {
            id: IssueId::new("lin_2").expect("issue id should be valid"),
            identifier: IssueIdentifier::new("LOC-2").expect("identifier should be valid"),
            state: "Todo".to_string(),
        }];
        issue.title = "parent".to_string();
        issue
    }

    fn inventory(pairs: &[(&str, &str)]) -> BTreeMap<String, RepoRef> {
        pairs
            .iter()
            .map(|(slug, url)| ((*slug).to_string(), repo(url, slug)))
            .collect()
    }

    #[test]
    fn leaf_with_known_repo_label_resolves_to_repo_ref() {
        let issue = leaf_issue(&["repo:test-repo"]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        let resolved = repo_for_issue(&issue, &inv).expect("repo should resolve");

        assert_eq!(resolved.key, "test-repo");
        assert_eq!(resolved.url, "https://example.com/test-repo.git");
    }

    #[test]
    fn leaf_with_unknown_repo_slug_resolves_to_none() {
        let issue = leaf_issue(&["repo:nope"]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        assert!(repo_for_issue(&issue, &inv).is_none());
    }

    #[test]
    fn leaf_with_no_labels_resolves_to_none() {
        let issue = leaf_issue(&[]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        assert!(repo_for_issue(&issue, &inv).is_none());
    }

    #[test]
    fn leaf_with_multiple_repo_labels_resolves_to_none() {
        let issue = leaf_issue(&["repo:alpha", "repo:beta"]);
        let inv = inventory(&[
            ("alpha", "https://example.com/alpha.git"),
            ("beta", "https://example.com/beta.git"),
        ]);

        assert!(
            repo_for_issue(&issue, &inv).is_none(),
            "multiple `repo:` labels must be treated as invalid"
        );
    }

    #[test]
    fn parent_with_repo_label_ignores_label_and_resolves_to_none() {
        let issue = parent_issue(&["repo:test-repo"]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        assert!(
            repo_for_issue(&issue, &inv).is_none(),
            "parent issues never carry a repo, regardless of labels"
        );
    }

    #[test]
    fn parent_with_no_labels_resolves_to_none() {
        let issue = parent_issue(&[]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        assert!(repo_for_issue(&issue, &inv).is_none());
    }

    #[test]
    fn parent_with_multiple_repo_labels_still_resolves_to_none() {
        let issue = parent_issue(&["repo:alpha", "repo:beta"]);
        let inv = inventory(&[
            ("alpha", "https://example.com/alpha.git"),
            ("beta", "https://example.com/beta.git"),
        ]);

        assert!(repo_for_issue(&issue, &inv).is_none());
    }

    #[test]
    fn empty_inventory_makes_any_repo_label_unresolved() {
        let issue = leaf_issue(&["repo:test-repo"]);
        let inv: BTreeMap<String, RepoRef> = BTreeMap::new();

        assert!(repo_for_issue(&issue, &inv).is_none());
    }

    #[test]
    fn area_label_is_not_parsed_as_repo_label() {
        let issue = leaf_issue(&["area:test-area"]);
        let inv = inventory(&[("test-area", "https://example.com/area.git")]);

        assert!(
            repo_for_issue(&issue, &inv).is_none(),
            "`area:` is not a reserved repo prefix"
        );
    }

    #[test]
    fn repo_prefix_without_colon_is_not_parsed_as_repo_label() {
        let issue = leaf_issue(&["repo", "repo:test-repo"]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        let resolved = repo_for_issue(&issue, &inv).expect("single valid label still resolves");
        assert_eq!(resolved.key, "test-repo");
    }

    #[test]
    fn repo_prefix_with_empty_slug_is_not_parsed_as_repo_label() {
        let issue = leaf_issue(&["repo:", "repo:test-repo"]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        let resolved = repo_for_issue(&issue, &inv).expect("single valid label still resolves");
        assert_eq!(resolved.key, "test-repo");
    }

    #[test]
    fn repo_prefix_match_is_case_insensitive() {
        let issue = leaf_issue(&["REPO:test-repo"]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        let resolved =
            repo_for_issue(&issue, &inv).expect("case-insensitive prefix should resolve");
        assert_eq!(resolved.key, "test-repo");
    }

    #[test]
    fn unrelated_label_does_not_block_single_repo_label() {
        let issue = leaf_issue(&["area:linear", "foundation", "repo:test-repo"]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        let resolved = repo_for_issue(&issue, &inv)
            .expect("repo label should resolve alongside area/foundation");
        assert_eq!(resolved.key, "test-repo");
    }

    #[test]
    fn slug_with_surrounding_whitespace_is_trimmed() {
        let issue = leaf_issue(&["  repo:  test-repo  "]);
        let inv = inventory(&[("test-repo", "https://example.com/test-repo.git")]);

        let resolved = repo_for_issue(&issue, &inv).expect("trimmed label should resolve");
        assert_eq!(resolved.key, "test-repo");
    }
}
