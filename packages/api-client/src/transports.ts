import type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
  ActionDispatch,
  ActionReceipt,
  PageCursor,
} from "@opensymphony/gateway-schema";
import type { GatewayTransport, GatewayTransportConfig, ActionCapableTransport } from "./index.js";

/**
 * HTTP-based transport adapter using fetch().
 *
 * Supports REST endpoints for snapshots/reads/mutations and SSE
 * for live event streams. Designed to be the baseline contract
 * that all other transport adapters must satisfy.
 */
export class HttpGatewayTransport implements GatewayTransport, ActionCapableTransport {
  readonly baseUri: string;
  private authToken?: string;
  private closed = false;
  private reconnectAttempts = 0;
  private maxReconnectAttempts = 5;
  private reconnectDelayMs = 1000;
  private lastEventTimestamp: number | null = null;
  private streamHealthy = true;
  private readonly streamHealthTimeoutMs = 30_000;
  private abortController: AbortController | null = null;

  constructor(config: GatewayTransportConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
    this.authToken = config.authToken;
  }

  // -- REST reads --

  async health(): Promise<GatewayCapabilities> {
    const response = await this.fetchJson(`${this.baseUri}/api/v1/health`);
    return response as GatewayCapabilities;
  }

  async snapshot(): Promise<DashboardSnapshot> {
    const response = await this.fetchJson(`${this.baseUri}/api/v1/snapshot`);
    return response as DashboardSnapshot;
  }

