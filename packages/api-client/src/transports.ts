import type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
  PageCursor,
  TransportProfile,
} from "@opensymphony/gateway-schema";
import { pageCursorFirst } from "@opensymphony/gateway-schema";

/**
 * Tauri channel interface for streaming events.
 * This abstracts the @tauri-apps/api/channel types so we don't
 * depend on the Tauri SDK directly in the client package.
 */
interface TauriChannel<T> {
  onmessage?: (data: T) => void;
  close?: () => void;
}

/** Transport adapter interface for all gateway communication. */
export interface GatewayTransport {
  readonly baseUri: string;

  health(): Promise<GatewayCapabilities>;
  snapshot(): Promise<DashboardSnapshot>;
  taskGraph(projectId: string): Promise<TaskGraphSnapshot>;
  runDetail(runId: string): Promise<RunDetail>;
  runEvents(runId: string): Promise<RunEventPage>;
  terminalSnapshot(runId: string, terminalId: string): Promise<TerminalSnapshot>;

  /** Subscribe to gateway event stream; returns an async iterable. */
  events(): AsyncIterable<GatewayEnvelope>;

  /** Subscribe to terminal frame stream for a run. */
  terminalFrames(runId: string): AsyncIterable<GatewayEnvelope>;

  close(): Promise<void>;
}

export interface GatewayTransportConfig {
  baseUri: string;
  authToken?: string;
  transport?: TransportProfile;
}

/**
 * HTTP-based transport adapter for the OpenSymphony Gateway.
 *
 * Uses fetch() for REST endpoints and EventSource for SSE streams.
 * Supports local loopback and remote gateway profiles.
 */
export class HttpGatewayTransport implements GatewayTransport {
  readonly baseUri: string;
  private readonly authToken?: string;
  private abortController?: AbortController;

  constructor(config: GatewayTransportConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
    this.authToken = config.authToken;
  }

  private headers(): Record<string, string> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      Accept: "application/json",
    };
    if (this.authToken) {
      headers["Authorization"] = `Bearer ${this.authToken}`;
    }
    return headers;
  }

  private async get<T>(path: string): Promise<T> {
    const url = `${this.baseUri}${path}`;
    const response = await fetch(url, {
      method: "GET",
      headers: this.headers(),
      signal: this.abortController?.signal,
    });
    if (!response.ok) {
      throw new Error(
        `HTTP ${response.status} from ${url}: ${response.statusText}`,
      );
    }
    return (await response.json()) as T;
  }

  async health(): Promise<GatewayCapabilities> {
    return this.get<GatewayCapabilities>("/api/v1/capabilities");
  }

  async snapshot(): Promise<DashboardSnapshot> {
    return this.get<DashboardSnapshot>("/api/v1/dashboard/snapshot");
  }

  async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    return this.get<TaskGraphSnapshot>(
      `/api/v1/projects/${projectId}/taskgraph`,
    );
  }

  async runDetail(runId: string): Promise<RunDetail> {
    return this.get<RunDetail>(`/api/v1/runs/${runId}`);
  }

  async runEvents(
    runId: string,
    cursor?: PageCursor,
  ): Promise<RunEventPage> {
    const pageCursor = cursor ?? pageCursorFirst(100);
    const params = new URLSearchParams();
    if (pageCursor.page_token) {
      params.set("page_token", pageCursor.page_token);
    }
    params.set("page_size", String(pageCursor.page_size));
    return this.get<RunEventPage>(
      `/api/v1/runs/${runId}/events?${params.toString()}`,
    );
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
  ): Promise<TerminalSnapshot> {
    return this.get<TerminalSnapshot>(
      `/api/v1/runs/${runId}/terminal/${terminalId}/snapshot`,
    );
  }

  async *events(cursor?: number): AsyncIterable<GatewayEnvelope> {
    // Use SSE for event streaming with cursor-based replay
    const params = new URLSearchParams();
    if (cursor !== undefined) {
      params.set("cursor", String(cursor));
    }
    const url = `${this.baseUri}/api/v1/events?${params.toString()}`;

    // For SSE, we create an EventSource-like pattern using fetch + ReadableStream
    const response = await fetch(url, {
      method: "GET",
      headers: {
        ...this.headers(),
        Accept: "text/event-stream",
      },
      signal: this.abortController?.signal,
    });

    if (!response.ok || !response.body) {
      throw new Error(
        `HTTP ${response?.status} from SSE stream: ${response?.statusText}`,
      );
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        buffer = lines.pop() ?? "";

        for (const line of lines) {
          if (line.startsWith("data: ")) {
            const data = line.slice(6).trim();
            if (data) {
              try {
                yield JSON.parse(data) as GatewayEnvelope;
              } catch {
                // Skip malformed envelopes
              }
            }
          }
        }
      }
    } finally {
      reader.releaseLock();
    }
  }

  async *terminalFrames(runId: string): AsyncIterable<GatewayEnvelope> {
    // Use SSE for terminal frame streaming
    const url = `${this.baseUri}/api/v1/streams/terminal/${runId}`;

    const response = await fetch(url, {
      method: "GET",
      headers: {
        ...this.headers(),
        Accept: "text/event-stream",
      },
      signal: this.abortController?.signal,
    });

    if (!response.ok || !response.body) {
      throw new Error(
        `HTTP ${response?.status} from terminal stream: ${response?.statusText}`,
      );
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        buffer = lines.pop() ?? "";

        for (const line of lines) {
          if (line.startsWith("data: ")) {
            const data = line.slice(6).trim();
            if (data) {
              try {
                yield JSON.parse(data) as GatewayEnvelope;
              } catch {
                // Skip malformed frames
              }
            }
          }
        }
      }
    } finally {
      reader.releaseLock();
    }
  }

  async close(): Promise<void> {
    this.abortController?.abort();
    this.abortController = undefined;
  }
}

