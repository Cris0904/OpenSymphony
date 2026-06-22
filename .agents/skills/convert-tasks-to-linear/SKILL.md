---
name: convert-tasks-to-linear
description: |
  Use this skill when a docs/tasks/task-package.yaml planning wave should be
  validated, previewed, or published to Linear with milestone assignments,
  parent/sub-issue relationships, blocker relations, and additive label
  management.
---

# Convert Task Packages To Linear

## Purpose

Convert a deterministic task package into Linear milestones, issues,
sub-issues, and blocker relations.

The task package is the planning source of truth. Linear is the published
projection. Publish results are stored locally in `docs/tasks/linear-publish.yaml`
so later waves can update or resume reliably.

## Required Inputs

- Repository root.
- `docs/tasks/task-package.yaml`.
- Linear project slug.
- Linear workspace/team access through `LINEAR_API_KEY`.
- Optional team key when the Linear project has more than one team.

## Task Package Contract

`create-implementation-plan` should create this package:

```yaml
planningWave: rich-client-hosted-mode
tasksDir: docs/tasks
milestones:
  - "M1: Gateway And Stream Contract"
  - "M2: Shared Client And Desktop Alpha"
tasks:
  - id: TASK-001
    file: docs/tasks/001-current-gateway-inventory.md
```

Rules:

- `planningWave` is a stable string identifier for the planning round.
- `milestones` contains exact Linear milestone names.
- `tasks` is the complete list of files to convert.
- Task file discovery uses the manifest list.
- `docs/tasks/milestones.md` is expected for human review, while conversion uses `task-package.yaml`.

Each task file must include:

```yaml
id: TASK-001
title: Current Gateway Inventory
milestone: "M1: Gateway And Stream Contract"
priority: 3
estimate: 3
blockedBy: []
blocks: []
areas:
  - gateway
parent: null
repo: opensymphony
```

`areas` is optional for older task packages, but new packages should include
stable lowercase area slugs. The converter applies them to Linear as canonical
`area:<slug>` labels.

`repo` is **required on leaf tasks** (top-level issues without `parent` and
every sub-issue) and **forbidden on parent/review tasks** (top-level issues
with sub-issues). The value MUST be the exact project-set repo slug /
`RepoRef.key`. The converter publishes it as a canonical Linear label named
`repo:<slug>` and rejects any other value as an invalid-repo-routing error
during `validate`. See the *Reserved Linear label namespaces* section below
and `create-implementation-plan/SKILL.md` for the planning-side contract
that produces this frontmatter.

### Reserved Linear label namespaces

OpenSymphony owns and manages two reserved Linear label namespaces. The
converter MUST treat them as managed and MUST NOT let user-supplied values
collide with them:

- `area:<slug>` — the canonical Memory / docs area label. Emitted only from
  `areas` frontmatter (after slug normalization).
- `repo:<slug>` — the canonical repository identity label. Emitted only from
  the task's `repo` frontmatter (exact-match) or applied by the runtime
  resolver from the project-set inventory. `repo:<slug>` MUST map to the
  exact project-set repo slug / `RepoRef.key`; the converter does not
  lowercase, slugify, or otherwise coerce it.

The two namespaces are deliberately separate:

