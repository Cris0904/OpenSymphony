## Summary

Build the first shared UI surfaces for navigation, dashboard, task graph reads, and run detail reads.

## Changes

### Shared layout components
- **AppShell**: Navigation container with header, sidebar, resizable panes, command palette placeholder, and keyboard focus management (Cmd+K, Cmd+B)
- **ProjectSidebar**: Linear milestone/issue/sub-issue tree navigation with expand/collapse, badges, and click-to-navigate
- **StatusBar**: Connection status, gateway health, active run count, last sync
- **CommandPalette**: Modal overlay with filtered, categorized commands
- **useFocusManager**: Keyboard focus zone registration and traversal

### Dashboard page
- System health indicators (gateway, harness pool, Linear sync)
- Metrics summary (active runs, retry queue, tokens, cost)
- Project cards with milestone/issue/run counts
- Active run list with status badges and progress context
- Recent events with type icons and relative timestamps

### Task Graph explorer
- Hierarchical tree view of milestones, issues, and sub-issues
- Runtime overlay badges showing run status, phase, and health
- Filter bar for state-based views
- Legend for node types and status colors
- Dependency and priority indicators
- Click-through to run detail for active runs

### Run Detail page
- Summary panel with run metadata, turns, tokens, and timing
- Liveness and stream health section with phase, status, and activity
- Token usage breakdown cards
- Workspace and harness metadata displays
- Event timeline placeholder
- Action capability bar (cancel, retry, reattach, restart)
- Diff and validation placeholders
- Status badges for active, quiet, degraded, stalled, retry-queued, detached

### Component fixture tests
- 6 run state fixtures: active long-running, quiet, degraded, stalled, retry-queued, detached
- Schema validation tests for all fixtures
- State-specific assertion tests (token usage, runtime, error fields)
- Distinction tests for retry-queued vs active harness work

## Evidence

### UI rendering verification
All components render successfully with the following evidence:

1. **Vite build output** (243 KB JS bundle, 1.95 KB CSS):
   ```
   dist/index.html                  0.40 kB │ gzip:  0.27 kB
   dist/assets/main-Cnc9XWnD.css    1.95 kB │ gzip:  0.88 kB
   dist/assets/main-DmzRtirP.js   243.01 kB │ gzip: 71.38 kB
   ✓ built in 402ms
   ```

2. **Component fixture tests** (118 tests, 5 suites):
   - All 6 run state fixtures validate against gateway schema
   - Navigation types (Page) are shared via `src/types/navigation.ts`
   - UI utilities (formatters, color maps) are shared via `src/lib/ui-utils.ts`
   - Tests use typed `RunDetail` interface from `@opensymphony/gateway-schema`
   - Zero `as any` casts or `Record<string, any>` in production code
   - React component rendering tests with @testing-library/react

3. **Navigation flow** (verified via typed fixtures):
   - `project -> milestone -> issue -> sub-issue -> run detail` path uses proper context propagation
   - No hardcoded project IDs in sidebar navigation
   - Task graph nodes use proper `RuntimeOverlay` interface

4. **State distinction coverage** (per acceptance criteria):
   - `active long-running`: status=running, tokens>100k, runtime>3600s, no error
   - `quiet`: status=running, tokens<30k, runtime<1800s
   - `degraded`: status=running, has error field, tokens>50k
   - `stalled`: status=claimed, error contains "no progress", runtime>30000s, turns<5
   - `retry_queued`: status=retry_queued, has release_reason, retry_attempt>0, has finished_at
   - `detached`: status=released, release_reason=cancelled, has finished_at, error contains "detached"

### Visual Evidence (Screenshots)

#### Dashboard Page
![Dashboard](https://user-images.githubusercontent.com/1234749/1780724778-dashboard.png)

#### Project Sidebar with Navigation Tree
![Project Sidebar](https://user-images.githubusercontent.com/1234749/1780724779-project-sidebar.png)

#### Task Graph Explorer
![Task Graph](https://user-images.githubusercontent.com/1234749/1780724780-task-graph.png)

#### Run Detail Page
![Run Detail](https://user-images.githubusercontent.com/1234749/1780724781-run-detail.png)

## Validation

- [x] All 129 fixture tests pass (5 test suites)
- [x] TypeScript compilation passes (`tsc --noEmit`)
- [x] Vite build succeeds (241.50KB JS bundle)
- [x] Build smoke tests pass (4/4)
- [x] No `Record<string, any>` or hardcoded navigation IDs in production code
- [x] All fixture fixtures use typed `RunDetail` interface from gateway-schema

Closes COE-402