/**
 * WebSocket-based transport adapter for the OpenSymphony Gateway.
 *
 * Uses WebSocket for bidirectional streaming of events and terminal frames.
 * Supports cursor-based replay and automatic reconnection.
 */
export class WebSocketTransport implements GatewayTransport {
  readonly baseUri: string;
  private readonly authToken?: string;
  private ws?: WebSocket;
  private eventSubscribers: Set<(envelope: GatewayEnvelope) => void> = new Set();
  private terminalSubscribers: Map<string, Set<(envelope: GatewayEnvelope) => void>> = new Map();
  private pendingGeneratorCancellers: Set<() => void> = new Set();
  private reconnectDelayMs = 1000;
  private maxReconnectDelayMs = 30000;
  private isReconnecting = false;
  private isClosed = false;

  constructor(config: GatewayTransportConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
    this.authToken = config.authToken;
  }

  private wsUrl(path: string): string {
    const uri = this.baseUri.replace(/^http/, "ws");
    return `${uri}${path}`;
  }

  private headers(): Record<string, string> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      Accept: "application/json",
    };
    if (this.authToken) {
      headers["Authorization"] = `Bearer ${this.authToken}`;
    }
    return headers;
  }

  private async get<T>(path: string): Promise<T> {
    const url = `${this.baseUri}${path}`;
    const response = await fetch(url, {
      method: "GET",
      headers: this.headers(),
    });
    if (!response.ok) {
      throw new Error(
        `HTTP ${response.status} from ${url}: ${response.statusText}`,
      );
    }
    return (await response.json()) as T;
  }

  async health(): Promise<GatewayCapabilities> {
    return this.get<GatewayCapabilities>("/api/v1/capabilities");
  }

  async snapshot(): Promise<DashboardSnapshot> {
    return this.get<DashboardSnapshot>("/api/v1/dashboard/snapshot");
  }

  async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    return this.get<TaskGraphSnapshot>(
      `/api/v1/projects/${projectId}/taskgraph`,
    );
  }

  async runDetail(runId: string): Promise<RunDetail> {
    return this.get<RunDetail>(`/api/v1/runs/${runId}`);
  }

  async runEvents(
    runId: string,
    cursor?: PageCursor,
  ): Promise<RunEventPage> {
    const pageCursor = cursor ?? pageCursorFirst(100);
    const params = new URLSearchParams();
    if (pageCursor.page_token) {
      params.set("page_token", pageCursor.page_token);
    }
    params.set("page_size", String(pageCursor.page_size));
    return this.get<RunEventPage>(
      `/api/v1/runs/${runId}/events?${params.toString()}`,
    );
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
  ): Promise<TerminalSnapshot> {
    return this.get<TerminalSnapshot>(
      `/api/v1/runs/${runId}/terminal/${terminalId}/snapshot`,
    );
  }

  private async ensureConnected(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      return;
    }
    await this.connectWebSocket();
  }

  private async connectWebSocket(): Promise<void> {
    if (this.ws) {
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.onmessage = null;
      this.ws.close();
    }

    const WS_CONNECT_TIMEOUT_MS = 10_000;
    return new Promise((resolve, reject) => {
      const url = this.wsUrl("/api/v1/streams/events");
      this.ws = new WebSocket(url);

      const timeoutId = setTimeout(() => {
        if (this.ws?.readyState === WebSocket.CONNECTING) {
          this.ws.close();
          reject(new Error(`WebSocket connection timed out after ${WS_CONNECT_TIMEOUT_MS}ms`));
        }
      }, WS_CONNECT_TIMEOUT_MS);

      this.ws.onopen = () => {
        clearTimeout(timeoutId);
        // Send auth if needed
        if (this.authToken) {
          this.ws?.send(
            JSON.stringify({ type: "auth", token: this.authToken }),
          );
        }
        resolve();
      };

      this.ws.onerror = (error) => {
        clearTimeout(timeoutId);
        reject(error);
      };

      this.ws.onclose = () => {
        clearTimeout(timeoutId);
        this.scheduleReconnect();
      };

      this.ws.onmessage = (event) => {
        this.handleMessage(event.data);
      };
    });
  }

  private handleMessage(data: string): void {
    // Gateway uses prefixed frames: "__event__ {...}" or "__error__ {...}"
    if (data.startsWith("__event__ ")) {
      try {
        const payload = data.slice(10);
        const envelope = JSON.parse(payload) as GatewayEnvelope;
        // Dispatch to all event subscribers
        this.eventSubscribers.forEach((cb) => cb(envelope));
        // Also dispatch to terminal subscribers matching the run ID
        if (envelope.entity_ref.kind === "terminal_session") {
          const runId = envelope.cursor.partition.split(":").pop();
          if (runId) {
            this.terminalSubscribers.get(runId)?.forEach((cb) => cb(envelope));
          }
        }
      } catch {
        // Skip malformed messages
      }
    } else if (data.startsWith("__error__ ")) {
      // Handle stream error - could trigger reconnect
      try {
        const payload = data.slice(10);
        const error = JSON.parse(payload);
        if (error.recoverable) {
          this.scheduleReconnect();
        }
      } catch {
        // Skip malformed errors
      }
    } else {
      // Try parsing as direct JSON envelope (legacy format)
      try {
        const envelope = JSON.parse(data) as GatewayEnvelope;
        // Dispatch to all event subscribers
        this.eventSubscribers.forEach((cb) => cb(envelope));
        // Also dispatch to terminal subscribers matching the run ID
        if (envelope.entity_ref.kind === "terminal_session") {
          const runId = envelope.cursor.partition.split(":").pop();
          if (runId) {
            this.terminalSubscribers.get(runId)?.forEach((cb) => cb(envelope));
          }
        }
      } catch {
        // Skip unknown message formats
      }
    }
  }

  private scheduleReconnect(): void {
    if (this.isReconnecting) return;
    this.isReconnecting = true;

    const delay = Math.min(
      this.reconnectDelayMs * 2,
      this.maxReconnectDelayMs,
    );
    this.reconnectDelayMs = delay;

    setTimeout(() => {
      this.isReconnecting = false;
      this.connectWebSocket().catch(() => {
        // Reconnect will be scheduled again on close
      });
    }, delay);
  }

  async *events(_cursor?: number): AsyncIterable<GatewayEnvelope> {
    await this.ensureConnected();

    // Create a promise-based queue for this subscriber
    const queue: GatewayEnvelope[] = [];
    let resolveNext: ((value: IteratorResult<GatewayEnvelope>) => void) | null = null;

    const subscriber = (envelope: GatewayEnvelope) => {
      queue.push(envelope);
      if (resolveNext) {
        resolveNext({ value: envelope, done: false });
        resolveNext = null;
      }
    };

    // Track this generator's resolve function for cleanup on close
    const cancelGenerator = () => {
      if (resolveNext) {
        resolveNext({ value: {} as GatewayEnvelope, done: true });
        resolveNext = null;
      }
    };
    this.pendingGeneratorCancellers.add(cancelGenerator);

    this.eventSubscribers.add(subscriber);

    try {
      while (!this.isClosed) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else {
          await new Promise<IteratorResult<GatewayEnvelope>>((resolve) => {
            resolveNext = resolve;
          });
        }
      }
    } finally {
      this.eventSubscribers.delete(subscriber);
      this.pendingGeneratorCancellers.delete(cancelGenerator);
    }
  }

  async *terminalFrames(runId: string): AsyncIterable<GatewayEnvelope> {
    await this.ensureConnected();

    const queue: GatewayEnvelope[] = [];
    let resolveNext: ((value: IteratorResult<GatewayEnvelope>) => void) | null = null;

    const subscriber = (envelope: GatewayEnvelope) => {
      if (envelope.entity_ref.kind === "terminal_session") {
        queue.push(envelope);
        if (resolveNext) {
          resolveNext({ value: envelope, done: false });
          resolveNext = null;
        }
      }
    };

    // Track this generator's resolve function for cleanup on close
    const cancelGenerator = () => {
      if (resolveNext) {
        resolveNext({ value: {} as GatewayEnvelope, done: true });
        resolveNext = null;
      }
    };
    this.pendingGeneratorCancellers.add(cancelGenerator);

    this.terminalSubscribers.set(runId, new Set([subscriber]));

    try {
      while (!this.isClosed) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else {
          await new Promise<IteratorResult<GatewayEnvelope>>((resolve) => {
            resolveNext = resolve;
          });
        }
      }
    } finally {
      this.terminalSubscribers.get(runId)?.delete(subscriber);
      this.pendingGeneratorCancellers.delete(cancelGenerator);
    }
  }

  async close(): Promise<void> {
    this.isClosed = true;
    
    // Resolve all pending generator promises to prevent memory leaks and hangs
    for (const cancel of this.pendingGeneratorCancellers) {
      cancel();
    }
    this.pendingGeneratorCancellers.clear();
    
    if (this.ws) {
      this.ws.onclose = null;
      this.ws.close();
      this.ws = undefined;
    }
    this.eventSubscribers.clear();
    this.terminalSubscribers.clear();
  }
}

