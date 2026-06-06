/**
 * Shared UI utilities.
 *
 * Extracted common helpers used across multiple components to avoid
 * duplication and drift.
 */

import type { RunStatus } from "@opensymphony/gateway-schema";

// -- Time formatting --

/** Format relative time (e.g., "5m ago", "2h ago", "just now"). */
export function formatTimeAgo(isoString: string): string {
  const diff = Date.now() - new Date(isoString).getTime();
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ago`;
}

/** Format duration from seconds (e.g., "2h 30m", "5m 12s", "45s"). */
export function formatDuration(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}

/** Format date/time for display. */
export function formatDateTime(isoString: string): string {
  const date = new Date(isoString);
  return date.toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// -- Token and cost formatting --

/** Format token count for display. */
export function formatTokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`;
  return count.toString();
}

/** Format cost from micros to dollars. */
export function formatCost(micros: number): string {
  const dollars = micros / 1_000_000;
  if (dollars >= 1) return `$${dollars.toFixed(2)}`;
  if (dollars >= 0.01) return `$${dollars.toFixed(3)}`;
  return `${(dollars * 100).toFixed(1)}¢`;
}

// -- Run status badge colors --

type BadgeColors = { bg: string; fg: string };

/** Known liveness states for run monitoring. */
export type LivenessState = "active" | "quiet" | "degraded" | "stalled" | "detached";

/** Color mapping for run status badges (shared across all components). */
export const RUN_STATUS_COLORS: Record<RunStatus, BadgeColors> = {
  running: { bg: "rgba(63, 185, 80, 0.15)", fg: "var(--color-success)" },
  retry_queued: { bg: "rgba(210, 153, 34, 0.15)", fg: "var(--color-attention)" },
  released: { bg: "rgba(139, 148, 158, 0.15)", fg: "var(--color-fg-muted)" },
  claimed: { bg: "rgba(88, 166, 255, 0.15)", fg: "var(--color-accent)" },
  unclaimed: { bg: "rgba(110, 118, 129, 0.15)", fg: "var(--color-fg-subtle)" },
} as const satisfies Record<RunStatus, BadgeColors>;

/** Color mapping for liveness badges (shared across all components). */
export const LIVENESS_COLORS: Record<LivenessState, BadgeColors> = {
  active: { bg: "rgba(63, 185, 80, 0.15)", fg: "var(--color-success)" },
  quiet: { bg: "rgba(139, 148, 158, 0.15)", fg: "var(--color-fg-muted)" },
  degraded: { bg: "rgba(210, 153, 34, 0.15)", fg: "var(--color-attention)" },
  stalled: { bg: "rgba(248, 81, 73, 0.15)", fg: "var(--color-danger)" },
  detached: { bg: "rgba(110, 118, 129, 0.15)", fg: "var(--color-fg-subtle)" },
} as const satisfies Record<LivenessState, BadgeColors>;

/** Color mapping for state category badges (from gateway TaskGraphStateCategory). */
export const STATE_CATEGORY_COLORS: Record<
  "done" | "in_progress" | "todo" | "backlog" | "canceled",
  BadgeColors
> = {
  done: { bg: "rgba(63, 185, 80, 0.15)", fg: "var(--color-success)" },
  in_progress: { bg: "rgba(88, 166, 255, 0.15)", fg: "var(--color-accent)" },
  todo: { bg: "rgba(139, 148, 158, 0.15)", fg: "var(--color-fg-muted)" },
  backlog: { bg: "rgba(110, 118, 129, 0.15)", fg: "var(--color-fg-subtle)" },
  canceled: { bg: "rgba(248, 81, 73, 0.15)", fg: "var(--color-danger)" },
} as const;
