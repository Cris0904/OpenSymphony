/**
 * Worker-based decode and render loop for terminal frames.
 *
 * Uses requestAnimationFrame for smooth rendering while decoding
 * happens in a worker thread to prevent main thread blocking.
 */

import type { TerminalFrame, TerminalEncoding } from "@opensymphony/gateway-schema";
import type { DecodedFrame } from "./decoder.js";
import type { ScrollbackBuffer } from "./scrollback.js";
import { decodeBatch } from "./decoder.js";
import { appendFrames, createScrollbackBuffer, jumpToLatest } from "./scrollback.js";

export interface RenderMetrics {
  fps: number;
  memoryBytes: number;
  frameCount: number;
  decodeTimeMs: number;
  renderTimeMs: number;
  uiBlocked: boolean;
}

export interface RendererConfig {
  maxBufferCapacity: number;
  renderIntervalMs: number;
  batchSize: number;
}

const DEFAULT_CONFIG: RendererConfig = {
  maxBufferCapacity: 2000,
  renderIntervalMs: 16, // ~60fps
  batchSize: 100,
};

/**
 * Terminal renderer that handles high-throughput frame output
 * without blocking the main UI thread.
 */
export class TerminalRenderer {
  private config: RendererConfig;
  private buffer: ScrollbackBuffer;
  private pendingFrames: Array<{
    content: string;
    encoding: TerminalEncoding;
    frame: TerminalFrame;
  }> = [];
  private rafId: number | null = null;
  private lastRenderTime = 0;
  private metrics: RenderMetrics;
  private renderCallback?: (frames: DecodedFrame[], buffer: ScrollbackBuffer) => void;
  private worker: Worker | null = null;
  private useWorker: boolean;

  constructor(config?: Partial<RendererConfig>, useWorker = false) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.buffer = createScrollbackBuffer(this.config.maxBufferCapacity);
    this.metrics = {
      fps: 0,
      memoryBytes: 0,
      frameCount: 0,
      decodeTimeMs: 0,
      renderTimeMs: 0,
      uiBlocked: false,
    };
    this.useWorker = useWorker;

    if (this.useWorker) {
      this.initWorker();
    }
  }

  /**
   * Initialize Web Worker for off-main-thread decoding.
   * Note: In a real implementation, this would load a separate worker file.
   * For the prototype, we simulate worker behavior with message passing.
   */
  private initWorker(): void {
    // Simulated worker for prototype
    // In production: this.worker = new Worker(new URL("./decoder.worker.ts", import.meta.url));
    this.useWorker = false; // Fallback for prototype
    console.warn("Worker not available in prototype mode; using main thread decoding");
  }

  /**
   * Queue terminal frames for rendering.
   * Frames are batched and decoded in the render loop.
   */
  queueFrames(frames: Array<{ content: string; encoding: TerminalEncoding; frame: TerminalFrame }>): void {
    this.pendingFrames.push(...frames);

    if (!this.rafId) {
      this.lastRenderTime = performance.now();
      this.scheduleNextRender();
    }
  }

  /**
   * Single frame convenience method.
   */
  queueFrame(content: string, encoding: TerminalEncoding, frame: TerminalFrame): void {
    this.queueFrames([{ content, encoding, frame }]);
  }

  /**
   * Main render loop using requestAnimationFrame.
   * Decodes pending frames and updates the scrollback buffer.
   */
  private renderLoop = (timestamp: number): void => {
    const elapsed = timestamp - this.lastRenderTime;

    // Only render if enough time has passed (throttle to renderIntervalMs)
    if (elapsed < this.config.renderIntervalMs) {
      if (this.pendingFrames.length > 0) {
        this.scheduleNextRender();
      } else {
        this.rafId = null;
      }
      return;
    }

    this.lastRenderTime = timestamp;

    // Decode frames in batches
    const decodeStart = performance.now();
    const batch = this.pendingFrames.splice(0, this.config.batchSize);
    const decoded = decodeBatch(batch);
    const decodeTime = performance.now() - decodeStart;

    // Update metrics
    this.metrics.decodeTimeMs = decodeTime;
    this.metrics.frameCount += decoded.length;

    // Append to scrollback buffer
    const renderStart = performance.now();
    this.buffer = appendFrames(this.buffer, decoded);

    // Auto-scroll if at bottom
    if (this.buffer.atBottom) {
      this.buffer = jumpToLatest(this.buffer);
    }

    const renderTime = performance.now() - renderStart;
    this.metrics.renderTimeMs = renderTime;

    // Calculate FPS
    if (elapsed > 0) {
      this.metrics.fps = Math.round(1000 / elapsed);
    }

    // Estimate memory usage
    this.metrics.memoryBytes = this.estimateMemoryUsage();

    // Check if UI is blocked (decode or render took > 50ms)
    this.metrics.uiBlocked = decodeTime > 50 || renderTime > 50;

    // Invoke render callback if provided
    if (this.renderCallback) {
      this.renderCallback(decoded, this.buffer);
    }

    // Continue loop if more frames pending
    if (this.pendingFrames.length > 0) {
      this.scheduleNextRender();
    } else {
      this.rafId = null;
    }
  };

  /**
   * Schedule next render frame. Handles both browser and Node.js environments.
   */
  private scheduleNextRender(): void {
    const raf = typeof requestAnimationFrame !== "undefined"
      ? requestAnimationFrame
      : (callback: (time: number) => void) => setTimeout(() => callback(performance.now()), 0);
    
    this.rafId = raf(this.renderLoop) as unknown as number;
  }

  /**
   * Set callback for rendering updates.
   */
  onRender(callback: (frames: DecodedFrame[], buffer: ScrollbackBuffer) => void): void {
    this.renderCallback = callback;
  }

  /**
   * Get current render metrics.
   */
  getMetrics(): RenderMetrics {
    return { ...this.metrics };
  }

  /**
   * Get current scrollback buffer.
   */
  getBuffer(): ScrollbackBuffer {
    return this.buffer;
  }

  /**
   * Jump to latest output (bottom of scrollback).
   */
  jumpToLatest(): void {
    this.buffer = jumpToLatest(this.buffer);
    if (this.renderCallback) {
      this.renderCallback([], this.buffer);
    }
  }

  /**
   * Clear all frames and reset state.
   */
  clear(): void {
    this.pendingFrames = [];
    this.buffer = createScrollbackBuffer(this.config.maxBufferCapacity);
    if (this.rafId) {
      const cancelRaf = typeof cancelAnimationFrame !== "undefined"
        ? cancelAnimationFrame
        : clearTimeout;
      cancelRaf(this.rafId);
      this.rafId = null;
    }
  }

  /**
   * Dispose of renderer and cancel pending operations.
   */
  dispose(): void {
    this.clear();
    if (this.worker) {
      this.worker.terminate();
      this.worker = null;
    }
  }

  /**
   * Estimate memory usage for the buffer.
   */
  private estimateMemoryUsage(): number {
    const frameSize = this.buffer.visibleFrames.length * 100;
    const textData = this.buffer.visibleFrames.reduce((sum, f) => sum + f.text.length * 2, 0);
    const pendingSize = this.pendingFrames.reduce((sum, f) => sum + f.content.length * 2, 0);
    return frameSize + textData + pendingSize + 512; // Base overhead
  }
}

/**
 * Create a new terminal renderer instance.
 */
export function createTerminalRenderer(config?: Partial<RendererConfig>): TerminalRenderer {
  return new TerminalRenderer(config);
}
