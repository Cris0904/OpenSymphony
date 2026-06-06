/**
 * DOM-based terminal viewer component.
 *
 * Renders decoded frames to the browser DOM with support for:
 * - Scrollback with virtualized buffering
 * - Text search and highlighting
 * - Copy to clipboard
 * - Jump to latest output
 * - ANSI color display
 */

import { searchText } from "@opensymphony/ui-core";
import type { DecodedFrame, ScrollbackBuffer, TextStyle, ColorStyle } from "@opensymphony/ui-core";
import type { TerminalRenderer } from "@opensymphony/ui-core";

export interface TerminalViewerConfig {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  wrapLines: boolean;
  maxVisibleFrames: number;
}

export interface TerminalViewerOptions {
  container: HTMLElement;
  config?: Partial<TerminalViewerConfig>;
}

/**
 * Terminal viewer that renders frames to a DOM container.
 *
 * Maintains live DOM element references to avoid O(n²) thrashing:
 * - New lines are appended directly (not cloned)
 * - Old lines are removed when exceeding maxVisibleFrames
 * - Search operates on in-document elements
 */
export class TerminalViewer {
  private container: HTMLElement;
  private config: TerminalViewerConfig;
  private scrollContainer!: HTMLElement;
  private toolbar!: HTMLElement;
  private searchInput!: HTMLInputElement;
  private searchButton!: HTMLButtonElement;
  private copyButton!: HTMLButtonElement;
  private jumpButton!: HTMLButtonElement;
  private statusSpan!: HTMLSpanElement;
  /** Live DOM elements that are children of scrollContainer. */
  private lineElements: HTMLElement[] = [];
  private searchTerm = "";
  private searchResults: number[] = [];
  private currentSearchIndex = 0;
  private pendingFocusFrameIndex: number | undefined;

  constructor(private renderer: TerminalRenderer, options: TerminalViewerOptions) {
    this.container = options.container;
    this.config = {
      fontFamily: options.config?.fontFamily ?? "Menlo, Monaco, 'Courier New', monospace",
      fontSize: options.config?.fontSize ?? 14,
      lineHeight: options.config?.lineHeight ?? 1.4,
      wrapLines: options.config?.wrapLines ?? true,
      maxVisibleFrames: options.config?.maxVisibleFrames ?? 200,
    };

    // Build UI structure
    this.buildUI();
    this.attachRenderer();
  }

