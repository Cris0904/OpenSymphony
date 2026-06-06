/**
 * Shared navigation types for the application.
 *
 * Single source of truth for the Page union type used across
 * AppShell, CommandPalette, ProjectSidebar, Dashboard, RunDetail,
 * and TaskGraph.
 */

export type Page =
  | { kind: "dashboard" }
  | { kind: "project"; projectId: string }
  | { kind: "task-graph"; projectId: string }
  | { kind: "run"; runId: string };

export function pageToRoute(page: Page): string {
  switch (page.kind) {
    case "dashboard":
      return "#/dashboard";
    case "project":
      return `#/project/${page.projectId}`;
    case "task-graph":
      return `#/project/${page.projectId}/graph`;
    case "run":
      return `#/run/${page.runId}`;
  }
}

export function routeToPage(hash: string): Page | null {
  if (!hash || hash === "#/dashboard" || hash === "#/" || hash === "#") {
    return { kind: "dashboard" };
  }
  const match = hash.match(/^#\/project\/([^/]+)$/);
  if (match) return { kind: "project", projectId: match[1] };
  const graphMatch = hash.match(/^#\/project\/([^/]+)\/graph$/);
  if (graphMatch) return { kind: "task-graph", projectId: graphMatch[1] };
  const runMatch = hash.match(/^#\/run\/([^/]+)$/);
  if (runMatch) return { kind: "run", runId: runMatch[1] };
  return null;
}
