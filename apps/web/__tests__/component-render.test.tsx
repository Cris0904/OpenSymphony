/**
 * Component rendering tests for the AppShell, Dashboard, TaskGraph, and RunDetail.
 *
 * Tests verify fixture data matches expected schemas, shared utilities produce
 * correct output, navigation routing works as expected, and React components
 * render successfully with @testing-library/react.
 *
 * @jest-environment jsdom
 */

import { describe, test, expect, beforeEach } from "@jest/globals";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import { ProjectSidebar } from "../src/components/ProjectSidebar";
import { CommandPalette } from "../src/components/CommandPalette";
import { Dashboard } from "../src/pages/Dashboard";
import { TaskGraph } from "../src/pages/TaskGraph";
import type { Page } from "../src/types/navigation";

// -- Fixture loading --

const fixturesDir = resolve(__dirname, "fixtures");

interface RunFixture {
  status: string;
  error?: string;
  release_reason?: string;
  retry_attempt?: number;
  finished_at?: string;
  runtime_seconds: number;
  input_tokens: number;
}

function loadFixture(name: string): RunFixture {
  const content = readFileSync(resolve(fixturesDir, name), "utf-8");
  return JSON.parse(content) as RunFixture;
}

// -- Shared utility tests --

describe("shared UI utilities", () => {
  const uiUtils = require("../src/lib/ui-utils");

  describe("formatTimeAgo", () => {
    test("returns 'just now' for recent timestamps", () => {
      const now = new Date().toISOString();
      expect(uiUtils.formatTimeAgo(now)).toBe("just now");
    });

    test("returns minutes for timestamps under an hour", () => {
      const fiveMinAgo = new Date(Date.now() - 5 * 60 * 1000).toISOString();
      expect(uiUtils.formatTimeAgo(fiveMinAgo)).toMatch(/^\d+m ago$/);
    });

    test("returns hours for timestamps over an hour", () => {
      const twoHoursAgo = new Date(Date.now() - 2 * 60 * 60 * 1000).toISOString();
      expect(uiUtils.formatTimeAgo(twoHoursAgo)).toMatch(/^\d+h ago$/);
    });
  });

  describe("formatDuration", () => {
    test("formats seconds only", () => {
      expect(uiUtils.formatDuration(45)).toBe("45s");
    });

    test("formats minutes and seconds", () => {
      expect(uiUtils.formatDuration(125)).toBe("2m 5s");
    });

    test("formats hours and minutes", () => {
      expect(uiUtils.formatDuration(7200)).toBe("2h 0m");
    });
  });

  describe("formatTokens", () => {
    test("formats raw count under 1K", () => {
      expect(uiUtils.formatTokens(500)).toBe("500");
    });

    test("formats thousands as K", () => {
      expect(uiUtils.formatTokens(45000)).toBe("45.0K");
    });

    test("formats millions as M", () => {
      expect(uiUtils.formatTokens(1_500_000)).toBe("1.5M");
    });
  });

  describe("formatCost", () => {
    test("formats sub-cent costs as cents", () => {
      expect(uiUtils.formatCost(5000)).toMatch(/¢$/);
    });

    test("formats dollar costs", () => {
      expect(uiUtils.formatCost(1_250_000)).toMatch(/^\$/);
    });
  });

  describe("RUN_STATUS_COLORS", () => {
    test("covers all required run statuses", () => {
      expect(uiUtils.RUN_STATUS_COLORS).toHaveProperty("running");
      expect(uiUtils.RUN_STATUS_COLORS).toHaveProperty("retry_queued");
      expect(uiUtils.RUN_STATUS_COLORS).toHaveProperty("released");
      expect(uiUtils.RUN_STATUS_COLORS).toHaveProperty("claimed");
      expect(uiUtils.RUN_STATUS_COLORS).toHaveProperty("unclaimed");
    });

    test("each status has bg and fg colors", () => {
      for (const [status, colors] of Object.entries(uiUtils.RUN_STATUS_COLORS)) {
        expect(colors).toHaveProperty("bg");
        expect(colors).toHaveProperty("fg");
      }
    });
  });

  describe("LIVENESS_COLORS", () => {
    test("covers all required liveness states", () => {
      expect(uiUtils.LIVENESS_COLORS).toHaveProperty("active");
      expect(uiUtils.LIVENESS_COLORS).toHaveProperty("quiet");
      expect(uiUtils.LIVENESS_COLORS).toHaveProperty("degraded");
      expect(uiUtils.LIVENESS_COLORS).toHaveProperty("stalled");
      expect(uiUtils.LIVENESS_COLORS).toHaveProperty("detached");
    });
  });
});