  /**
   * Build the terminal viewer UI.
   */
  private buildUI(): void {
    // Create terminal container
    this.container.innerHTML = "";
    this.container.style.cssText = `
      display: flex;
      flex-direction: column;
      height: 100%;
      max-height: 600px;
      background: #0d1117;
      border: 1px solid #30363d;
      border-radius: 6px;
      overflow: hidden;
      font-family: ${this.config.fontFamily};
    `;

    // Toolbar
    this.toolbar = document.createElement("div");
    this.toolbar.style.cssText = `
      display: flex;
      gap: 8px;
      padding: 8px;
      background: #161b22;
      border-bottom: 1px solid #30363d;
      align-items: center;
    `;

    // Search input
    this.searchInput = document.createElement("input");
    this.searchInput.type = "text";
    this.searchInput.placeholder = "Search terminal output...";
    this.searchInput.style.cssText = `
      flex: 1;
      padding: 4px 8px;
      background: #0d1117;
      border: 1px solid #30363d;
      border-radius: 4px;
      color: #c9d1d9;
      font-size: 13px;
    `;

    // Search button
    this.searchButton = document.createElement("button");
    this.searchButton.textContent = "Search";
    this.searchButton.style.cssText = `
      padding: 4px 12px;
      background: #21262d;
      border: 1px solid #30363d;
      border-radius: 4px;
      color: #c9d1d9;
      cursor: pointer;
      font-size: 13px;
    `;

    // Copy button
    this.copyButton = document.createElement("button");
    this.copyButton.textContent = "Copy";
    this.copyButton.style.cssText = `
      padding: 4px 12px;
      background: #21262d;
      border: 1px solid #30363d;
      border-radius: 4px;
      color: #c9d1d9;
      cursor: pointer;
      font-size: 13px;
    `;

    // Jump to latest button
    this.jumpButton = document.createElement("button");
    this.jumpButton.textContent = "Latest";
    this.jumpButton.style.cssText = `
      padding: 4px 12px;
      background: #238636;
      border: 1px solid #2ea043;
      border-radius: 4px;
      color: #ffffff;
      cursor: pointer;
      font-size: 13px;
    `;

    // Status span
    this.statusSpan = document.createElement("span");
    this.statusSpan.style.cssText = `
      margin-left: auto;
      color: #8b949e;
      font-size: 12px;
    `;

    // Add toolbar elements
    this.toolbar.appendChild(this.searchInput);
    this.toolbar.appendChild(this.searchButton);
    this.toolbar.appendChild(this.copyButton);
    this.toolbar.appendChild(this.jumpButton);
    this.toolbar.appendChild(this.statusSpan);
    this.container.appendChild(this.toolbar);

    // Scroll container for terminal output
    this.scrollContainer = document.createElement("div");
    this.scrollContainer.style.cssText = `
      flex: 1;
      overflow-y: auto;
      padding: 8px;
      font-size: ${this.config.fontSize}px;
      line-height: ${this.config.lineHeight};
      word-wrap: ${this.config.wrapLines ? "break-word" : "normal"};
    `;
    this.container.appendChild(this.scrollContainer);

    // Event listeners
    this.searchButton.addEventListener("click", () => this.performSearch());
    this.searchInput.addEventListener("keypress", (e) => {
      if (e.key === "Enter") this.performSearch();
    });
    this.copyButton.addEventListener("click", () => this.copyToClipboard());
    this.jumpButton.addEventListener("click", () => this.jumpToLatest());
  }

  /**
   * Attach to the renderer and listen for updates.
   */
  private attachRenderer(): void {
    this.renderer.onRender((decodedFrames: DecodedFrame[], buffer: ScrollbackBuffer) => {
      this.renderFrames(decodedFrames, buffer);
    });
  }

  /**
   * Render new frames to the DOM using incremental updates.
   * Only appends new lines and removes pruned ones to avoid O(n²) DOM thrashing.
   * When called with empty decodedFrames (e.g., from scrollToFrame/jumpToLatest),
   * rebuilds the visible DOM from the buffer's current visibleFrames.
   */
  private renderFrames(decodedFrames: DecodedFrame[], buffer: ScrollbackBuffer): void {
    if (decodedFrames.length > 0 && buffer.atBottom) {
      // Append only new frames
      const firstFrameIndex = buffer.totalFrames - decodedFrames.length;
      for (let i = 0; i < decodedFrames.length; i++) {
        const frame = decodedFrames[i];
        const lineElement = this.createLineElement(frame, firstFrameIndex + i);
        this.lineElements.push(lineElement);
        this.scrollContainer.appendChild(lineElement);
      }

      // Enforce max visible lines by removing oldest from DOM
      const excess = this.lineElements.length - this.config.maxVisibleFrames;
      for (let i = 0; i < excess; i++) {
        const removed = this.lineElements.shift();
        if (removed && removed.parentNode === this.scrollContainer) {
          this.scrollContainer.removeChild(removed);
        }
      }
    } else if (decodedFrames.length === 0 || this.visibleDomWasPruned(buffer)) {
      // Rebuild visible DOM from buffer (for scrollToFrame/jumpToLatest/history view)
      this.rebuildVisibleFrames(buffer);
    }

    this.updateStatus(buffer);

    // Auto-scroll to bottom if at latest
    if (buffer.atBottom) {
      this.scrollToBottom();
    }
  }

  /**
   * Detect when the currently rendered history has fallen out of retained buffer history.
   */
  private visibleDomWasPruned(buffer: ScrollbackBuffer): boolean {
    const firstLine = this.lineElements[0];
    if (!firstLine) return true;

    const firstIndex = Number(firstLine.dataset.lineIndex);
    return !Number.isFinite(firstIndex) || firstIndex < buffer.offset;
  }