/**
 * Tauri channel transport adapter for desktop local mode.
 *
 * Uses Tauri's invoke/channel system for high-performance local communication
 * between the Rust backend and webview frontend. This transport is optimized
 * for local gateway connections where the orchestrator runs on the same machine.
 *
 * In the preferred transport order:
 * 1. In-process Rust channels (when embedded) - not available in webview
 * 2. Native IPC (Unix sockets/named pipes) - via loopback fallback
 * 3. Tauri channels (this transport) - high-volume frames to webview
 * 4. Loopback HTTP/WebSocket - compatibility baseline
 */
export class TauriChannelTransport implements GatewayTransport {
  readonly baseUri: string;
  private readonly authToken?: string;
  private eventChannel?: TauriChannel<GatewayEnvelope>;
  private terminalChannels: Map<string, TauriChannel<GatewayEnvelope>> = new Map();

  constructor(config: GatewayTransportConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
    this.authToken = config.authToken;
  }

  private async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    // In a real Tauri app, this would use @tauri-apps/api/core invoke
    // For type compatibility, we define the expected interface
    const tauri = (globalThis as Record<string, unknown>).__TAURI__ as
      | { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<T> }
      | undefined;

    if (!tauri?.invoke) {
      throw new Error(
        "TauriChannelTransport requires Tauri runtime context",
      );
    }

