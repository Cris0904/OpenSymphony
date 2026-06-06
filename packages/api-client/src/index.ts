/**
 * OpenSymphony Gateway API Client.
 *
 * Provides transport adapters for connecting to the gateway across
 * different deployment modes (local desktop, web, hosted).
 */

export {
  HttpGatewayTransport,
  WebSocketTransport,
  TauriChannelTransport,
  TransportFactory,
} from "./transports.js";

export type {
  GatewayTransport,
  GatewayTransportConfig,
} from "./transports.js";

// Re-export gateway schema types for convenience
export type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
  TransportProfile,
  PageCursor,
} from "@opensymphony/gateway-schema";

export {
  pageCursorFirst,
  GATEWAY_SCHEMA_VERSION,
  isValidGatewayEnvelope,
  assertValidGatewayEnvelope,
} from "@opensymphony/gateway-schema";