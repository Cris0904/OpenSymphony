---
id: TASK-PARENT
title: Multi-Repo Sub-Issue Parent
milestone: "M14: Multi-Repo Phase 1"
priority: 3
estimate: 5
blockedBy: []
blocks: ["TASK-SUB-A", "TASK-SUB-B"]
areas:
  - planning
parent: null
---

## Summary

Parent task for the LOC-29 sub-issue multi-repo variant. Owns two
sub-issue leaves that carry different `repo:` slugs.

## Scope

### In scope

- Carry the parent-with-sub-issues shape.
- Demonstrate the parent-without-repo contract.

### Out of scope

- Repo routing — lives on the sub-issue leaves.

## Deliverables

- One top-level parent task with no `repo:` value.

## Acceptance Criteria

- [ ] The manifest validator accepts this parent as in-contract.
- [ ] The sub-issue leaves are accepted as in-contract.

## Test Plan

- `cargo test --lib opensymphony_planning::graph_validate::manifest`
  on the tiny multi-repo sub-issue fixture.

## Context

- `.agents/skills/create-implementation-plan/SKILL.md`
- `crates/opensymphony-planning/src/graph_validate/manifest.rs`

## Definition of Ready

- [ ] `repo:` intentionally omitted.
- [ ] `parent: null` because this task is the parent.

## Notes

Reproducible fixture: `tests/fixtures/multirepo/tiny-multi-repo-sub-issues`.
