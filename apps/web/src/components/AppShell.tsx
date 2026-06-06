/**
 * AppShell component.
 *
 * Root layout container providing navigation shell, project sidebar,
 * resizable panes, command palette placeholder, connection status bar,
 * and keyboard focus model.
 */

import { useState, useCallback, useEffect, useRef } from "react";
import { ProjectSidebar } from "./ProjectSidebar";
import { StatusBar } from "./StatusBar";
import { CommandPalette } from "./CommandPalette";
import { Dashboard } from "../pages/Dashboard";
import { TaskGraph } from "../pages/TaskGraph";
import { RunDetail } from "../pages/RunDetail";
import { useFocusManager } from "../hooks/useFocusManager";
import type { Page } from "../types/navigation";
import { routeToPage, pageToRoute } from "../types/navigation";

function renderPage(page: Page, navigate: (page: Page) => void): React.ReactNode {
  switch (page.kind) {
    case "dashboard":
      return <Dashboard navigate={navigate} />;
    case "project":
      return <TaskGraph projectId={page.projectId} navigate={navigate} />;
    case "task-graph":
      return <TaskGraph projectId={page.projectId} navigate={navigate} />;
    case "run":
      return <RunDetail runId={page.runId} navigate={navigate} />;
  }
}

export function AppShell(): React.ReactElement {
  const [page, setPage] = useState<Page>({ kind: "dashboard" });
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(280);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const resizing = useRef(false);
  const { registerZone, focusNext, focusPrev } = useFocusManager();

  // Listen for hash changes.
  useEffect(() => {
    const onHashChange = () => {
      const hash = window.location.hash;
      const p = routeToPage(hash);
      if (p) setPage(p);
    };
    window.addEventListener("hashchange", onHashChange);
    onHashChange();
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  // Sync hash with page state.
  const navigate = useCallback((nextPage: Page) => {
    window.location.hash = pageToRoute(nextPage);
    setPage(nextPage);
  }, []);

  // Keyboard shortcuts.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Cmd/Ctrl+K opens command palette.
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setPaletteOpen((prev) => !prev);
      }
      // Escape closes palette.
      if (e.key === "Escape" && paletteOpen) {
        setPaletteOpen(false);
      }
      // Cmd/Ctrl+B toggles sidebar.
      if ((e.metaKey || e.ctrlKey) && e.key === "b") {
        e.preventDefault();
        setSidebarOpen((prev) => !prev);
      }
      // Cmd/Ctrl+Alt+Arrow navigates focus zones.
      if ((e.metaKey || e.ctrlKey) && e.altKey && e.key === "ArrowDown") {
        e.preventDefault();
        focusNext();
      }
      if ((e.metaKey || e.ctrlKey) && e.altKey && e.key === "ArrowUp") {
        e.preventDefault();
        focusPrev();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [paletteOpen, focusNext, focusPrev]);

  // Sidebar resize handlers.
  const startResize = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    resizing.current = true;
    const startX = e.clientX;
    const startWidth = sidebarWidth;

    const onMouseMove = (ev: MouseEvent) => {
      if (!resizing.current) return;
      const delta = ev.clientX - startX;
      const newWidth = Math.max(200, Math.min(500, startWidth + delta));
      setSidebarWidth(newWidth);
    };

    const onMouseUp = () => {
      resizing.current = false;
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  }, [sidebarWidth]);

  // Register focus zones.
  useEffect(() => {
    const sidebar = registerZone("sidebar");
    const main = registerZone("main");
    return () => {
      sidebar.cleanup();
      main.cleanup();
    };
  }, [registerZone]);

  return (
    <div
      className="app-shell"
      style={{ display: "flex", flexDirection: "column", height: "100%" }}
    >
      {/* Header */}
      <header
        style={{
          height: "var(--header-height)",
          display: "flex",
          alignItems: "center",
          padding: "0 var(--space-4)",
          borderBottom: "1px solid var(--color-border-default)",
          background: "var(--color-bg-secondary)",
          flexShrink: 0,
        }}
      >
        <button
          onClick={() => setSidebarOpen((prev) => !prev)}
          style={{
            background: "none",
            border: "none",
            color: "var(--color-fg-default)",
            cursor: "pointer",
            padding: "var(--space-2)",
            marginRight: "var(--space-3)",
            borderRadius: "var(--radius-md)",
          }}
          aria-label="Toggle sidebar"
          tabIndex={0}
        >
          ☰
        </button>
        <Breadcrumbs page={page} navigate={navigate} />
        <div style={{ flex: 1 }} />
        <button
          onClick={() => setPaletteOpen(true)}
          style={{
            background: "var(--color-bg-tertiary)",
            border: "1px solid var(--color-border-default)",
            color: "var(--color-fg-muted)",
            cursor: "pointer",
            padding: "var(--space-1) var(--space-3)",
            borderRadius: "var(--radius-md)",
            fontSize: "12px",
          }}
          aria-label="Open command palette"
          tabIndex={0}
        >
          ⌘K
        </button>
      </header>

      {/* Body */}
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        {/* Sidebar */}
        {sidebarOpen && (
          <>
            <aside
              data-focus-zone="sidebar"
              style={{
                width: sidebarWidth,
                minWidth: sidebarWidth,
                borderRight: "1px solid var(--color-border-default)",
                background: "var(--color-bg-secondary)",
                overflow: "auto",
              }}
            >
              <ProjectSidebar navigate={navigate} />
            </aside>
            {/* Resize handle */}
            <div
              onMouseDown={startResize}
              style={{
                width: "4px",
                cursor: "col-resize",
                background: "transparent",
                transition: "background 0.15s",
                flexShrink: 0,
              }}
              onMouseEnter={(e) => (e.currentTarget.style.background = "var(--color-accent)")}
              onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
              role="separator"
              aria-orientation="vertical"
              tabIndex={0}
            />
          </>
        )}

        {/* Main content */}
        <main
          data-focus-zone="main"
          style={{
            flex: 1,
            overflow: "auto",
            padding: "var(--space-4)",
          }}
        >
          {renderPage(page, navigate)}
        </main>
      </div>

      {/* Status bar */}
      <StatusBar />

      {/* Command palette */}
      {paletteOpen && (
        <CommandPalette
          onClose={() => setPaletteOpen(false)}
          navigate={navigate}
          currentPage={page}
          currentProjectId={getCurrentProjectId(page)}
        />
      )}
    </div>
  );
}