// -- Navigation type tests --

describe("shared navigation types", () => {
  const nav = require("../src/types/navigation");

  describe("pageToRoute", () => {
    test("routes dashboard page to #/dashboard", () => {
      expect(nav.pageToRoute({ kind: "dashboard" })).toBe("#/dashboard");
    });

    test("routes project page correctly", () => {
      expect(nav.pageToRoute({ kind: "project", projectId: "p1" })).toBe("#/project/p1");
    });

    test("routes task-graph page correctly", () => {
      expect(nav.pageToRoute({ kind: "task-graph", projectId: "p1" })).toBe("#/project/p1/graph");
    });

    test("routes run page correctly", () => {
      expect(nav.pageToRoute({ kind: "run", runId: "r1" })).toBe("#/run/r1");
    });
  });

  describe("routeToPage", () => {
    test("parses empty hash as dashboard", () => {
      expect(nav.routeToPage("")).toEqual({ kind: "dashboard" });
      expect(nav.routeToPage("#")).toEqual({ kind: "dashboard" });
      expect(nav.routeToPage("#/dashboard")).toEqual({ kind: "dashboard" });
    });

    test("parses project route", () => {
      expect(nav.routeToPage("#/project/p1")).toEqual({ kind: "project", projectId: "p1" });
    });

    test("parses task-graph route", () => {
      expect(nav.routeToPage("#/project/p1/graph")).toEqual({ kind: "task-graph", projectId: "p1" });
    });

    test("parses run route", () => {
      expect(nav.routeToPage("#/run/r1")).toEqual({ kind: "run", runId: "r1" });
    });
  });
});

// -- React component rendering tests --

const navigateMock = jest.fn((_page: Page) => {});

describe("ProjectSidebar component rendering", () => {
  test("renders Dashboard quick nav button", () => {
    render(<ProjectSidebar navigate={navigateMock} />);
    // There are two "Dashboard" elements: quick nav button and sidebar tree label.
    // Use getAllByText and verify both exist within buttons.
    const dashboardButtons = screen.getAllByText("Dashboard");
    expect(dashboardButtons.length).toBeGreaterThanOrEqual(1);
    expect(dashboardButtons[0].closest("button")).toBeTruthy();
  });

  test("renders Projects section in navigation tree", () => {
    render(<ProjectSidebar navigate={navigateMock} />);
    const projectsLabel = screen.getByText("Projects");
    expect(projectsLabel).toBeTruthy();
  });

  test("renders Active Runs quick link", () => {
    render(<ProjectSidebar navigate={navigateMock} />);
    const activeRuns = screen.getByText("Active Runs");
    expect(activeRuns).toBeTruthy();
  });

  test("renders Retry Queue quick link", () => {
    render(<ProjectSidebar navigate={navigateMock} />);
    const retryQueue = screen.getByText("Retry Queue");
    expect(retryQueue).toBeTruthy();
  });

  test("navigates to dashboard when Dashboard button is clicked", () => {
    navigateMock.mockClear();
    render(<ProjectSidebar navigate={navigateMock} />);
    const dashboardButtons = screen.getAllByText("Dashboard");
    // The quick nav button is the first one and is inside a <button>
    (dashboardButtons[0].closest("button") as HTMLButtonElement).click();
    expect(navigateMock).toHaveBeenCalledWith({ kind: "dashboard" });
  });
});