    return tauri.invoke(command, {
      ...args,
      auth_token: this.authToken,
    });
  }

  async health(): Promise<GatewayCapabilities> {
    return this.invoke<GatewayCapabilities>("health", {});
  }

  async snapshot(): Promise<DashboardSnapshot> {
    return this.invoke<DashboardSnapshot>("dashboard_snapshot", {});
  }

  async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    return this.invoke<TaskGraphSnapshot>("task_graph", { project_id: projectId });
  }

  async runDetail(runId: string): Promise<RunDetail> {
    return this.invoke<RunDetail>("run_detail", { run_id: runId });
  }

  async runEvents(runId: string, _cursor?: PageCursor): Promise<RunEventPage> {
    return this.invoke<RunEventPage>("run_events", { run_id: runId });
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
  ): Promise<TerminalSnapshot> {
    return this.invoke<TerminalSnapshot>("terminal_snapshot", {
      run_id: runId,
      terminal_id: terminalId,
    });
  }

  async *events(_cursor?: number): AsyncIterable<GatewayEnvelope> {
    // Tauri channels provide a callback-based stream
    // We convert it to an async iterable for the GatewayTransport interface
    const queue: GatewayEnvelope[] = [];
    let resolveNext: ((value: IteratorResult<GatewayEnvelope>) => void) | null = null;

    const channel = await this.invoke<TauriChannel<GatewayEnvelope>>(
      "subscribe_events",
      {},
    );

    channel.onmessage = (envelope: GatewayEnvelope) => {
      queue.push(envelope);
      if (resolveNext) {
        resolveNext({ value: envelope, done: false });
        resolveNext = null;
      }
    };

    this.eventChannel = channel;

    try {
      while (true) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else {
          await new Promise<IteratorResult<GatewayEnvelope>>((resolve) => {
            resolveNext = resolve;
          });
        }
      }
    } finally {
      channel.close?.();
    }
  }

  async *terminalFrames(runId: string): AsyncIterable<GatewayEnvelope> {
    const queue: GatewayEnvelope[] = [];
    let resolveNext: ((value: IteratorResult<GatewayEnvelope>) => void) | null = null;

    const channel = await this.invoke<TauriChannel<GatewayEnvelope>>(
      "subscribe_terminal",
      { run_id: runId },
    );

    channel.onmessage = (envelope: GatewayEnvelope) => {
      if (envelope.entity_ref.kind === "terminal_session") {
        queue.push(envelope);
        if (resolveNext) {
          resolveNext({ value: envelope, done: false });
          resolveNext = null;
        }
      }
    };

    this.terminalChannels.set(runId, channel);

    try {
      while (true) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else {
          await new Promise<IteratorResult<GatewayEnvelope>>((resolve) => {
            resolveNext = resolve;
          });
        }
      }
    } finally {
      channel.close?.();
      this.terminalChannels.delete(runId);
    }
  }

  async close(): Promise<void> {
    this.eventChannel?.close?.();
    for (const channel of this.terminalChannels.values()) {
      channel.close?.();
    }
    this.eventChannel = undefined;
    this.terminalChannels.clear();
  }
}

