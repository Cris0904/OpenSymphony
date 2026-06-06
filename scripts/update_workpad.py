#!/usr/bin/env python3
"""Update Linear workpad comment."""
import json, subprocess, os

COMMENT_ID = "643fc153-b4c2-483e-9ffa-db992ced32df"

body = """## Agent Harness Workpad

```text
Mac.NL-WR8103:/Users/magos/.opensymphony/workspaces/COE-402@fe8e684
```

### Plan

- [x] 1. Extract shared types and utilities
- [x] 2. Wire up focus manager keyboard shortcuts
- [x] 3. Add expand/collapse to TaskGraph TreeNode
- [x] 4. Add component render tests with @testing-library/react
  - [x] 4.1 Create component-render.test.tsx with JSX support (57 tests)
  - [x] 4.2 Configure Jest with ts-jest JSX transform and jsdom environment
  - [x] 4.3 Create tsconfig.test.json for test type checking with JSX
- [x] 5. Fix ProjectSidebar navigation to propagate project context
  - [x] 5.1 Replace hardcoded projectContext fallbacks with parentProjectId propagation
  - [x] 5.2 Recursive SidebarTreeNode resolves project ID from hierarchy
- [x] 6. Stabilize React callbacks (useCallback for closePalette)
- [x] 7. Fix SSR safety in TaskGraph (DOM mutation in useEffect)
- [x] 8. Address all AI review comments inline

### Acceptance Criteria

- [x] Users can navigate from project to milestone to issue to sub-issue to run detail.
- [x] Reconnecting and stale states are visible in dashboard and detail views.
- [x] Task graph views use Linear milestone, issue, and sub-issue nomenclature.
- [x] Dashboard and run detail distinguish quiet/degraded active work from stalled, retry-queued, and detached runs.
- [x] Active runs display last progress and stream-health context when provided by the gateway.
- [x] Retry queue views make detached or active-underlying-harness state explicit when present.

### Validation

- [x] All 150 tests pass (5 test suites): `npm test`
- [x] TypeScript compilation passes: `npm run type-check`
- [x] Vite build succeeds (242KB JS bundle): `npm run build --workspace=@opensymphony/web`
- [x] Component rendering tests cover ProjectSidebar, Dashboard, CommandPalette, TaskGraph
- [x] ProjectSidebar navigation uses resolved project context, no hardcoded fallbacks
- [x] AI review comments addressed and replied to inline in PR threads

### Notes

- 2026-06-06 02:42Z: State transition: Todo -> In Progress, created workpad
- 2026-06-06 02:56Z: Pull skill: merged origin/main clean, HEAD at 7033d57
- 2026-06-06 03:00Z: All 7 AI review comments addressed and replied to inline
- 2026-06-06 03:06Z: Pushed commit 72ceacb, all 129 tests green, type-check passes
- 2026-06-06 03:15Z: Fixed unstable onClose callback using useCallback in AppShell
- 2026-06-06 03:22Z: Fixed all 4 AI review structural issues, committed 6dea1f4
- 2026-06-06 03:30Z: Fixed TaskGraph SSR crash by moving DOM mutation into useEffect
- 2026-06-06 03:45Z: Added @testing-library/react component rendering tests (57 tests)
- 2026-06-06 03:46Z: Fixed ProjectSidebar to propagate project context through recursive SidebarTreeNode
- 2026-06-06 03:47Z: Created tsconfig.test.json for JSX type checking in tests
- 2026-06-06 03:48Z: All 150 tests pass, TypeScript clean, build succeeds, pushed fe8e684

### Confusions

- AI review cycle takes significant time between pushes and feedback
- Some review comments reference deleted files (component-render.test.ts renamed to .tsx)"""

vars = {"id": COMMENT_ID, "body": body}
vars_file = "/tmp/linear_vars_update.json"
with open(vars_file, "w") as f:
    json.dump(vars, f)

cmd = [
    "python3",
    ".agents/skills/linear/scripts/linear_graphql.py",
    "--query-file", ".agents/skills/linear/queries/comment_update.graphql",
    "--variables-file", vars_file
]
result = subprocess.run(cmd, capture_output=True, text=True, cwd="/Users/magos/.opensymphony/workspaces/COE-402")
print("STDOUT:", result.stdout[:300] if result.stdout else "(empty)")
if result.stderr:
    print("STDERR:", result.stderr[:300])
print("Return code:", result.returncode)
