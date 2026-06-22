---
name: create-implementation-plan
description: |
  Generate a structured implementation plan from project requirements with
  decomposed tasks, milestones, dependencies, and acceptance criteria.
  Use when starting a new project or planning a new development wave that
  should produce docs/tasks/task-package.yaml and issue-ready task files.
---

# Create Implementation Plan Skill

## Purpose

Generate a structured planning package from project requirements. The package
must include shared project context plus a deterministic task manifest that can
be reviewed by humans, converted to Linear, and executed by implementation
agents without hidden chat history.

## When To Use

Use this skill when a team is ready to turn a product idea, PRD, design note,
research brief, or follow-on development request into implementation tasks.

For iterative projects, create a new planning wave instead of rewriting the
identity of an already-published wave. A planning wave is a named round of
planning and decomposition such as `bootstrap-mvp`, `hosted-alpha`, or
`rich-client-hosted-mode`.

## Required Inputs

Before generating files, gather or infer:

- Project or planning-wave name.
- Project description and success criteria.
- Key requirements and features.
- Technical constraints and preferences.
- Existing PRDs, architecture notes, source files, external links, and research.
- Existing task and Linear context when this is a follow-on planning wave.

## Process

### Step 1: Gather Context

Collect relevant source material and synthesize it before creating tasks:

- Existing design documents and PRDs.
- Stakeholder requirements and success criteria.
- Technical research findings.
- Reference implementations or public API documentation.
- Existing repo conventions, architecture, and task history.

Add targeted supplemental research only where it improves the task plan.

### Step 2: Generate Shared Context And Architecture Documentation

Create or update these files when the project needs them:

**AGENTS.md** - Persistent implementation context for coding agents:

- Project mission and scope.
- Non-negotiable constraints and architectural invariants.
- Cross-cutting definitions and repository conventions.
- Commands, environment expectations, and references to deeper docs.

If `AGENTS.md` already exists, refine it while preserving useful project-specific guidance.

**README.md** - Human-facing project overview:

- Problem statement and goals.
- Setup and primary workflows.
- High-level architecture summary.
- Links to detailed docs and task plans.

**docs/architecture.md** - System architecture:

- Component breakdown.
- Data flow.
- Integration points.
- Technology choices and rationale.

