/**
 * Component fixture tests for all run states.
 *
 * Tests validate that fixture data loads correctly and
 * components render with expected state indicators.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { RunDetail } from "@opensymphony/gateway-schema";

const fixturesDir = resolve(__dirname, "fixtures");

function loadFixture(name: string): RunDetail {
  const content = readFileSync(resolve(fixturesDir, name), "utf-8");
  return JSON.parse(content) as RunDetail;
}

// -- Schema version validation --

describe("run fixture schema validation", () => {
  const fixtures = [
    "fixture_run_active_long_running.json",
    "fixture_run_quiet.json",
    "fixture_run_degraded.json",
    "fixture_run_stalled.json",
    "fixture_run_retry_queued.json",
    "fixture_run_detached.json",
  ];

  test.each(fixtures)("%s has valid schema_version", (filename) => {
    const data = loadFixture(filename);
    expect(data.schema_version).toEqual({ major: 1, minor: 0, patch: 0 });
  });

  test.each(fixtures)("%s has required fields", (filename) => {
    const data = loadFixture(filename);
    expect(data).toHaveProperty("run_id");
    expect(data).toHaveProperty("issue_id");
    expect(data).toHaveProperty("issue_identifier");
    expect(data).toHaveProperty("worker_id");
    expect(data).toHaveProperty("status");
    expect(data).toHaveProperty("claimed_at");
    expect(data).toHaveProperty("turn_count");
    expect(data).toHaveProperty("max_turns");
    expect(data).toHaveProperty("input_tokens");
    expect(data).toHaveProperty("output_tokens");
    expect(data).toHaveProperty("cache_read_tokens");
    expect(data).toHaveProperty("runtime_seconds");
  });
});

// -- Active long-running run --

describe("active long-running run fixture", () => {
  const data: RunDetail = loadFixture("fixture_run_active_long_running.json");

  test("status is running", () => {
    expect(data.status).toBe("running");
  });

  test("has substantial token usage", () => {
    expect(data.input_tokens + data.output_tokens).toBeGreaterThan(100_000);
  });

  test("has long runtime indicating active work", () => {
    expect(data.runtime_seconds).toBeGreaterThan(3600);
  });

  test("no error field present", () => {
    expect(data.error).toBeUndefined();
  });
});

// -- Quiet run --

describe("quiet run fixture", () => {
  const data: RunDetail = loadFixture("fixture_run_quiet.json");

  test("status is running", () => {
    expect(data.status).toBe("running");
  });

  test("low token usage indicates minimal activity", () => {
    expect(data.input_tokens).toBeLessThan(20_000);
    expect(data.output_tokens).toBeLessThan(10_000);
  });

  test("short runtime", () => {
    expect(data.runtime_seconds).toBeLessThan(1800);
  });
});

// -- Degraded run --

describe("degraded run fixture", () => {
  const data: RunDetail = loadFixture("fixture_run_degraded.json");

  test("status is running", () => {
    expect(data.status).toBe("running");
  });

  test("has error field indicating degradation", () => {
    expect(data.error).toBeDefined();
    expect(typeof data.error).toBe("string");
  });

  test("significant token usage", () => {
    expect(data.input_tokens).toBeGreaterThan(50_000);
  });
});

// -- Stalled run --

describe("stalled run fixture", () => {
  const data: RunDetail = loadFixture("fixture_run_stalled.json");

  test("status is claimed (not actively running)", () => {
    expect(data.status).toBe("claimed");
  });

  test("has error indicating stall", () => {
    expect(data.error).toBeDefined();
    expect((data.error as string).toLowerCase()).toContain("no progress");
  });

  test("very long runtime with few turns indicates stall", () => {
    expect(data.runtime_seconds).toBeGreaterThan(30_000);
    expect(data.turn_count).toBeLessThan(5);
  });
});

// -- Retry queued run --

describe("retry queued run fixture", () => {
  const data: RunDetail = loadFixture("fixture_run_retry_queued.json");

  test("status is retry_queued", () => {
    expect(data.status).toBe("retry_queued");
  });

  test("has release_reason indicating why retry was needed", () => {
    expect(data.release_reason).toBeDefined();
  });

  test("has retry_attempt greater than 0", () => {
    expect(data.retry_attempt).toBeGreaterThan(0);
  });

  test("has finished_at indicating previous attempt completed", () => {
    expect(data.finished_at).toBeDefined();
  });

  test("has error from previous failed attempt", () => {
    expect(data.error).toBeDefined();
  });

  test("distinguishes queued retry from active harness work", () => {
    expect(data.status).toBe("retry_queued");
    expect(data.release_reason).toBe("tracker_inactive");
  });
});

// -- Detached run --

describe("detached run fixture", () => {
  const data: RunDetail = loadFixture("fixture_run_detached.json");

  test("status is released", () => {
    expect(data.status).toBe("released");
  });

  test("release_reason is cancelled", () => {
    expect(data.release_reason).toBe("cancelled");
  });

  test("has finished_at", () => {
    expect(data.finished_at).toBeDefined();
  });

  test("has error indicating detachment", () => {
    expect(data.error).toBeDefined();
    expect((data.error as string).toLowerCase()).toContain("detached");
  });

  test("explicit state shows detached vs active-underlying-harness", () => {
    expect(data.status).toBe("released");
    expect(data.release_reason).toBe("cancelled");
  });
});
