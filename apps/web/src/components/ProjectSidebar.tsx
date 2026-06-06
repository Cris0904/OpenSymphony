/**
 * ProjectSidebar component.
 *
 * Displays project navigation, milestones, issues, and quick links.
 * Uses Linear nomenclature for milestone, issue, and sub-issue hierarchy.
 */

import { useState } from "react";
import type { Page } from "../types/navigation";

interface ProjectSidebarProps {
  navigate: (page: Page) => void;
  currentProjectId?: string;
}

interface SidebarItem {
  id: string;
  label: string;
  icon?: string;
  badge?: { text: string; color: string };
  children?: SidebarItem[];
  action?: () => void;
  // Track which project this item belongs to for navigation context.
  projectContext?: string;
}

// Placeholder data - in production this would come from the gateway API.
const sidebarData: SidebarItem[] = [
  {
    id: "dashboard",
    label: "Dashboard",
    icon: "\ud83d\udcca",
    action: () => {},
  },
  {
    id: "projects",
    label: "Projects",
    icon: "\ud83d\udcc1",
    children: [
      {
        id: "project-1",
        label: "OpenSymphony-bootstrap",
        projectContext: "project-1",
        children: [
          {
            id: "milestone-1",
            label: "M7: Shared Client And Desktop Alpha",
            projectContext: "project-1",
            children: [
              {
                id: "issue-1",
                label: "COE-402 App Shell, Dashboard, Task Graph...",
                badge: { text: "In Progress", color: "var(--color-accent)" },
                projectContext: "project-1",
                children: [
                  {
                    id: "sub-issue-1",
                    label: "COE-402 Sub-task: Layout Components",
                    projectContext: "project-1",
                  },
                  {
                    id: "sub-issue-2",
                    label: "COE-402 Sub-task: Dashboard Page",
                    projectContext: "project-1",
                  },
                ],
              },
              {
                id: "issue-2",
                label: "COE-411 Task Graph Editor and Runtime Overlay",
                badge: { text: "Todo", color: "var(--color-fg-muted)" },
                projectContext: "project-1",
              },
              {
                id: "issue-3",
                label: "COE-414 Diff, Validation, Approval, and Run Action Views",
                badge: { text: "Todo", color: "var(--color-fg-muted)" },
                projectContext: "project-1",
              },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "active-runs",
    label: "Active Runs",
    icon: "\u25b6\ufe0f",
    badge: { text: "3", color: "var(--color-success)" },
  },
  {
    id: "retry-queue",
    label: "Retry Queue",
    icon: "\ud83d\udd04",
    badge: { text: "1", color: "var(--color-attention)" },
  },
];

export function ProjectSidebar({ navigate }: ProjectSidebarProps): React.ReactElement {
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set(["projects", "milestone-1"]));

  const toggleExpand = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div style={{ padding: "var(--space-2)" }}>
      {/* Quick nav */}
      <div style={{ marginBottom: "var(--space-3)" }}>
        <button
          onClick={() => navigate({ kind: "dashboard" })}
          style={{
            width: "100%",
            display: "flex",
            alignItems: "center",
            gap: "var(--space-2)",
            padding: "var(--space-2) var(--space-3)",
            background: "var(--color-bg-tertiary)",
            border: "1px solid var(--color-border-default)",
            borderRadius: "var(--radius-md)",
            color: "var(--color-fg-default)",
            cursor: "pointer",
            fontSize: "13px",
          }}
          tabIndex={0}
        >
          <span>\ud83d\udcca</span>
          <span>Dashboard</span>
        </button>
      </div>

      {/* Navigation tree */}
      <nav aria-label="Project navigation">
        {sidebarData.map((item) => (
          <SidebarTreeNode
            key={item.id}
            item={item}
            depth={0}
            expandedIds={expandedIds}
            onToggle={toggleExpand}
            navigate={navigate}
          />
        ))}
      </nav>
    </div>
  );
}

/** Recursive sidebar tree node. */
function SidebarTreeNode({
  item,
  depth,
  expandedIds,
  onToggle,
  navigate,
}: {
  item: SidebarItem;
  depth: number;
  expandedIds: Set<string>;
  onToggle: (id: string) => void;
  navigate: (page: Page) => void;
}): React.ReactElement | null {
  const isExpanded = expandedIds.has(item.id);
  const hasChildren = item.children && item.children.length > 0;

  const handleClick = () => {
    if (hasChildren) {
      onToggle(item.id);
    }
    if (item.action) {
      item.action();
    }
    // Navigate based on item id.
    if (item.id === "dashboard") {
      navigate({ kind: "dashboard" });
    } else if (item.id.startsWith("project-")) {
      navigate({ kind: "project", projectId: item.id });
    } else if (item.id.startsWith("issue-") || item.id.startsWith("sub-issue-")) {
      // TODO: Extract real project ID from parent hierarchy when multiple projects are supported.
      const projectId = item.projectContext ?? "project-1";
      navigate({ kind: "task-graph", projectId });
    } else if (item.id.startsWith("run-")) {
      navigate({ kind: "run", runId: item.id });
    }
  };

  return (
    <div>
      <button
        onClick={handleClick}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
          width: "100%",
          padding: "var(--space-1) var(--space-2)",
          paddingLeft: `${depth * 12 + 8}px`,
          background: "transparent",
          border: "none",
          color: "var(--color-fg-default)",
          cursor: "pointer",
          textAlign: "left",
          fontSize: "13px",
          borderRadius: "var(--radius-sm)",
          minHeight: "28px",
        }}
        onMouseEnter={(e) =>
          (e.currentTarget.style.background = "var(--color-bg-tertiary)")
        }
        onMouseLeave={(e) =>
          (e.currentTarget.style.background = "transparent")
        }
        tabIndex={0}
      >
        {hasChildren && (
          <span style={{ fontSize: "10px", color: "var(--color-fg-subtle)", width: "12px" }}>
            {isExpanded ? "\u25bc" : "\u25b6"}
          </span>
        )}
        {!hasChildren && <span style={{ width: "12px" }} />}
        {item.icon && <span>{item.icon}</span>}
        <span className="truncate">{item.label}</span>
        {item.badge && (
          <span
            style={{
              marginLeft: "auto",
              fontSize: "10px",
              padding: "1px 6px",
              borderRadius: "10px",
              background: item.badge.color,
              color: "#fff",
              fontWeight: 500,
            }}
          >
            {item.badge.text}
          </span>
        )}
      </button>
      {hasChildren && isExpanded && (
        <div>
          {item.children!.map((child) => (
            <SidebarTreeNode
              key={child.id}
              item={child}
              depth={depth + 1}
              expandedIds={expandedIds}
              onToggle={onToggle}
              navigate={navigate}
            />
          ))}
        </div>
      )}
    </div>
  );
}
