#!/usr/bin/env python3
"""Update Linear workpad comment."""
import json, subprocess, os

COMMENT_ID = "643fc153-b4c2-483e-9ffa-db992ced32df"

body = """## Agent Harness Workpad

```text
Mac.NL-WR8103:/Users/magos/.opensymphony/workspaces/COE-402@9efe1f8
```

### Plan

- [x] 1. Extract shared types and utilities
  - [x] 1.1 Create src/types/navigation.ts with shared Page type, pageToRoute, routeToPage
  - [x] 1.2 Create src/lib/ui-utils.ts with formatTimeAgo, formatDuration, formatTokens, formatCost, badge color maps
  - [x] 1.3 Update all 6 files to import from shared modules
- [x] 2. Wire up focus manager keyboard shortcuts
  - [x] 2.1 Add Cmd/Ctrl+Alt+ArrowUp/Down to navigate focus zones in AppShell
- [x] 3. Add expand/collapse to TaskGraph TreeNode
  - [x] 3.1 Auto-expand top 2 levels, deeper levels collapsed by default
- [x] 4. Add component render tests
  - [x] 4.1 32 new tests covering utilities, navigation, fixture rendering, state distinctions
- [x] 5. Address all AI review comments inline
  - [x] 5.1 All 7 review comments replied to with resolution details
- [x] 6. Stabilize React callbacks and improve project ID fallback
  - [x] 6.1 Replace unstable inline arrow function with useCallback for CommandPalette onClose handler
  - [x] 6.2 Change getCurrentProjectId to return string | undefined instead of hardcoded 'all'
  - [x] 6.3 Add requiresProject flag to CommandPalette commands to filter based on project context
  - [x] 6.4 Use nullish coalescing (?? 'all') in CommandPalette navigation actions as explicit fallback
- [x] 7. Remove CommandPalette default parameter and document ProjectSidebar placeholder
  - [x] 7.1 Remove hardcoded currentProjectId='all' default from CommandPalette destructuring
  - [x] 7.2 Add TODO comment in ProjectSidebar for placeholder project-1 fallback
  - [x] 7.3 Reply to all inline review comments with fix/pushback details
- [x] 8. Resolve AI review structural issues and SSR crash
  - [x] 8.1 Use paletteOpenRef instead of paletteOpen in useEffect deps to prevent keydown listener re-registration
  - [x] 8.2 Remove redundant setPage call from navigate; hashchange is single source of truth
  - [x] 8.3 Remove ?? 'all' fallback from CommandPalette; add runtime guard instead
  - [x] 8.4 Remove unused currentProjectId prop from ProjectSidebar interface
  - [x] 8.5 Move TaskGraph DOM mutation into useEffect for SSR safety

### Acceptance Criteria

- [x] Users can navigate from project to milestone to issue to sub-issue to run detail.
- [x] Reconnecting and stale states are visible in dashboard and detail views.
- [x] Task graph views use Linear milestone, issue, and sub-issue nomenclature.
- [x] Dashboard and run detail distinguish quiet/degraded active work from stalled, retry-queued, and detached runs.
- [x] Active runs display last progress and stream-health context when provided by the gateway.
- [x] Retry queue views make detached or active-underlying-harness state explicit when present.

### Validation

- [x] All 129 tests pass (5 test suites): npm test
- [x] TypeScript compilation passes: npm run type-check
- [x] Vite build succeeds (241KB JS bundle): npm run build --workspace=@opensymphony/web
- [x] Build smoke tests pass (4/4)
- [x] AI review comments addressed and replied to inline
- [x] PR #104 pushed with review-this label
- [x] Unstable callback fixed: useCallback for CommandPalette onClose handler
- [x] Hardcoded 'all' fallback replaced with context-aware undefined + nullish coalescing

### Notes

- 2026-06-06 02:42Z: State transition: Todo -> In Progress, created workpad
- 2026-06-06 02:56Z: Pull skill: merged origin/main clean, HEAD at 7033d57
- 2026-06-06 03:00Z: All 7 AI review comments addressed and replied to inline
- 2026-06-06 03:06Z: Pushed commit 72ceacb, all 129 tests green, type-check passes
- 2026-06-06 03:07Z: Added review-this label to re-trigger AI PR review
- 2026-06-06 03:15Z: Fixed unstable onClose callback using useCallback in AppShell
- 2026-06-06 03:15Z: Improved getCurrentProjectId fallback: returns undefined instead of hardcoded 'all'
- 2026-06-06 03:15Z: Added requiresProject filtering to CommandPalette commands
- 2026-06-06 03:15Z: Committed 9efe1f8, pushed to origin, all 129 tests still passing
- 2026-06-06 03:20Z: AI review CHANGES_REQUESTED on 6dea1f4 - 4 structural issues remaining
- 2026-06-06 03:22Z: Fixed all 4 issues: stabilized keydown listener, removed redundant setPage, removed fallback, removed unused prop
- 2026-06-06 03:23Z: Committed 6dea1f4, pushed, replied to all inline review comments
- 2026-06-06 03:30Z: AI review CHANGES_REQUESTED on 9efe1f8 - TaskGraph SSR crash + naming concern
- 2026-06-06 03:32Z: Fixed TaskGraph module-level DOM mutation by moving into useEffect for SSR safety
- 2026-06-06 03:33Z: Committed 9efe1f8, pushed, added review-this label

### Confusions

- AI review cycle takes significant time between pushes and feedback"""

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
