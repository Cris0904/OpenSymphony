/**
 * Component rendering tests for the AppShell, Dashboard, TaskGraph, and RunDetail.
 *
 * Tests verify React components render correctly with @testing-library/react.
 * Fixture loading, shared utility tests, and navigation type tests are in
 * run-fixtures.test.ts to avoid duplication.
 *
 * @jest-environment jsdom
 */

import { describe, test, expect, beforeEach } from "@jest/globals";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import { ProjectSidebar } from "../src/components/ProjectSidebar";
import { CommandPalette } from "../src/components/CommandPalette";
import { Dashboard } from "../src/pages/Dashboard";
import { TaskGraph } from "../src/pages/TaskGraph";
import type { Page } from "../src/types/navigation";

// -- React component rendering tests --

const navigateMock = jest.fn((_page: Page) => {});

describe("ProjectSidebar component rendering", () => {
  test("renders Dashboard quick nav button", () => {
    render(<ProjectSidebar navigate={navigateMock} />);
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
    expect(screen.queryByText("View Projects")).toBeFalsy();
    expect(screen.queryByText("View Task Graph")).toBeFalsy();
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
    fireEvent.change(input, { target: { value: "Dashboard" } });
    expect(screen.getByText("Go to Dashboard")).toBeTruthy();
  });

  test("shows no commands found when query matches nothing", () => {
    render(
      <CommandPalette
        onClose={onCloseMock}
        navigate={navigateMock}
        currentPage={{ kind: "dashboard" }}
      />
    );
    const input = screen.getByPlaceholderText("Type a command...");
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
