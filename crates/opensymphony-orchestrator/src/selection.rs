use std::collections::HashSet;

use crate::opensymphony_domain::{NormalizedIssue, TrackerIssue};

pub fn issue_blocked_by_non_terminal_blockers(issue: &TrackerIssue) -> bool {
    issue
        .blocked_by
        .iter()
        .any(|blocker| !blocker.is_terminal())
}

pub fn parent_issue_blocked_by_incomplete_children(
    issue: &TrackerIssue,
    terminal_states: &HashSet<String>,
) -> bool {
    !issue.sub_issues.is_empty()
        && issue
            .sub_issues
            .iter()
            .any(|sub_issue| !sub_issue.is_terminal(terminal_states))
}

pub fn should_dispatch_issue(issue: &TrackerIssue, terminal_states: &HashSet<String>) -> bool {
    !issue_blocked_by_non_terminal_blockers(issue)
        && !parent_issue_blocked_by_incomplete_children(issue, terminal_states)
}

/// Dispatch-gate predicate for **D6** — terminal leaf issues without a
/// resolvable repo are blocked from the work-clone path (LOC-14).
///
/// `is_terminal_leaf` is `true` when the issue is a leaf (no sub-issues).
/// `repo_resolved` is the boolean outcome of
/// [`crate::repo_resolver::repo_for_issue`] — i.e. whether the issue has
/// exactly one `repo:<slug>` label that maps to the project-set inventory.
///
/// Returns `true` only when the issue is a terminal leaf **and** has no
/// resolvable repo. Parents never trigger this predicate (they are
/// handled by [`issue_is_parent_deferred`]); leaves with a resolvable
/// repo always return `false`.
///
/// The predicate is pure and side-effect-free; the orchestrator passes
/// the resolved-repo fact in from the scheduler because the selection
/// layer has no inventory of its own.
pub fn issue_is_blocked_for_missing_repo(is_terminal_leaf: bool, repo_resolved: bool) -> bool {
    is_terminal_leaf && !repo_resolved
}

/// Dispatch-gate predicate for **D10** — cross-repo parent issues are
/// treated as lightweight, read-only review nodes and must not enter the
/// work-clone path (LOC-14). The deep mechanism for a parent to read
/// child workspaces read-only is exploration E2 and is explicitly out of
/// scope for Phase 1.
///
/// Returns `true` when the issue has at least one sub-issue. The
/// orchestrator does not clone a workspace, does not start a worker,
/// and emits a `ParentDeferred` `ReleaseReason` on the execution
/// snapshot so operators can see the deferral.
///
/// `repo:` labels on a parent are intentionally ignored — the repo
/// resolver already returns `None` for parents, and the gate fires
/// regardless of any `repo:` labels they happen to carry.
pub fn issue_is_parent_deferred(issue: &NormalizedIssue) -> bool {
    !issue.sub_issues.is_empty()
}

pub fn filter_issues_for_dispatch<I>(
    issues: I,
    terminal_states: &HashSet<String>,
) -> Vec<TrackerIssue>
where
    I: IntoIterator<Item = TrackerIssue>,
{
    let mut filtered = issues
        .into_iter()
        .filter(|issue| should_dispatch_issue(issue, terminal_states))
        .collect::<Vec<_>>();
    sort_issues_for_dispatch(&mut filtered);
    filtered
}

pub fn sort_issues_for_dispatch(issues: &mut [TrackerIssue]) {
    issues.sort_by(|left, right| {
        priority_rank(left)
            .cmp(&priority_rank(right))
            .then_with(|| left.sub_issues.len().cmp(&right.sub_issues.len()))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.identifier.cmp(&right.identifier))
    });
}

