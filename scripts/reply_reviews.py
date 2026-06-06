#!/usr/bin/env python3
"""Reply to inline review comments on PR #104."""

import json, subprocess

comments_to_reply = {
    3366564102: {
        "path": "apps/web/src/components/AppShell.tsx",
        "line": 228,
        "body": "Fixed in a547f35: Replaced unstable inline arrow function with useCallback-stabilized closePalette handler. The keydown listener now uses a stable reference and is only cleaned up on unmount."
    },
    3366564106: {
        "path": "apps/web/src/components/AppShell.tsx",
        "line": 239,
        "body": "Fixed in a547f35: getCurrentProjectId now returns string | undefined instead of hardcoded 'all'. CommandPalette filters project-scoped commands when currentProjectId is undefined, and navigation actions use nullish coalescing (?? 'all') as explicit fallback."
    },
    3366578473: {
        "path": "apps/web/src/components/CommandPalette.tsx",
        "line": 35,
        "body": "Fixed in fd7d2f1: Removed hardcoded default parameter currentProjectId='all'. The value now flows entirely from AppShell's getCurrentProjectId which returns undefined when no project context exists."
    },
    3366578474: {
        "path": "apps/web/src/components/ProjectSidebar.tsx",
        "line": 193,
        "body": "Pushback: This is intentional scaffolding for a single-project alpha. Added TODO comment (line 185) documenting that the fallback must be replaced with real parent-hierarchy project ID extraction when multi-project support is added. This is out of scope for the current milestone."
    },
    3366524323: {
        "path": "apps/web/__tests__/component-render.test.ts",
        "line": 6,
        "body": "Pushback: @testing-library/react is already installed in the workspace (129 tests pass including fixture validation). Full React component rendering is deferred because the repo uses testEnvironment: 'node' which is incompatible with JSX transformation. Adding React rendering tests requires significant Jest/Vite reconfiguration which is out of scope for this scaffolding milestone. A follow-up issue will be filed to address component rendering tests."
    },
    3366564101: {
        "path": "apps/web/__tests__/component-render.test.ts",
        "line": 6,
        "body": "See reply to #3366524323 above."
    },
    3366578472: {
        "path": "apps/web/__tests__/component-render.test.ts",
        "line": 6,
        "body": "See reply to #3366524323 above."
    },
    3366541665: {
        "path": "apps/web/__tests__/component-render.test.ts",
        "line": 6,
        "body": "See reply to #3366524323 above."
    },
    3366551687: {
        "path": "apps/web/__tests__/component-render.test.ts",
        "line": 6,
        "body": "See reply to #3366524323 above."
    },
    3366536053: {
        "path": "apps/web/__tests__/component-render.test.ts",
        "line": 6,
        "body": "See reply to #3366524323 above."
    },
}

for comment_id, reply_data in comments_to_reply.items():
    # Escape body for shell
    body = reply_data['body'].replace('\\', '\\\\').replace('"', '\\"')
    cmd = f'gh api repos/kumanday/OpenSymphony/pulls/104/comments/{comment_id}/replies -X POST -f "body={body}"'
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True, cwd="/Users/magos/.opensymphony/workspaces/COE-402")
    status = "OK" if result.returncode == 0 else f"FAILED: {result.stderr[:100]}"
    print(f"Reply to #{comment_id}: {status}")
