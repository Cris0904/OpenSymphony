---
id: OSYM-600
title: Multi-Repo Phase 1 — Repo Identity and Dispatch (Integration Audit)
type: parent
area: orchestrator
priority: P0
estimate: 13
milestone: M14: Multi-Repo Phase 1
linear: LOC-9
children:
  - OSYM-601 (LOC-11, RepoRef domain type)
  - OSYM-602 (LOC-13, repo on issue + single resolver)
  - OSYM-603 (LOC-14, dispatch gate + parent review-node principle)
  - OSYM-604 (LOC-15, static clone hook + env-injected RepoRef)
related:
  - OSYM-700+ multi-repo memory planning wave
decision_refs:
  - D3 — repo only on terminals
  - D4 — canonical identity `RepoRef`
  - D5 — `repo:<slug>` label + resolver
  - D6 — terminal without repo ⇒ BLOCK
  - D7 — clone via static hook + env-injected RepoRef
  - D8 — Arch A (one orchestrator over N repos)
  - D10 — parent = lightweight, read-only review node
status: integration-verified
---

# OSYM-600: Multi-Repo Phase 1 — Repo Identity and Dispatch (Integration Audit)

## Scope of this artifact

`LOC-9` is the **parent / integration-review node** for Phase 1 of the
multi-repo dispatch path (D10). It is intentionally **read-only**: it does
not duplicate child work, it does not clone or write across repos, and it
does not invent new code that belongs to a child. Its deliverable is the
integration audit captured in this file plus the verification evidence
captured in the Linear workpad comment.

The four child issues implement the actual seams:

| Child   | Linear | Title                                                       | Commit  | Merged PR |
|:-------:|:------:|:------------------------------------------------------------|:--------|:----------|
| OSYM-601 | LOC-11 | `RepoRef` domain type                                       | `302ce01` | #1 |
| OSYM-602 | LOC-13 | repo on the issue + single resolver                         | `19f71c6` | #3 |
| OSYM-603 | LOC-14 | dispatch gate + parent review-node principle               | `655932f` | #4 |
| OSYM-604 | LOC-15 | static `workspace clone` hook + env-injected `RepoRef`      | `f49857d` | #5 |

All four children are `Done` in Linear, all four are merged to
`origin/main` at `923a139`, and all four are independently covered by
green `cargo test-system-duckdb` suites.

## Integration seam: "label → RepoRef → gate → clone"

End-to-end path for a terminal leaf issue that carries a valid
`repo:<slug>` label:

1. **Label parse + resolve** — `repo_for_issue` in
   `crates/opensymphony-orchestrator/src/repo_resolver.rs` reads the
   issue's `repo:<slug>` label(s), validates it is a leaf (parents
   return `None` per D3), looks the slug up in
   `SchedulerConfig.project_set_inventory`, and returns a `RepoRef`.
2. **Carry on `NormalizedIssue`** — the resolved `RepoRef` lands in
   `NormalizedIssue::execution_repo_ref` (added in
   `crates/opensymphony-domain/src/issue.rs`) and is therefore
   accessible to every downstream consumer (gate, workspace manager,
   snapshot) without going back to the tracker.
3. **Dispatch gate (D6 / D10)** — `dispatch_ready_issues` in
   `crates/opensymphony-orchestrator/src/scheduler.rs` consults
   `should_dispatch_issue` and the `selection` helpers
   (`issue_is_blocked_for_missing_repo`, `issue_is_parent_deferred`).
   - Leaf with `execution_repo_ref == Some(_)` ⇒ dispatch.
   - Leaf with `execution_repo_ref == None` ⇒ release as
     `ReleaseReason::MissingRepo` (no clone, no agent).
   - Parent (`sub_issues` non-empty) ⇒ release as
     `ReleaseReason::ParentDeferred` (D10).
