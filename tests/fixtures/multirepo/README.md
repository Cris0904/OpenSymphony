# Multi-Repo Frontmatter Fixtures

Tiny multi-repo task-package fixtures that exercise the
`create-implementation-plan` / `ManifestValidator` /
`convert-tasks-to-linear validate` contract end-to-end.

## Layout

- `tiny-multi-repo-sub-issues/` — one parent (no `repo:`) + two
  sub-issue leaves (`repo: repo-a`, `repo: repo-b`). Each leaf is a
  Linear sub-issue; the parent is the review node. This is the
  canonical LOC-29 fixture and matches the contract acceptance
  criterion "one parent with no `repo`, one leaf with `repo: repo-a`,
  and one leaf with `repo: repo-b`".
- `tiny-multi-repo-top-level/` — two top-level leaves each carrying
  a different `repo:` slug (`repo-a`, `repo-b`). Every task is a
  top-level Linear issue with no sub-issues, so each task is a
  leaf by `parent:` reference analysis. This exercises the
  multi-repo top-level layout where the planning agent has chosen
  to file each repo's work as a separate top-level issue.

Both fixtures ship with a `.opensymphony/project-set.yaml` so the
Python `convert-tasks-to-linear validate` step can prove the
`repo:` slugs match inventory keys character-for-character without
touching the real OpenSymphony project-set.

## Contract coverage

- Leaf tasks carry exactly one `repo:` slug, exact inventory key.
- Parent tasks carry no `repo:` value, even when they own sub-issues.
- `areas:` lists never use the reserved `repo:<slug>` namespace.
- Repo slugs are preserved character-for-character — no
  lowercasing, no slugification, no whitespace coercion beyond
  trimming.

## Consumers

- `crates/opensymphony-planning/src/graph_validate/manifest.rs`
  Rust unit tests (`tiny_multi_repo_top_level_plan_passes`,
  `tiny_multi_repo_sub_issue_plan_passes`).
- `.agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py`
  `validate` subcommand when run from the fixture directory.
- `scripts/multirepo_planner_contract_check.sh`
  `check_create_implementation_plan_contract` guard.