/** Extract current project ID from page state for navigation. */
function getCurrentProjectId(page: Page): string {
  if (page.kind === "project" || page.kind === "task-graph") {
    return page.projectId;
  }
  return "all";
}

/** Breadcrumb navigation showing current location. */
function Breadcrumbs({
  page,
  navigate,
}: {
  page: Page;
  navigate: (p: Page) => void;
}): React.ReactElement {
  const items: { label: string; page: Page }[] = [{ label: "Dashboard", page: { kind: "dashboard" } }];

  if (page.kind === "project" || page.kind === "task-graph") {
    items.push({ label: "Project", page: { kind: "project", projectId: page.projectId } });
    if (page.kind === "task-graph") {
      items.push({ label: "Task Graph", page });
    }
  } else if (page.kind === "run") {
    items.push({ label: `Run ${page.runId.slice(0, 8)}`, page });
  }

  return (
    <nav
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-2)",
        fontSize: "14px",
      }}
      aria-label="Breadcrumb"
    >
      {items.map((item, idx) => (
        <span key={idx} style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
          {idx > 0 && <span style={{ color: "var(--color-fg-subtle)" }}>/</span>}
          <button
            onClick={() => navigate(item.page)}
            style={{
              background: "none",
              border: "none",
              color: idx === items.length - 1 ? "var(--color-fg-default)" : "var(--color-fg-muted)",
              cursor: "pointer",
              padding: 0,
              fontWeight: idx === items.length - 1 ? 600 : 400,
            }}
            tabIndex={0}
          >
            {item.label}
          </button>
        </span>
      ))}
    </nav>
  );
}