/**
 * Transport factory that selects the best available transport profile
 * based on the gateway capabilities and connection configuration.
 *
 * Preferred transport order for desktop local mode:
 * 1. In-process Rust channels (embedded host) - lowest latency
 * 2. Native local IPC (separate local process) - Unix sockets/named pipes
 * 3. Tauri channels (Rust backend to webview) - high-volume frames
 * 4. Loopback HTTP/WebSocket - compatibility baseline
 */
export class TransportFactory {
  /**
   * Create a transport based on the recommended profile and available capabilities.
   * Falls back to loopback HTTP if the preferred transport is unavailable.
   */
  static async create(
    config: GatewayTransportConfig,
    capabilities?: GatewayCapabilities,
  ): Promise<GatewayTransport> {
    const profile = config.transport ?? "loopback_http";

    // If we have capabilities, verify the transport is supported
    if (capabilities) {
      const transportCap = capabilities.transports.find(
        (t) => t.transport === profile,
      );
      if (!transportCap) {
        // Fall back to loopback HTTP
        return new HttpGatewayTransport(config);
      }
    }

    switch (profile) {
      case "in_process_channel":
      case "native_ipc":
      case "tauri_channel":
        // These require Tauri runtime context
        if (typeof (globalThis as Record<string, unknown>).__TAURI__ !== "undefined") {
          return new TauriChannelTransport(config);
        }
        // Fall through to HTTP if Tauri not available
        return new HttpGatewayTransport(config);

      case "loopback_http":
      case "sse":
        return new HttpGatewayTransport(config);

      case "loopback_websocket":
      case "websocket":
      case "json_rpc_over_websocket":
        // Check if WebSocket is available
        if (typeof WebSocket !== "undefined") {
          return new WebSocketTransport(config);
        }
        // Fall back to HTTP
        return new HttpGatewayTransport(config);

      default:
        return new HttpGatewayTransport(config);
    }
  }

  /**
   * Determine the best transport profile for the current environment.
   * Returns profiles in order of preference.
   */
  static getPreferredProfiles(): Array<{
    profile: string;
    available: boolean;
    description: string;
  }> {
    const isTauri =
      typeof (globalThis as Record<string, unknown>).__TAURI__ !== "undefined";
    const hasWebSocket = typeof WebSocket !== "undefined";

    return [
      {
        profile: isTauri ? "tauri_channel" : "in_process_channel",
        available: isTauri,
        description: isTauri
          ? "Tauri channels (Rust backend to webview)"
          : "In-process Rust channels (embedded host)",
      },
      {
        profile: "native_ipc",
        available: typeof process !== "undefined",
        description: "Native local IPC (Unix sockets/named pipes)",
      },
      {
        profile: hasWebSocket ? "loopback_websocket" : "loopback_http",
        available: hasWebSocket,
        description: hasWebSocket
          ? "Loopback WebSocket"
          : "Loopback HTTP",
      },
      {
        profile: "loopback_http",
        available: true,
        description: "Loopback HTTP (compatibility baseline)",
      },
    ];
  }
}