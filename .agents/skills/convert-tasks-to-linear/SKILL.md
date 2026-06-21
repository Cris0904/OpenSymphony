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
```

`areas` is optional for older task packages, but new packages should include
stable lowercase area slugs. The converter applies them to Linear as canonical
`area:<slug>` labels.

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
converter cannot simply overwrite labels. Labels are merged by namespace:

- **`area:*`** is rebuilt exactly from the task's frontmatter `areas` field.
  - When `areas` is present (including an empty list), the converter drops
    every existing `area:*` label and applies exactly the listed ones.
  - When `areas` is absent, existing `area:*` labels are preserved.
- **`repo:*`** is *not* managed by this skill (LOC-25 owns repo labels).
  When `apply` is called without an explicit desired-repo state, existing
  `repo:*` labels are preserved.
- **All other labels** (e.g. `priority:*`, `ops:*`, hand-set team labels)
  are preserved untouched.

Concretely:

- A re-publish never wipes a hand-set `repo:` label.
- A re-publish never deletes a `priority:` / `severity:` / `bug:` label
  authored outside this converter.
- The converter fails the run before calling `issue_update` if the existing
  label set is paginated/truncated so it cannot be proven complete.
- Per-issue label hydration happens for both provenance-discovered issues
  and issues mapped through `linear-publish.yaml`.

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
- Unmanaged labels (hand-set `repo:`, `priority:`, etc.) survive a re-publish.
- No issue is blocked by itself.
- Local task IDs remain only in provenance comments or explicit source-context sections.
- `linear-publish.yaml` contains every converted task.

## Fallback

When a package predates `task-package.yaml`, first create the manifest and align
task frontmatter with the contract. Use direct Linear GraphQL calls only for
manual repair or recovery after the scripted path reports a clear blocker.