- `areas` frontmatter MUST produce only `area:<slug>` labels. A `repo:<slug>`
  entry (or any other reserved non-area namespace) MUST NOT appear in
  `areas`; the converter validates `areas` values during `validate` and
  rejects any reserved non-area namespace such as `repo:` (see
  [LOC-25](https://linear.app/localgputokenscrazy/issue/LOC-25/planning-seeds-the-repo-skill-and-crate)
  and the `normalize_area_slugs` helper in
  `convert_tasks_to_linear.py`). Keep `areas` strictly area-shaped at
  planning time so the validation never has to fire on real waves.
- `repo` frontmatter (when present) MUST produce exactly one `repo:<slug>`
  label per leaf task; parents and review nodes MUST NOT carry `repo:`.
  Repo label emission, inventory validation, and the parent-vs-leaf
  shape are part of
  [LOC-25](https://linear.app/localgputokenscrazy/issue/LOC-25/planning-seeds-the-repo-skill-and-crate);
  the namespace-aware update path that keeps live `repo:` labels from being
  wiped or stale-preserved belongs to
  [LOC-22](https://linear.app/localgputokenscrazy/issue/LOC-22/converter-additive-label-update).

### Area slug normalization vs exact repo slug matching

Area and repo labels follow different rules on purpose:

- **Areas are normalized.** The converter lowercases, trims, and slugifies
  each `areas` entry (see `area_slug` in `convert_tasks_to_linear.py`), so
  `OpenHands Runtime`, `OpenHands-Runtime`, and `area:OpenHands Runtime`
  all collapse to the canonical `area:openhands-runtime`.
- **Repo slugs are exact.** `repo:<slug>` MUST match the project-set
  inventory slug / `RepoRef.key` character-for-character. The converter
  trims surrounding whitespace but does not lowercase or slugify; the
  resolver depends on the exact key to look the repo up.

See `create-implementation-plan/SKILL.md` for the planning-side contract that
produces this frontmatter.

## Preferred Script Workflow

Use the skill-local Python converter:

```bash
uv run --script .agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py \
  validate \
  --manifest docs/tasks/task-package.yaml
```

Preview without Linear writes:

```bash
uv run --script .agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py \
  dry-run \
  --manifest docs/tasks/task-package.yaml
```

Publish to Linear:

```bash
uv run --script .agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py \
  apply \
  --manifest docs/tasks/task-package.yaml \
  --project-slug my-project-5250e49b61f4
```

If the Linear project contains multiple teams, pass `--team-key TEAMKEY`.

## Publish Output

Successful `apply` writes `docs/tasks/linear-publish.yaml`:

```yaml
planningWave: rich-client-hosted-mode
linearProject: my-project-5250e49b61f4
publishedAt: "2026-05-12T10:30:00-05:00"
tasks:
  TASK-001:
    issue: COE-123
    issueId: 00000000-0000-0000-0000-000000000000
    url: https://linear.app/workspace/issue/COE-123/current-gateway-inventory
    file: docs/tasks/001-current-gateway-inventory.md
```

The publish file is the primary mapping for future updates. The converter also
adds short HTML comments to Linear issue descriptions as a recovery aid:

```markdown
<!-- task-planning-wave: rich-client-hosted-mode -->
<!-- task-source-id: TASK-001 -->
```

## Conversion Behavior

- Validate the manifest, frontmatter, sections, parent references, dependency references, and dependency DAG before any Linear writes.
- Create or reuse Linear milestones by exact milestone name.
- Create or update top-level tasks as Linear issues.
- Create or update tasks with `parent` as Linear sub-issues.
- Create tasks in dependency waves so every parent and blocker exists before a dependent task needs it.
- Apply blocker relations through Linear issue relation metadata.
- Rewrite created issue bodies so task references point to real Linear issue IDs and canonical URLs.
- Update the Linear project overview with a planning-wave summary and live issue links.

## Label Management (Additive, Namespace-Aware)

Linear's `issue_update` REPLACES the issue's label set on every call, so the
converter cannot simply overwrite labels. Labels are merged by namespace
through `scripts/label_merge.py`, which exposes a
`DesiredRepo` policy the converter projects per task from the validated
`repo:` frontmatter:

- **`area:*`** is rebuilt exactly from the task's frontmatter `areas` field.
  - When `areas` is present (including an empty list), the converter drops
    every existing `area:*` label and applies exactly the listed ones.
  - When `areas` is absent, existing `area:*` labels are preserved.
- **`repo:*`** is managed by this skill for repo-aware packages (LOC-30).
  The converter projects a `DesiredRepo` value per task from
  `validate_repo_routing`'s leaf-vs-parent verdict:
  - **Leaves** (no `parent:` frontmatter) with a non-empty `repo:` slug get
    `DesiredRepo.managed(<slug>)` — exactly one `repo:<slug>` label is
    applied; every other pre-existing `repo:*` label is dropped.
  - **Parents / review nodes** (id appears in any other task's `parent:`)
    get `DesiredRepo.cleared()` — no `repo:*` label survives; this
    guarantees stale parent-side labels are removed on re-publish.
  - **Defensive preserve** only fires when the validator was bypassed (the
    frontmatter omitted `repo:` on a leaf). The publish path then keeps
    existing `repo:*` labels untouched instead of stripping them.
- **All other labels** (e.g. `priority:*`, `ops:*`, hand-set team labels)
  are preserved untouched regardless of the `area:*` / `repo:*` policy.

Concretely:

- A re-publish of a leaf drops every pre-existing `repo:*` label and
  applies exactly one `repo:<slug>` label (the declared slug).
- A re-publish of a parent removes every pre-existing `repo:*` label so
  routing never accidentally lands on a review node.
- A re-publish never deletes a `priority:` / `severity:` / `bug:` /
  `ops:` label authored outside this converter.
- The converter fails the run before calling `issue_update` if the
  existing label set is paginated/truncated so it cannot be proven
  complete. Both the project-level `_assert_project_state_complete`
  guard and the per-issue `fetch_labels_complete` paginator enforce
  this — neither provenance-discovered issues nor
  `linear-publish.yaml`-mapped issues can update against a partial
  label set.
- Per-issue label hydration happens for both provenance-discovered
  issues and issues mapped through `linear-publish.yaml` (the mapped
  path always paginates until the cursor reports `hasNextPage: false`).

The merge helper itself lives in
`scripts/label_merge.py` and can be imported independently for unit tests
or for callers (like LOC-25) that want to drive the same merge logic.

## Validation Checklist

Before reporting success:

- Every manifest task exists in Linear.
- Every task is assigned to the expected milestone.
- Every `parent` task is represented as a Linear parent/sub-issue relationship.
- Every `blockedBy` edge is represented as a Linear blocker relation.
- Every declared area is represented as a Linear `area:<slug>` label.
- Every leaf task's `repo:` slug is published as exactly one
  `repo:<slug>` Linear label (LOC-30).
- Every parent / review task carries zero `repo:*` Linear labels
  (LOC-30).
- Unmanaged labels (hand-set `priority:`, `ops:`, etc.) and out-of-package
  `repo:` labels survive a re-publish; only the converter's managed
  namespaces (`area:*` and `repo:*`) are rewritten.
- No issue is blocked by itself.
- Local task IDs remain only in provenance comments or explicit source-context sections.
- `linear-publish.yaml` contains every converted task.

## Fallback

When a package predates `task-package.yaml`, first create the manifest and align
task frontmatter with the contract. Use direct Linear GraphQL calls only for
manual repair or recovery after the scripted path reports a clear blocker.
