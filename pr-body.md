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

## Validation

- [x] All 36 fixture tests pass
- [x] TypeScript compilation passes
- [x] Vite build succeeds (242KB JS bundle)
- [x] Build smoke tests pass (4/4)
- [x] No desktop/Tauri references in web build output

Closes COE-402