**docs/decisions/** - Architecture Decision Records when decisions need a durable record.

### Step 3: Create The Task Package Manifest

Create `docs/tasks/task-package.yaml`. This file is the canonical
machine-readable input for `convert-tasks-to-linear`.

Use this shape:

```yaml
planningWave: rich-client-hosted-mode
tasksDir: docs/tasks
milestones:
  - "M1: Gateway And Stream Contract"
  - "M2: Shared Client And Desktop Alpha"
tasks:
  - id: TASK-001
    file: docs/tasks/001-current-gateway-inventory.md
  - id: TASK-002
    file: docs/tasks/002-gateway-schemas.md
```

Rules:

- `planningWave` is a stable string identifier for this planning round.
- `tasksDir` is the directory containing task files.
- `milestones` contains exact Linear milestone names.
- `tasks` is the complete list of task files for this wave.
- Task discovery reads the manifest task list.

### Step 4: Generate Implementation Tasks

Create one Markdown file per task. File names may follow a readable convention
such as `001-brief-description.md`, but the manifest is the source of truth.

Each task file must include this frontmatter. The template shows a
top-level **leaf** task — `parent: null` plus a single `repo:` slug.

For a top-level **parent** / review task (one that owns sub-issues),
omit `repo:` and use `parent: null`; the validator rejects any
parent that carries a `repo:` value. For a **sub-issue**, set
`parent: <parent-id>` and carry a `repo:` slug (sub-issues are
always leaves).

```markdown
---
id: TASK-001
title: Human-readable task title
milestone: "M1: Gateway And Stream Contract"
priority: 3
estimate: 3
blockedBy: []
blocks: []
areas:
  - gateway
parent: null
repo: opensymphony
---
```

A multi-repo top-level layout — one parent, one leaf per repo — looks
like this:

```markdown
---
# Parent / review task — no `repo:` allowed.
id: TASK-PARENT
title: Multi-repo parent
milestone: "M14: Multi-Repo Phase 1"
priority: 3
estimate: 5
blockedBy: []
blocks: []
areas:
  - planning
parent: null
---

---
# Leaf 1 — explicit `repo:` slug, exact inventory key.
id: TASK-LEAF-A
title: Repo-A leaf
milestone: "M14: Multi-Repo Phase 1"
priority: 3
estimate: 3
blockedBy: []
blocks: []
areas:
  - planning
parent: null
repo: repo-a
---

---
# Leaf 2 — same parent, different repo.
id: TASK-LEAF-B
title: Repo-B leaf
milestone: "M14: Multi-Repo Phase 1"
priority: 3
estimate: 3
blockedBy: []
blocks: []
areas:
  - planning
parent: null
repo: repo-b
---
```

The same shape applies to sub-issue leaves: set `parent: <id>` and
carry a single `repo:` slug. A mixed parent + sub-issues layout is
explicit in `tests/fixtures/multirepo/tiny-multi-repo-plan` so the
contract is reproducible from a real on-disk fixture.

Field rules:

- `id` must be unique within `task-package.yaml`.
- `milestone` must exactly match one entry in `task-package.yaml`.
- `priority` uses Linear-compatible numeric priority: `1=Urgent`, `2=High`, `3=Normal`, `4=Low`.
- `estimate` is a numeric story-point estimate.
- `blockedBy` and `blocks` contain task IDs from the same manifest.
- `areas` contains planning-time area slugs chosen with LLM judgment, such as
  `memory`, `openhands-runtime`, or `gateway`. The converter publishes them as
  canonical Linear labels named `area:<slug>`.
- `parent` is `null` for top-level issues or a task ID for a Linear sub-issue.
- `repo: <slug>` is the leaf-task routing identity. It is **required on leaf tasks**
  (top-level issues without `parent` and every sub-issue) and is
  **forbidden on parent/review tasks** (top-level issues with sub-issues).
  The `<slug>` MUST be the **exact** project-set inventory repo slug /
  `RepoRef.key` — no lowercasing, no slugification, no whitespace
  coercion beyond trimming. The converter publishes it as a canonical
  Linear label named `repo:<slug>`. See the *Reserved Linear label
  namespaces* section below for the dual-source-of-truth contract with
  `opensymphony-planning`.

#### One-repo obvious-case auto-fill

The Rust planner in `opensymphony-planning` exposes the project-set
inventory to the planning session via `PlanningSession::available_repos`
and a `single_repo_slug()` accessor. When the inventory has **exactly
one** repo entry and a leaf task's `repo:` field is omitted, the
generator may auto-fill the obvious single slug; the validator then
treats the leaf as in-contract. When the inventory has **zero** or
**multiple** repo entries, multi-repo assignment is the responsibility
of the planning agent / human — the planner does **not** infer the
slug from `areas`, from the issue title, or from any other heuristic.
The Python `convert-tasks-to-linear validate` command is the publish-time
gate: even an auto-filled leaf is revalidated against the inventory
slug before any Linear write happens. The contract is therefore:

- **Inventory has one repo**: the planning session may leave `repo:`
  unset on a leaf and the validator treats the auto-filled slug as
  in-contract. Always prefer setting `repo:` explicitly so the
  frontmatter is self-describing.
- **Inventory has zero repos**: the planning session **must** set
  `repo:` explicitly on every leaf, even if the project-set inventory
  has not been onboarded yet; the manifest validator's
  `missing_leaf_repo` finding still fires.
- **Inventory has multiple repos**: the planning agent / human
  **must** pick the exact `repo:` slug per leaf; the planner never
  infers from `areas`, the issue title, or any other signal. The
  validator's `missing_leaf_repo` finding fires for any leaf without
  an exact slug, and `unknown_repo_slug` fires for any slug outside
  the inventory at publish time.

### Reserved Linear label namespaces

OpenSymphony owns and manages two reserved Linear label namespaces. Future
work MUST NOT reuse, reinterpret, or collide with them:

- `area:<slug>` — the canonical Memory / docs area label. Only `areas`
  frontmatter produces these labels.
- `repo:<slug>` — the canonical repository identity label, resolved by the
  single Linear `repo_for_issue` resolver into a `RepoRef`. Only the project-set
  inventory and explicit `repo:` frontmatter produce these labels.

The two namespaces are deliberately separate:

- `areas` frontmatter owns only `area:<slug>` labels. A planning task MUST NOT
  place a `repo:<slug>` entry (or any other reserved non-area namespace) in
  its `areas` list. The converter's `normalize_area_slugs` helper rejects
  `areas` values that use a reserved non-area namespace such as `repo:`
  (see [LOC-25](https://linear.app/localgputokenscrazy/issue/LOC-25/planning-seeds-the-repo-skill-and-crate));
  keep `areas` strictly area-shaped at planning time so the validation
  never has to fire on real waves.
- Quick mental model: `areas` produces `area:<slug>`; `repo` produces
  `repo:<slug>`; mixing `repo:<slug>` into `areas` is a misuse of the
  `areas` namespace.
- `repo:<slug>` is published from the task's `repo` frontmatter (see
  `convert-tasks-to-linear/SKILL.md`) and uses the **exact** project-set repo
  slug / `RepoRef.key`. It is not lowercased, slugified, or otherwise coerced.

### Area slug normalization vs exact repo slug matching

`area:<slug>` and `repo:<slug>` follow different normalization rules on purpose:

- Area slugs are **normalized**: the converter lowercases, trims, and
  slugifies each `areas` entry (see `area_slug` in
  `convert_tasks_to_linear.py`), so `OpenHands Runtime`, `OpenHands-Runtime`,
  and `area:OpenHands Runtime` all collapse to the canonical `area:openhands-runtime`.
- Repo slugs are **exact**: `repo:<slug>` MUST match the project-set inventory
  slug / `RepoRef.key` character-for-character. Do not lowercase, slugify, or
  reorder segments; the resolver depends on the exact key to look the repo up.

This split keeps areas user-friendly at planning time while keeping repo
identity strictly tied to the project-set inventory.

Use this body structure:

```markdown
## Summary

One or two sentences describing what this task accomplishes.

## Scope

### In scope

- Specific item 1
- Specific item 2

### Out of scope

- Explicitly excluded item 1

## Deliverables

- File or artifact 1
- File or artifact 2

## Acceptance Criteria

- [ ] Criterion 1: measurable outcome
- [ ] Criterion 2: measurable outcome

## Test Plan

- Test command or verification step 1
- Test command or verification step 2

## Context

- Relevant repo paths to inspect or modify.
- Docs, specs, or external sources to read first.
- Parent task, blockers, or sibling work that matter.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Any additional context, references, or gotchas.
```

### Step 5: Generate The Human Milestone Index

Create `docs/tasks/milestones.md` as the human-readable overview. It should use
the same milestone names as `task-package.yaml`.

```markdown
# Project Milestones

## M1: Gateway And Stream Contract

Goal: Establish the versioned gateway and stream contract.

Tasks:

- TASK-001 Current Gateway Inventory
- TASK-002 Gateway Schemas
```

`milestones.md` can include goals and explanatory prose. Conversion relies on
`task-package.yaml`.

### Step 6: Validate Completeness

Before finishing, check that:

- Every manifest task file exists.
- Every task has required frontmatter and body sections.
- Every task ID is unique.
- Every dependency and parent reference points to a manifest task.
- `blockedBy`, `blocks`, and `parent` references point to manifest tasks.
- Area slugs are stable, lowercase, and useful for deterministic memory lookup.
- The dependency graph has no cycles.
- Each task is independently implementable.
- Acceptance criteria and test plans are measurable.

## Expected Output

After completing this skill, the repository should contain:

```text
AGENTS.md
README.md
docs/
├── architecture.md
├── decisions/
└── tasks/
    ├── task-package.yaml
    ├── milestones.md
    ├── 001-bootstrap.md
    ├── 002-setup-testing.md
    └── ...
```

## Next Steps

After generating the package:

1. Ask the user to review `docs/tasks/task-package.yaml`, `docs/tasks/milestones.md`, and the task files.
2. Use `convert-tasks-to-linear` to validate, dry-run, and publish the planning wave.
3. Verify hierarchy, blockers, and project placement in Linear.
4. Begin execution with `opensymphony run`.