fn priority_rank(issue: &TrackerIssue) -> u8 {
    issue.priority.unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::opensymphony_domain::{
        IssueId, IssueIdentifier, IssueRef, IssueState, IssueStateCategory, NormalizedIssue,
        TrackerIssue, TrackerIssueBlocker, TrackerIssueRef, TrackerIssueState,
        TrackerIssueStateKind,
    };
    use chrono::{DateTime, Utc};

    use super::{
        filter_issues_for_dispatch, issue_blocked_by_non_terminal_blockers,
        issue_is_blocked_for_missing_repo, issue_is_parent_deferred,
        parent_issue_blocked_by_incomplete_children, should_dispatch_issue,
        sort_issues_for_dispatch,
    };

    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("timestamp should parse")
            .with_timezone(&Utc)
    }

    fn terminal_states() -> HashSet<String> {
        HashSet::from([String::from("Done"), String::from("Canceled")])
    }

    fn state(name: &str, kind: TrackerIssueStateKind) -> TrackerIssueState {
        TrackerIssueState {
            id: format!("state-{}", name.to_ascii_lowercase().replace(' ', "-")),
            name: name.to_string(),
            tracker_type: match &kind {
                TrackerIssueStateKind::Completed => "completed",
                TrackerIssueStateKind::Canceled => "canceled",
                TrackerIssueStateKind::Started => "started",
                TrackerIssueStateKind::Unstarted => "unstarted",
                TrackerIssueStateKind::Backlog => "backlog",
                TrackerIssueStateKind::Triage => "triage",
                TrackerIssueStateKind::Unknown(_) => "unknown",
            }
            .to_string(),
            kind,
        }
    }

    fn blocker(identifier: &str, state: TrackerIssueState) -> TrackerIssueBlocker {
        TrackerIssueBlocker {
            id: format!("issue-{}", identifier.to_ascii_lowercase()),
            identifier: identifier.to_string(),
            title: format!("Issue {identifier}"),
            state,
        }
    }

    fn child(identifier: &str, state: &str) -> TrackerIssueRef {
        TrackerIssueRef {
            id: format!("issue-{}", identifier.to_ascii_lowercase()),
            identifier: identifier.to_string(),
            title: None,
            url: None,
            state: state.to_string(),
        }
    }

    fn issue(
        identifier: &str,
        priority: Option<u8>,
        created_at: &str,
        blocked_by: Vec<TrackerIssueBlocker>,
        sub_issues: Vec<TrackerIssueRef>,
    ) -> TrackerIssue {
        TrackerIssue {
            id: format!("issue-{}", identifier.to_ascii_lowercase()),
            identifier: identifier.to_string(),
            url: format!("https://linear.app/example/{identifier}"),
            title: format!("Issue {identifier}"),
            description: None,
            priority,
            state: "In Progress".to_string(),
            labels: Vec::new(),
            parent_id: None,
            parent: None,
            project_milestone: None,
            blocked_by,
            sub_issues,
            created_at: ts(created_at),
            updated_at: ts(created_at),
        }
    }

    #[test]
    fn parent_issue_is_blocked_when_any_child_is_non_terminal() {
        let issue = issue(
            "COE-277",
            Some(1),
            "2026-03-22T00:00:00Z",
            Vec::new(),
            vec![child("COE-278", "In Progress"), child("COE-279", "Done")],
        );

        assert!(parent_issue_blocked_by_incomplete_children(
            &issue,
            &terminal_states()
        ));
    }

    #[test]
    fn parent_issue_is_ready_when_all_children_are_terminal() {
        let issue = issue(
            "COE-277",
            Some(1),
            "2026-03-22T00:00:00Z",
            Vec::new(),
            vec![child("COE-278", "Done"), child("COE-279", "Canceled")],
        );

        assert!(!parent_issue_blocked_by_incomplete_children(
            &issue,
            &terminal_states()
        ));
    }

    #[test]
    fn blocker_check_composes_with_hierarchy_check() {
        let issue = issue(
            "COE-277",
            Some(1),
            "2026-03-22T00:00:00Z",
            vec![blocker(
                "COE-260",
                state("In Progress", TrackerIssueStateKind::Started),
            )],
            vec![child("COE-278", "Done")],
        );

        assert!(issue_blocked_by_non_terminal_blockers(&issue));
        assert!(!should_dispatch_issue(&issue, &terminal_states()));
    }

    #[test]
    fn sort_prefers_leaf_issues_before_parents_when_priorities_match() {
        let mut issues = vec![
            issue(
                "COE-277",
                Some(1),
                "2026-03-20T00:00:00Z",
                Vec::new(),
                vec![child("COE-278", "Done")],
            ),
            issue(
                "COE-278",
                Some(1),
                "2026-03-21T00:00:00Z",
                Vec::new(),
                Vec::new(),
            ),
        ];

        sort_issues_for_dispatch(&mut issues);

        assert_eq!(
            issues
                .iter()
                .map(|issue| issue.identifier.as_str())
                .collect::<Vec<_>>(),
            vec!["COE-278", "COE-277"]
        );
    }

    #[test]
    fn filter_skips_parent_until_children_finish() {
        let issues = vec![
            issue(
                "COE-277",
                Some(1),
                "2026-03-20T00:00:00Z",
                Vec::new(),
                vec![child("COE-278", "In Progress")],
            ),
            issue(
                "COE-278",
                Some(1),
                "2026-03-21T00:00:00Z",
                Vec::new(),
                Vec::new(),
            ),
        ];

        let filtered = filter_issues_for_dispatch(issues, &terminal_states());

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].identifier, "COE-278");
    }

    #[test]
    fn nested_hierarchy_dispatches_only_the_leaf_issue() {
        let issues = vec![
            issue(
                "COE-P1",
                Some(1),
                "2026-03-20T00:00:00Z",
                Vec::new(),
                vec![child("COE-S1", "In Progress")],
            ),
            issue(
                "COE-S1",
                Some(1),
                "2026-03-21T00:00:00Z",
                Vec::new(),
                vec![child("COE-SS1", "In Progress")],
            ),
            issue(
                "COE-SS1",
                Some(1),
                "2026-03-22T00:00:00Z",
                Vec::new(),
                Vec::new(),
            ),
        ];

        let filtered = filter_issues_for_dispatch(issues, &terminal_states());

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].identifier, "COE-SS1");
    }

    #[test]
    fn adding_a_new_child_reblocks_the_parent_on_the_next_snapshot() {
        let terminal_states = terminal_states();
        let mut parent = issue(
            "COE-277",
            Some(1),
            "2026-03-20T00:00:00Z",
            Vec::new(),
            Vec::new(),
        );

        assert!(should_dispatch_issue(&parent, &terminal_states));

        parent.sub_issues.push(child("COE-278", "Todo"));

        assert!(!should_dispatch_issue(&parent, &terminal_states));
    }

    // ---- LOC-14 dispatch-gate predicates (D6, D10) -----------------------

    fn normalized_leaf(identifier: &str) -> NormalizedIssue {
        NormalizedIssue {
            id: IssueId::new(format!("lin-{identifier}")).expect("issue id should be valid"),
            identifier: IssueIdentifier::new(identifier.to_string())
                .expect("identifier should be valid"),
            title: format!("Issue {identifier}"),
            description: None,
            priority: Some(1),
            state: IssueState {
                id: None,
                name: "In Progress".to_string(),
                category: IssueStateCategory::Active,
            },
            branch_name: None,
            url: None,
            labels: Vec::new(),
            parent_id: None,
            blocked_by: Vec::new(),
            sub_issues: Vec::new(),
            created_at: None,
            updated_at: None,
            execution_repo_ref: None,
        }
    }

    fn normalized_parent(identifier: &str, children: &[(&str, &str)]) -> NormalizedIssue {
        let mut issue = normalized_leaf(identifier);
        issue.sub_issues = children
            .iter()
            .map(|(child_id, child_state)| IssueRef {
                id: IssueId::new(format!("lin-{child_id}")).expect("child id should be valid"),
                identifier: IssueIdentifier::new(child_id.to_string())
                    .expect("child identifier should be valid"),
                state: child_state.to_string(),
            })
            .collect();
        issue.title = format!("Parent {identifier}");
        issue
    }

    #[test]
    fn missing_repo_predicate_blocks_terminal_leaf_without_repo() {
        // D6: leaf with no resolvable repo → blocked.
        assert!(issue_is_blocked_for_missing_repo(true, false));
    }

    #[test]
    fn missing_repo_predicate_passes_leaf_with_resolved_repo() {
        // Leaf with a resolvable repo → not blocked.
        assert!(!issue_is_blocked_for_missing_repo(true, true));
    }

    #[test]
    fn missing_repo_predicate_ignores_parents() {
        // Parents are handled by issue_is_parent_deferred, not this predicate.
        // A "parent without repo" must NOT be reported as MissingRepo so the
        // operator surface distinguishes the two cases.
        assert!(!issue_is_blocked_for_missing_repo(false, false));
        assert!(!issue_is_blocked_for_missing_repo(false, true));
    }

    #[test]
    fn parent_deferred_predicate_only_fires_when_sub_issues_present() {
        // D10: any issue with at least one sub-issue is a parent → deferred.
        let leaf = normalized_leaf("COE-LEAF");
        assert!(!issue_is_parent_deferred(&leaf));

        let parent_with_one_child = normalized_parent("COE-PARENT", &[("COE-CHILD", "Done")]);
        assert!(issue_is_parent_deferred(&parent_with_one_child));

        let parent_with_unfinished_child =
            normalized_parent("COE-PARENT", &[("COE-CHILD", "In Progress")]);
        assert!(
            issue_is_parent_deferred(&parent_with_unfinished_child),
            "parent deferral is independent of child completion in Phase 1"
        );
    }
}