4. **Workspace prepare + env injection** — for a dispatched leaf, the
   workspace manager builds a `HookContext { workspace, repo_ref }`
   and `inject_repo_env` materializes:
   - `OPENSYMPHONY_EXECUTION_REPO_URL` (always)
   - `OPENSYMPHONY_EXECUTION_REPO_KEY` (always)
   - `OPENSYMPHONY_EXECUTION_REPO_DEFAULT_BRANCH` (only when
     `RepoRef.default_branch` is `Some`)

   These constants are exported from
   `crates/opensymphony-workspace/src/lib.rs` and consumed by the static
   `after_create` hook.
5. **Static clone (D7)** — the `after_create` hook is the static
   command `opensymphony workspace clone`. The subcommand lives in
   `crates/opensymphony-cli/src/workspace_clone.rs`; it reads the three
   env vars, materializes a `git clone` argv (no `sh -c`, no
   templating), and either fetches or fast-forwards the target
   workspace.

Arch A (D8) is preserved because the entire path runs inside a single
`Scheduler::tick()` over a unified `project_set_inventory` —
`max_concurrent_agents`, the OpenHands server, and the listening port
are global, not per-repo.

## Integration evidence

All evidence below was captured against
`origin/main @ 923a139` (LOC-19: init multi-repo onboarding), branch
`cristianmunozholguin/loc-9-multi-repo-phase-1-repo-identity-and-dispatch`.

### Acceptance Criteria

- [x] All four child issues are Done and their cargo tests are green.
- [x] A terminal issue with a valid `repo:<slug>` is dispatched and
      clones the resolved URL.
- [x] A terminal issue without a resolvable repo is blocked (not
      dispatched).
- [x] Arch A single-loop preserved (D8).

### Targeted test runs

| Suite                                            | Result   |
|:-------------------------------------------------|:---------|
| `opensymphony_domain::repo`                       | 3 / 3    |
| `opensymphony_orchestrator::repo_resolver`        | 14 / 14  |
| `opensymphony_cli::workspace_clone`               | 13 / 13  |
| `tests/scheduler.rs` (integration)                | 23 / 23  |
| `tests/workspace_manager.rs` (integration)        | 28 / 28  |
| `cargo check-system-duckdb` (workspace-wide)     | green    |
| `cargo clippy-system-duckdb` (workspace-wide)     | green    |
| `cargo test-system-duckdb -- --test-threads=1`    | green    |

The full suite is run with the parent shell's
`OPENSYMPHONY_MEMORY_*` env vars unset because the `memory` integration
suite invokes the `opensymphony memory` binary and reads
`OPENSYMPHONY_MEMORY_*` from the inherited parent shell; the `run`
integration suite binds ephemeral ports that must not collide with
other parallel tests, hence `--test-threads=1`.

### Re-running the live label read (LOC-13 / LOC-8 regression)

The "live label read" referenced by the LOC-9 `Test Plan` is encoded in
`crates/opensymphony-orchestrator/tests/scheduler.rs` (the
`poll_normalize_*` and `dispatch_gate_*` tests at line ~1050). These
three tests are the canonical "tracker label → execution_repo_ref"
end-to-end read:

- `poll_normalize_populates_execution_repo_ref_from_known_slug`
- `poll_normalize_leaves_execution_repo_ref_none_when_inventory_empty`
- `poll_normalize_leaves_execution_repo_ref_none_for_unknown_slug`

All three passed in the integration run.

## Discovered follow-ups

None discovered during this integration review. The four child seams
compose without seams-of-seams, and the dispatch gate's
`ReleaseReason::{MissingRepo, ParentDeferred}` taxonomy already
captures the operator-visible failure modes the parent is responsible
for re-validating.

## Notes

- This file exists solely as a durable record of the LOC-9 review; it
  intentionally does not duplicate the child task files (OSYM-601..604).
- The integration audit is mirrored in the Linear workpad comment id
  `f78f9efd-166a-4f1e-9af5-af6ebbf5a057` on `LOC-9`.
- Future Phase 1 follow-ups (e.g., parent dynamic task creation, E1/E2
  explorations of cross-repo read-only review) are explicitly out of
  scope here and will be filed as separate `Backlog` issues if/when
  surfaced.
