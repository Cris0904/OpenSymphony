/**
 * CommandPalette component.
 *
 * Placeholder for command palette functionality. Shows a modal overlay
 * with quick navigation and action commands.
 */

import { useState, useEffect, useRef } from "react";
import type { Page } from "../types/navigation";

interface CommandPaletteProps {
  onClose: () => void;
  navigate: (page: Page) => void;
  currentPage: Page;
  currentProjectId?: string;
}

interface Command {
  id: string;
  label: string;
  shortcut?: string;
  action: () => void;
  category: string;
  requiresProject?: boolean;
}

export function CommandPalette({
  onClose,
  navigate,
  currentProjectId,
}: CommandPaletteProps): React.ReactElement {
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-focus input on mount.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Close on Escape.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  // Define available commands.
  const commands: Command[] = [
    {
      id: "dashboard",
      label: "Go to Dashboard",
      shortcut: "G D",
      action: () => { navigate({ kind: "dashboard" }); onClose(); },
      category: "Navigation",
    },
    {
      id: "projects",
      label: "View Projects",
      shortcut: "G P",
      action: () => { navigate({ kind: "project", projectId: currentProjectId ?? "all" }); onClose(); },
      category: "Navigation",
      requiresProject: true,
    },
    {
      id: "task-graph",
      label: "View Task Graph",
      shortcut: "G T",
      action: () => { navigate({ kind: "task-graph", projectId: currentProjectId ?? "all" }); onClose(); },
      category: "Navigation",
      requiresProject: true,
    },
    {
      id: "active-runs",
      label: "Show Active Runs",
      action: () => { /* TODO: filter to active runs */ onClose(); },
      category: "Views",
    },
    {
      id: "retry-queue",
      label: "Show Retry Queue",
      action: () => { /* TODO: show retry queue */ onClose(); },
      category: "Views",
    },
    {
      id: "toggle-theme",
      label: "Toggle Theme",
      shortcut: "⌘ T",
      action: () => { /* TODO: theme toggle */ onClose(); },
      category: "Settings",
    },
  ];

  // Filter commands by query and project availability.
  const filtered = commands.filter(
    (cmd) =>
      (!cmd.requiresProject || currentProjectId !== undefined) &&
      (query === "" ||
      cmd.label.toLowerCase().includes(query.toLowerCase()) ||
      cmd.category.toLowerCase().includes(query.toLowerCase())),
  );

  // Group by category.
  const grouped = filtered.reduce<Record<string, Command[]>>((acc, cmd) => {
    if (!acc[cmd.category]) acc[cmd.category] = [];
    acc[cmd.category].push(cmd);
    return acc;
  }, {});

  return (
    <div
      style={{
        position: "fixed",
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        background: "rgba(0, 0, 0, 0.5)",
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "center",
        paddingTop: "20vh",
        zIndex: 1000,
      }}
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="Command Palette"
    >
      <div
        style={{
          background: "var(--color-bg-secondary)",
          border: "1px solid var(--color-border-default)",
          borderRadius: "var(--radius-lg)",
          width: "min(600px, 90vw)",
          maxHeight: "60vh",
          display: "flex",
          flexDirection: "column",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.4)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Search input */}
        <div style={{ padding: "var(--space-3)", borderBottom: "1px solid var(--color-border-default)" }}>
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Type a command..."
            style={{
              width: "100%",
              background: "transparent",
              border: "none",
              color: "var(--color-fg-default)",
              fontSize: "14px",
              outline: "none",
            }}
          />
        </div>

        {/* Command list */}
        <div style={{ overflow: "auto", padding: "var(--space-2)" }}>
          {Object.entries(grouped).map(([category, cmds]) => (
            <div key={category} style={{ marginBottom: "var(--space-2)" }}>
              <div
                style={{
                  fontSize: "11px",
                  color: "var(--color-fg-subtle)",
                  textTransform: "uppercase",
                  padding: "var(--space-1) var(--space-2)",
                  letterSpacing: "0.05em",
                }}
              >
                {category}
              </div>
              {cmds.map((cmd) => (
                <button
                  key={cmd.id}
                  onClick={cmd.action}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    width: "100%",
                    background: "transparent",
                    border: "none",
                    color: "var(--color-fg-default)",
                    padding: "var(--space-2) var(--space-3)",
                    borderRadius: "var(--radius-md)",
                    cursor: "pointer",
                    textAlign: "left",
                    fontSize: "13px",
                  }}
                  onMouseEnter={(e) =>
                    (e.currentTarget.style.background = "var(--color-bg-tertiary)")
                  }
                  onMouseLeave={(e) =>
                    (e.currentTarget.style.background = "transparent")
                  }
                  tabIndex={0}
                >
                  <span>{cmd.label}</span>
                  {cmd.shortcut && (
                    <kbd
                      style={{
                        background: "var(--color-bg-tertiary)",
                        border: "1px solid var(--color-border-default)",
                        borderRadius: "var(--radius-sm)",
                        padding: "2px 6px",
                        fontSize: "11px",
                        color: "var(--color-fg-muted)",
                        fontFamily: "var(--font-mono)",
                      }}
                    >
                      {cmd.shortcut}
                    </kbd>
                  )}
                </button>
              ))}
            </div>
          ))}
          {filtered.length === 0 && (
            <div
              style={{
                padding: "var(--space-4)",
                textAlign: "center",
                color: "var(--color-fg-subtle)",
              }}
            >
              No commands found
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
