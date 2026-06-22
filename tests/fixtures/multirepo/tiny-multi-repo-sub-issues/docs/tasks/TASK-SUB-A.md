---
id: TASK-SUB-A
title: Repo-A Sub-Issue
milestone: "M14: Multi-Repo Phase 1"
priority: 3
estimate: 3
blockedBy: []
blocks: []
areas:
  - planning
parent: TASK-PARENT
repo: repo-a
---

## Summary

Sub-issue leaf for repo-a in the LOC-29 tiny multi-repo sub-issue
fixture. Sub-issues are always leaves and therefore MUST carry a
`repo:` slug.

## Scope

### In scope

- Carry the sub-issue leaf shape.
- Demonstrate the leaf-with-exact-repo contract.

### Out of scope

- Inventory matching — lives in the Python `validate` step.

## Deliverables

- One sub-issue leaf task with `repo: repo-a`.

## Acceptance Criteria

- [ ] The manifest validator accepts this leaf as in-contract.
- [ ] The Python `convert-tasks-to-linear validate` command accepts
      this leaf against the project-set inventory.

## Test Plan

- `cargo test --lib opensymphony_planning::graph_validate::manifest`
  on the tiny multi-repo sub-issue fixture.

## Context

- `.agents/skills/create-implementation-plan/SKILL.md`
- `crates/opensymphony-planning/src/graph_validate/manifest.rs`

## Definition of Ready

- [ ] `repo: repo-a` matches an inventory key.

## Notes

Reproducible fixture: `tests/fixtures/multirepo/tiny-multi-repo-sub-issues`.