  /**
   * Rebuild the DOM from the current visible frame window.
   */
  private rebuildVisibleFrames(buffer: ScrollbackBuffer): void {
    this.scrollContainer.innerHTML = "";
    this.lineElements = [];

    const window = this.getRenderableWindow(buffer);
    for (let i = 0; i < window.frames.length; i++) {
      const frame = window.frames[i];
      const lineElement = this.createLineElement(frame, window.offset + i);
      this.lineElements.push(lineElement);
      this.scrollContainer.appendChild(lineElement);
    }
  }

  /**
   * Pick the DOM-sized slice to render from the buffer window.
   */
  private getRenderableWindow(buffer: ScrollbackBuffer): { frames: DecodedFrame[]; offset: number } {
    if (buffer.visibleFrames.length <= this.config.maxVisibleFrames) {
      return { frames: buffer.visibleFrames, offset: buffer.offset };
    }

    let start = buffer.atBottom ? buffer.visibleFrames.length - this.config.maxVisibleFrames : 0;
    const focusIndex = this.pendingFocusFrameIndex;
    const windowEnd = buffer.offset + buffer.visibleFrames.length;
    if (focusIndex !== undefined && focusIndex >= buffer.offset && focusIndex < windowEnd) {
      const focusOffset = focusIndex - buffer.offset;
      const centeredStart = focusOffset - Math.floor(this.config.maxVisibleFrames / 2);
      start = Math.max(0, Math.min(centeredStart, buffer.visibleFrames.length - this.config.maxVisibleFrames));
    }

    return {
      frames: buffer.visibleFrames.slice(start, start + this.config.maxVisibleFrames),
      offset: buffer.offset + start,
    };
  }

  /**
   * Create a DOM element for a decoded frame line.
   */
  private createLineElement(frame: DecodedFrame, lineIndex: number): HTMLElement {
    const line = document.createElement("div");
    line.style.cssText = `
      padding: 2px 0;
      white-space: pre-wrap;
    `;

    // Apply ANSI color styling
    if (frame.spans.length > 0) {
      this.applySpanStyles(line, frame.text, frame.spans);
    } else {
      // Color based on frame kind
      const color = this.getFrameKindColor(frame.frame.frame_kind);
      line.style.color = color;
      line.textContent = frame.text;
    }

    // Add data attribute for search
    line.dataset.lineIndex = String(lineIndex);
    line.dataset.frameKind = frame.frame.frame_kind;

    return line;
  }

  /**
   * Apply span styles for ANSI-colored text.
   */
  private applySpanStyles(
    container: HTMLElement,
    text: string,
    spans: Array<{ start: number; styles: TextStyle[] }>,
  ): void {
    // For now, just display the text with basic styling
    // A full implementation would create span elements for each style range
    container.textContent = text;

    // Apply foreground color from first span if present
    if (spans.length > 0) {
      const firstSpan = spans[0];
      for (const style of firstSpan.styles) {
        if (typeof style === "object" && "type" in style && (style as ColorStyle).type === "foreground" && "color" in style) {
          container.style.color = (style as ColorStyle).color;
          break;
        }
      }
    }
  }

  /**
   * Get color for frame kind.
   */
  private getFrameKindColor(kind: string): string {
    switch (kind) {
      case "stderr":
        return "#f85149"; // Red
      case "log":
        return "#8b949e"; // Gray
      case "prompt":
        return "#58a6ff"; // Blue
      case "status":
        return "#d29922"; // Yellow
      default:
        return "#c9d1d9"; // Default text
    }
  }

  /**
   * Update status display.
   */
  private updateStatus(buffer: ScrollbackBuffer): void {
    const metrics = this.renderer.getMetrics();
    const fps = metrics.fps > 0 ? `${metrics.fps} fps` : "idle";
    const memory = this.formatMemory(metrics.memoryBytes);

    this.statusSpan.textContent = `${buffer.totalFrames} frames | ${fps} | ${memory} | ${buffer.visibleFrames.length} visible`;
  }