  async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/projects/${encodeURIComponent(projectId)}/task-graph`,
    );
    return response as TaskGraphSnapshot;
  }

  async runDetail(runId: string): Promise<RunDetail> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}`,
    );
    return response as RunDetail;
  }

  async runEvents(runId: string, cursor?: PageCursor): Promise<RunEventPage> {
    const params = new URLSearchParams();
    if (cursor?.page_token) params.set("page_token", cursor.page_token);
    params.set("page_size", String(cursor?.page_size ?? 100));
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/events?${params}`,
    );
    return response as RunEventPage;
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
  ): Promise<TerminalSnapshot> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/terminals/${encodeURIComponent(terminalId)}/snapshot`,
    );
    return response as TerminalSnapshot;
  }

  // -- Event streams (SSE) --

  async *events(fromCursor?: { sequence: number; partition: string }): AsyncIterable<GatewayEnvelope> {
    const url = new URL(`${this.baseUri}/api/v1/events`);
    if (fromCursor) {
      url.searchParams.set("cursor_sequence", String(fromCursor.sequence));
      url.searchParams.set("cursor_partition", fromCursor.partition);
    }

    while (!this.closed) {
      const controller = new AbortController();
      this.abortController = controller;
      const response = await fetch(url.toString(), {
        ...this.buildRequestInit(),
        signal: controller.signal,
      });

      if (!response.ok) {
        throw new Error(`Event stream failed: ${response.status} ${response.statusText}`);
      }

      const reader = response.body?.getReader();
      if (!reader) {
        throw new Error("Event stream response has no readable body");
      }

      try {
        for await (const envelope of this.parseSSE(reader)) {
          this.lastEventTimestamp = Date.now();
          this.streamHealthy = true;
          this.reconnectAttempts = 0;
          yield envelope;
        }
      } finally {
        reader.releaseLock();
      }

      // Reconnect logic.
      if (!this.closed) {
        this.streamHealthy = false;
        await this.waitForReconnect();
      }
    }
  }

  async *terminalFrames(runId: string): AsyncIterable<GatewayEnvelope> {
    const url = new URL(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/terminal/stream`,
    );

    while (!this.closed) {
      const controller = new AbortController();
      this.abortController = controller;
      const response = await fetch(url.toString(), {
        ...this.buildRequestInit(),
        signal: controller.signal,
      });

      if (!response.ok) {
        throw new Error(`Terminal stream failed: ${response.status} ${response.statusText}`);
      }

      const reader = response.body?.getReader();
      if (!reader) {
        throw new Error("Terminal stream response has no readable body");
      }

      try {
        yield* this.parseSSE(reader);
      } finally {
        reader.releaseLock();
      }

      if (!this.closed) {
        await this.waitForReconnect();
      }
    }
  }

  /** Parse an SSE stream into GatewayEnvelope objects. */
  private async *parseSSE(
    reader: ReadableStreamDefaultReader<Uint8Array>,
  ): AsyncIterable<GatewayEnvelope> {
    const decoder = new TextDecoder();
    let buffer = "";
    let currentEvent = "";
    let currentId = "";
    let currentRetry = 0;
    let currentData = "";

    while (!this.closed) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        // Empty line = end of event block.
        if (line === "") {
          if (currentData) {
            try {
              const envelope = JSON.parse(currentData) as GatewayEnvelope;
              yield envelope;
            } catch (err) {
              console.error("SSE parse error: malformed JSON event data", err);
            }
            currentEvent = "";
            currentId = "";
            currentRetry = 0;
            currentData = "";
          }
          continue;
        }

        if (line.startsWith("event: ")) {
          currentEvent = line.slice(7);
        } else if (line.startsWith("id: ")) {
          currentId = line.slice(4);
        } else if (line.startsWith("retry: ")) {
          currentRetry = parseInt(line.slice(7), 10) || 0;
        } else if (line.startsWith("data: ")) {
          // Multi-line data: append with newline.
          if (currentData) currentData += "\n";
          currentData += line.slice(6);
        } else if (line.startsWith(":")) {
          // SSE comment line, ignore.
        }
        // Any other line is treated as data continuation.
        else {
          if (currentData) currentData += "\n";
          currentData += line;
        }
      }

      if (currentRetry > 0) {
        this.reconnectDelayMs = currentRetry;
      }
    }
  }

  // -- Action mutations --

  async dispatchAction(action: ActionDispatch): Promise<ActionReceipt> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/actions/dispatch`,
      {
        method: "POST",
        body: JSON.stringify(action),
      },
    );
    return response as ActionReceipt;
  }

  async cancelRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `cancel-${runId}-${Date.now()}`,
      action_kind: "cancel",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `cancel-${runId}`,
    });
  }

  async retryRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `retry-${runId}-${Date.now()}`,
      action_kind: "retry",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `retry-${runId}`,
    });
  }

  async resumeRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `resume-${runId}-${Date.now()}`,
      action_kind: "resume",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `resume-${runId}`,
    });
  }

  // -- Lifecycle --

  async close(): Promise<void> {
    this.closed = true;
    this.abortController?.abort();
  }

  // -- Stream health diagnostics --

  /** Whether the stream has received events recently. */
  isStreamHealthy(): boolean {
    if (this.lastEventTimestamp === null) return true;
    return Date.now() - this.lastEventTimestamp < this.streamHealthTimeoutMs;
  }

  /** Reconnect attempt count since last successful event. */
  getReconnectAttempts(): number {
    return this.reconnectAttempts;
  }

  // -- Private helpers --

  private buildRequestInit(): RequestInit {
    const init: RequestInit = {
      headers: {
        Accept: "text/event-stream",
      },
    };
    if (this.authToken) {
      init.headers = {
        ...init.headers,
        Authorization: `Bearer ${this.authToken}`,
      };
    }
    return init;
  }

  private async fetchJson(url: string, init?: RequestInit): Promise<unknown> {
    const requestInit: RequestInit = {
      ...init,
      headers: {
        "Content-Type": "application/json",
        ...(init?.headers ?? {}),
      },
    };
    if (this.authToken) {
      requestInit.headers = {
        ...requestInit.headers,
        Authorization: `Bearer ${this.authToken}`,
      };
    }

    const response = await fetch(url, requestInit);

    if (!response.ok) {
      const body = await response.text().catch(() => "");
      throw new Error(
        `HTTP ${response.status} ${response.statusText}: ${body}`,
      );
    }

    return response.json();
  }

  private async waitForReconnect(): Promise<void> {
    this.reconnectAttempts++;
    if (this.reconnectAttempts > this.maxReconnectAttempts) {
      throw new Error(
        `Max reconnect attempts (${this.maxReconnectAttempts}) reached`,
      );
    }
    const delay = this.reconnectDelayMs * Math.pow(2, this.reconnectAttempts - 1);
    await new Promise((resolve) => setTimeout(resolve, delay));
  }
}