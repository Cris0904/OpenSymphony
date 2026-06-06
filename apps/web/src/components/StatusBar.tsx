/**
 * StatusBar component.
 *
 * Displays connection status, gateway health, and active run indicators
 * at the bottom of the application shell.
 */

import { useState, useEffect } from "react";
import type { GatewayHealth } from "@opensymphony/gateway-schema";
import { formatTimeAgo } from "../lib/ui-utils";

type ConnectionState = "connected" | "reconnecting" | "disconnected" | "stale";

interface StatusState {
  connection: ConnectionState;
  gatewayHealth: GatewayHealth;
  activeRuns: number;
  lastSyncAt: string | null;
}

export function StatusBar(): React.ReactElement {
  const [status, setStatus] = useState<StatusState>({
    connection: "connected",
    gatewayHealth: "healthy",
    activeRuns: 0,
    lastSyncAt: new Date().toISOString(),
  });

  // Simulate connection monitoring. In production this would use WebSocket
  // events from the gateway to update connection state.
  useEffect(() => {
    const interval = setInterval(() => {
      setStatus((prev) => ({
        ...prev,
        lastSyncAt: new Date().toISOString(),
      }));
    }, 30_000);
    return () => clearInterval(interval);
  }, []);

  const connectionColor = {
    connected: "var(--color-success)",
    reconnecting: "var(--color-attention)",
    disconnected: "var(--color-danger)",
    stale: "var(--color-fg-subtle)",
  }[status.connection];

  const healthColor = {
    healthy: "var(--color-success)",
    degraded: "var(--color-attention)",
    failed: "var(--color-danger)",
    starting: "var(--color-fg-subtle)",
  }[status.gatewayHealth];

  return (
    <footer
      style={{
        height: "var(--statusbar-height)",
        display: "flex",
        alignItems: "center",
        padding: "0 var(--space-4)",
        borderTop: "1px solid var(--color-border-default)",
        background: "var(--color-bg-secondary)",
        fontSize: "11px",
        color: "var(--color-fg-muted)",
        flexShrink: 0,
        gap: "var(--space-4)",
      }}
    >
      {/* Connection status */}
      <span
        style={{ display: "flex", alignItems: "center", gap: "var(--space-1)" }}
        title={`Connection: ${status.connection}`}
      >
        <span
          style={{
            width: 8,
            height: 8,
            borderRadius: "50%",
            background: connectionColor,
            display: "inline-block",
          }}
        />
        <span style={{ textTransform: "capitalize" }}>{status.connection}</span>
      </span>

      {/* Gateway health */}
      <span
        style={{ display: "flex", alignItems: "center", gap: "var(--space-1)" }}
        title={`Gateway health: ${status.gatewayHealth}`}
      >
        <span
          style={{
            width: 8,
            height: 8,
            borderRadius: "50%",
            background: healthColor,
            display: "inline-block",
          }}
        />
        <span style={{ textTransform: "capitalize" }}>Gateway: {status.gatewayHealth}</span>
      </span>

      {/* Active runs */}
      {status.activeRuns > 0 && (
        <span title={`${status.activeRuns} active run(s)`}>
          Runs: {status.activeRuns}
        </span>
      )}

      <div style={{ flex: 1 }} />

      {/* Last sync */}
      {status.lastSyncAt && (
        <span title={`Last synced: ${status.lastSyncAt}`}>
          Synced {formatTimeAgo(status.lastSyncAt)}
        </span>
      )}
    </footer>
  );
}