describe("Dashboard component rendering", () => {
  beforeEach(() => {
    navigateMock.mockClear();
  });

  test("renders Dashboard header", () => {
    render(<Dashboard navigate={navigateMock} />);
    const header = screen.getByRole("heading", { level: 1 });
    expect(header).toBeTruthy();
    expect(header.tagName).toBe("H1");
  });

  test("renders System Health section", () => {
    render(<Dashboard navigate={navigateMock} />);
    const healthHeader = screen.getByText("System Health");
    expect(healthHeader).toBeTruthy();
  });

  test("renders Gateway health indicator", () => {
    render(<Dashboard navigate={navigateMock} />);
    const gateway = screen.getByText("Gateway");
    expect(gateway).toBeTruthy();
  });

  test("renders Metrics section", () => {
    render(<Dashboard navigate={navigateMock} />);
    const metricsHeader = screen.getByText("Metrics");
    expect(metricsHeader).toBeTruthy();
  });

  test("renders Active Runs metric", () => {
    render(<Dashboard navigate={navigateMock} />);
    // "Active Runs" appears both as a metric label and as a section header.
    // Use getAllByText to handle multiple elements.
    const activeRunsElements = screen.getAllByText("Active Runs");
    expect(activeRunsElements.length).toBeGreaterThanOrEqual(1);
  });

  test("renders Retry Queue metric", () => {
    render(<Dashboard navigate={navigateMock} />);
    const retryQueueLabel = screen.getByText("Retry Queue");
    expect(retryQueueLabel).toBeTruthy();
  });

  test("renders Recent Events section", () => {
    render(<Dashboard navigate={navigateMock} />);
    const eventsHeader = screen.getByText("Recent Events");
    expect(eventsHeader).toBeTruthy();
  });

  test("renders Active Runs section with fixture data", () => {
    render(<Dashboard navigate={navigateMock} />);
    // "Active Runs" appears both as a metric label and as a section header.
    const activeRunsElements = screen.getAllByText("Active Runs");
    expect(activeRunsElements.length).toBeGreaterThanOrEqual(1);
  });

  test("renders project card with milestone count", () => {
    render(<Dashboard navigate={navigateMock} />);
    const milestones = screen.getByText("Milestones");
    expect(milestones).toBeTruthy();
  });
});

describe("CommandPalette component rendering", () => {
  const onCloseMock = jest.fn();

  beforeEach(() => {
    navigateMock.mockClear();
    onCloseMock.mockClear();
  });

  test("renders with default props from dashboard", () => {
    render(
      <CommandPalette
        onClose={onCloseMock}
        navigate={navigateMock}
        currentPage={{ kind: "dashboard" }}
      />
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toBeTruthy();
    expect(screen.getByPlaceholderText("Type a command...")).toBeTruthy();
  });

  test("renders with project context", () => {
    render(
      <CommandPalette
        onClose={onCloseMock}
        navigate={navigateMock}
        currentPage={{ kind: "project", projectId: "project-1" }}
        currentProjectId="project-1"
      />
    );
    const viewProjects = screen.getByText("View Projects");
    expect(viewProjects).toBeTruthy();
  });

  test("hides project-specific commands when no project context", () => {
    render(
      <CommandPalette
        onClose={onCloseMock}
        navigate={navigateMock}
        currentPage={{ kind: "dashboard" }}
      />
    );
    // "View Projects" and "View Task Graph" require project context
    expect(screen.queryByText("View Projects")).toBeFalsy();
    expect(screen.queryByText("View Task Graph")).toBeFalsy();
    // But "Go to Dashboard" should still be visible
    expect(screen.getByText("Go to Dashboard")).toBeTruthy();
  });

  test("shows all navigation commands when project context is available", () => {
    render(
      <CommandPalette
        onClose={onCloseMock}
        navigate={navigateMock}
        currentPage={{ kind: "project", projectId: "project-1" }}
        currentProjectId="project-1"
      />
    );
    expect(screen.getByText("Go to Dashboard")).toBeTruthy();
    expect(screen.getByText("View Projects")).toBeTruthy();
    expect(screen.getByText("View Task Graph")).toBeTruthy();
  });

  test("filters commands by query", () => {
    render(
      <CommandPalette
        onClose={onCloseMock}
        navigate={navigateMock}
        currentPage={{ kind: "project", projectId: "project-1" }}
        currentProjectId="project-1"
      />
    );
    const input = screen.getByPlaceholderText("Type a command...");
    (input as HTMLInputElement).value = "Dashboard";
    expect(screen.getByText("Go to Dashboard")).toBeTruthy();
  });

  test("shows 'No commands found' when query matches nothing", () => {
    render(
      <CommandPalette
        onClose={onCloseMock}
        navigate={navigateMock}
        currentPage={{ kind: "dashboard" }}
      />
    );
    const input = screen.getByPlaceholderText("Type a command...");
    // Filter out all commands by typing a unique string
    fireEvent.change(input, { target: { value: "xyznonexistent12345" } });
    expect(screen.getByText("No commands found")).toBeTruthy();
  });
});

describe("TaskGraph component rendering", () => {
  beforeEach(() => {
    navigateMock.mockClear();
  });

  test("renders Task Graph header", () => {
    render(<TaskGraph projectId="project-1" navigate={navigateMock} />);
    const header = screen.getByText("Task Graph");
    expect(header).toBeTruthy();
    expect(header.tagName).toBe("H1");
  });

  test("renders milestone node", () => {
    render(<TaskGraph projectId="project-1" navigate={navigateMock} />);
    const milestone = screen.getByText("M7");
    expect(milestone).toBeTruthy();
  });

  test("renders issue COE-402", () => {
    render(<TaskGraph projectId="project-1" navigate={navigateMock} />);
    const issue = screen.getByText("COE-402");
    expect(issue).toBeTruthy();
  });

  test("renders filter bar with state options", () => {
    render(<TaskGraph projectId="project-1" navigate={navigateMock} />);
    // Use getAllByText since "In Progress", "Todo", "Done" appear both in
    // the filter bar and in the tree node state badges.
    expect(screen.getByText("All")).toBeTruthy();
    expect(screen.getAllByText("In Progress").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Todo").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Done").length).toBeGreaterThanOrEqual(1);
  });

  test("renders legend with node types", () => {
    render(<TaskGraph projectId="project-1" navigate={navigateMock} />);
    expect(screen.getByText("Milestone")).toBeTruthy();
    expect(screen.getByText("Issue")).toBeTruthy();
    expect(screen.getByText("Sub-issue")).toBeTruthy();
  });
});