  /**
   * Format memory bytes to human-readable string.
   */
  private formatMemory(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  /**
   * Scroll to bottom of terminal output.
   */
  private scrollToBottom(): void {
    requestAnimationFrame(() => {
      this.scrollContainer.scrollTop = this.scrollContainer.scrollHeight;
    });
  }

  /**
   * Perform text search in terminal output.
   * Searches the full scrollback buffer history, not just visible DOM elements.
   */
  private performSearch(): void {
    this.searchTerm = this.searchInput.value.trim();

    // Clear previous search highlights
    this.clearSearchHighlights();

    if (!this.searchTerm) {
      this.searchResults = [];
      this.currentSearchIndex = 0;
      this.pendingFocusFrameIndex = undefined;
      return;
    }

    // Search the full scrollback buffer history via the renderer.
    const buffer = this.renderer.getBuffer();
    this.searchResults = searchText(buffer, this.searchTerm, false);

    // Highlight first result if it's in visible range
    if (this.searchResults.length > 0) {
      this.highlightCurrentSearchResult();
    }
  }

  /**
   * Clear search highlights.
   */
  private clearSearchHighlights(): void {
    for (const line of this.lineElements) {
      line.style.outline = "";
      line.style.backgroundColor = "";
    }
  }

  /**
   * Highlight current search result.
   * Scrolls the buffer to show the matching frame and highlights it in DOM.
   */
  private highlightCurrentSearchResult(): void {
    if (this.searchResults.length === 0) return;

    // Clear previous highlights
    this.clearSearchHighlights();

    // Highlight current result
    const currentIndex = this.currentSearchIndex % this.searchResults.length;
    const frameIndex = this.searchResults[currentIndex];
    this.pendingFocusFrameIndex = frameIndex;

    // Use renderer's scrollTo to bring the frame into view
    this.renderer.scrollToFrame(frameIndex);

    // Apply visual highlight to the matching DOM element after render.
    requestAnimationFrame(() => this.highlightLine(frameIndex));
  }

  /**
   * Highlight a specific global frame index if it is visible.
   */
  private highlightLine(frameIndex: number): void {
    const targetElement = this.lineElements.find((line) => line.dataset.lineIndex === String(frameIndex));
    if (!targetElement) return;

    targetElement.style.outline = "2px solid #58a6ff";
    targetElement.style.backgroundColor = "rgba(88, 166, 255, 0.2)";
    targetElement.scrollIntoView({ block: "center" });
  }

  /**
   * Copy terminal output to clipboard.
   * Uses the full scrollback buffer to avoid losing pruned data.
   */
  private copyToClipboard(): void {
    // Get text from full scrollback buffer, not just visible DOM
    const buffer = this.renderer.getBuffer();
    const text = buffer.allFrames
      .map((frame) => frame.text)
      .join("\n");

    navigator.clipboard.writeText(text).then(() => {
      const originalText = this.copyButton.textContent;
      this.copyButton.textContent = "Copied!";
      setTimeout(() => {
        this.copyButton.textContent = originalText;
      }, 1500);
    }).catch(() => {
      // Fallback for browsers that don't support clipboard API
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand("copy");
      document.body.removeChild(textarea);
    });
  }

  /**
   * Jump to latest output (bottom of scrollback).
   * Updates visible frames to show the most recent content.
   */
  private jumpToLatest(): void {
    this.pendingFocusFrameIndex = undefined;
    this.renderer.jumpToLatest();
  }

  /**
   * Dispose of the viewer.
   */
  dispose(): void {
    this.container.innerHTML = "";
    this.lineElements = [];
  }
}

/**
 * Create a terminal viewer instance.
 */
export function createTerminalViewer(
  renderer: TerminalRenderer,
  options: TerminalViewerOptions,
): TerminalViewer {
  return new TerminalViewer(renderer, options);
}
