/**
 * Fixture loading and shared utility tests for the AppShell, Dashboard, TaskGraph, and RunDetail.
 *
 * Tests verify that fixture data matches expected schemas, shared utilities
 * produce correct output, and navigation routing works as expected.
 * React component rendering tests are in component-render.test.tsx.
 */

import { describe, test, expect } from "@jest/globals";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

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
  // Dynamically import the utilities module.
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