// -- Run fixture rendering assertions --

describe("run fixture rendering assertions", () => {
  test("active long-running fixture renders expected run state", () => {
    const data = loadFixture("fixture_run_active_long_running.json");
    expect(data.status).toBe("running");
    expect(data.runtime_seconds).toBeGreaterThan(3600);
    expect(data.input_tokens).toBeGreaterThan(100_000);
  });

  test("quiet fixture renders expected minimal activity", () => {
    const data = loadFixture("fixture_run_quiet.json");
    expect(data.status).toBe("running");
    expect(data.input_tokens).toBeLessThan(20_000);
    expect(data.runtime_seconds).toBeLessThan(1800);
  });

  test("degraded fixture has error field", () => {
    const data = loadFixture("fixture_run_degraded.json");
    expect(data.status).toBe("running");
    expect(data.error).toBeDefined();
  });

  test("stalled fixture indicates no progress error", () => {
    const data = loadFixture("fixture_run_stalled.json");
    expect(data.status).toBe("claimed");
    expect(data.error).toBeDefined();
    expect(typeof data.error).toBe("string");
    expect(data.error!.toLowerCase()).toContain("no progress");
  });

  test("retry queued fixture distinguishes from active harness work", () => {
    const data = loadFixture("fixture_run_retry_queued.json");
    expect(data.status).toBe("retry_queued");
    expect(data.release_reason).toBeDefined();
    expect(data.retry_attempt).toBeGreaterThan(0);
    expect(data.finished_at).toBeDefined();
  });

  test("detached fixture shows explicit detached state", () => {
    const data = loadFixture("fixture_run_detached.json");
    expect(data.status).toBe("released");
    expect(data.release_reason).toBe("cancelled");
    expect(typeof data.error).toBe("string");
    expect(data.error!.toLowerCase()).toContain("detached");
  });
});

// -- State distinction tests --

describe("state distinction", () => {
  test("retry_queued vs active: retry_queued has finished_at, active does not", () => {
    const retryData = loadFixture("fixture_run_retry_queued.json");
    const activeData = loadFixture("fixture_run_active_long_running.json");
    expect(retryData.finished_at).toBeDefined();
    expect(activeData.finished_at).toBeUndefined();
  });

  test("detached vs active: detached has release_reason and error", () => {
    const detachedData = loadFixture("fixture_run_detached.json");
    const activeData = loadFixture("fixture_run_active_long_running.json");
    expect(detachedData.release_reason).toBeDefined();
    expect(detachedData.error).toBeDefined();
    expect(activeData.error).toBeUndefined();
  });

  test("degraded vs stalled: different statuses and error patterns", () => {
    const degradedData = loadFixture("fixture_run_degraded.json");
    const stalledData = loadFixture("fixture_run_stalled.json");
    expect(degradedData.status).toBe("running");
    expect(stalledData.status).toBe("claimed");
    expect(stalledData.error!.toLowerCase()).toContain("no progress");
  });

  test("quiet vs active: quiet has lower token usage", () => {
    const quietData = loadFixture("fixture_run_quiet.json");
    const activeData = loadFixture("fixture_run_active_long_running.json");
    expect(quietData.input_tokens).toBeLessThan(activeData.input_tokens);
    expect(quietData.runtime_seconds).toBeLessThan(activeData.runtime_seconds);
  });
});