---
id: TASK-LEAF-A
title: Repo-A Top-Level Leaf
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

## Summary

Top-level leaf task for repo-a in the LOC-29 tiny multi-repo
top-level fixture. No parent is present in the manifest, so the
task is detected as a leaf by `parent:` reference analysis.

## Scope

### In scope

- Top-level leaf with explicit `repo: repo-a`.

### Out of scope

- Inventory matching — lives in the Python `validate` step.

## Deliverables

- One top-level leaf task with `repo: repo-a`.

## Acceptance Criteria

- [ ] Manifest validator accepts the leaf as in-contract.

## Test Plan

- `cargo test --lib opensymphony_planning::graph_validate::manifest`
  on the tiny multi-repo top-level fixture.

## Context

- `.agents/skills/create-implementation-plan/SKILL.md`
- `crates/opensymphony-planning/src/graph_validate/manifest.rs`

## Definition of Ready

- [ ] `repo: repo-a` matches an inventory key.

## Notes

Reproducible fixture: `tests/fixtures/multirepo/tiny-multi-repo-top-level`.
