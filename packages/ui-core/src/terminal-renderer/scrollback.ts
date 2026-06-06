/**
 * Virtualized scrollback buffer for high-throughput terminal output.
 *
 * Maintains a fixed-size window over the decoded frames to prevent
 * unbounded memory growth while keeping scrollback stable.
 */

import type { DecodedFrame } from "./decoder.js";

export interface ScrollbackBuffer {
  /** Total number of frames ever received (including pruned ones). */
  totalFrames: number;

  /** Currently visible frames in the buffer. */
  visibleFrames: DecodedFrame[];

  /** Index of the first visible frame in the total sequence. */
  offset: number;

  /** Maximum number of frames to keep visible. */
  capacity: number;

  /** Whether we are at the bottom (latest output visible). */
  atBottom: boolean;
}

/**
 * Create a new scrollback buffer with the given capacity.
 * @param capacity Maximum number of visible frames to retain
 */
export function createScrollbackBuffer(capacity = 1000): ScrollbackBuffer {
  return {
    totalFrames: 0,
    visibleFrames: [],
    offset: 0,
    capacity,
    atBottom: true,
  };
}

/**
 * Append decoded frames to the scrollback buffer.
 * Prunes old frames if capacity is exceeded while maintaining stable scrollback.
 */
export function appendFrames(buffer: ScrollbackBuffer, frames: DecodedFrame[]): ScrollbackBuffer {
  if (frames.length === 0) return buffer;

  const newTotal = buffer.totalFrames + frames.length;
  let newVisible = [...buffer.visibleFrames, ...frames];
  let newOffset = buffer.offset;

  // Prune if we exceed capacity
  if (newVisible.length > buffer.capacity) {
    const excess = newVisible.length - buffer.capacity;
    newOffset += excess;
    newVisible = newVisible.slice(excess);
  }

  return {
    ...buffer,
    totalFrames: newTotal,
    visibleFrames: newVisible,
    offset: newOffset,
    atBottom: buffer.atBottom,
  };
}

/**
 * Scroll to a specific frame index (relative to totalFrames).
 * Returns the new buffer state with updated offset.
 */
export function scrollTo(buffer: ScrollbackBuffer, targetIndex: number): ScrollbackBuffer {
  const clampedTarget = Math.max(0, Math.min(targetIndex, buffer.totalFrames - 1));

  // Calculate the offset to show frames around the target
  const halfCapacity = Math.floor(buffer.capacity / 2);
  const newOffset = Math.max(0, clampedTarget - halfCapacity);

  // Slice visible frames from the full set (we only have visibleFrames)
  // In practice, we'd need the full frame array, but for the prototype we use
  // the visible frames and adjust the offset accordingly
  const newVisible = buffer.visibleFrames;

  return {
    ...buffer,
    offset: newOffset,
    visibleFrames: newVisible,
    atBottom: false,
  };
}

/**
 * Jump to the latest frame (bottom of scrollback).
 */
export function jumpToLatest(buffer: ScrollbackBuffer): ScrollbackBuffer {
  return {
    ...buffer,
    atBottom: true,
  };
}

/**
 * Search for text within the visible frames.
 * Returns indices of frames containing the search text.
 */
export function searchText(
  buffer: ScrollbackBuffer,
  query: string,
  caseSensitive = false,
): number[] {
  if (!query || query.length === 0) return [];

  const searchQuery = caseSensitive ? query : query.toLowerCase();
  const results: number[] = [];

  for (let i = 0; i < buffer.visibleFrames.length; i++) {
    const frame = buffer.visibleFrames[i];
    const frameText = caseSensitive ? frame.text : frame.text.toLowerCase();

    if (frameText.includes(searchQuery)) {
      results.push(buffer.offset + i);
    }
  }

  return results;
}

/**
 * Copy text from a range of frames to clipboard.
 */
export function copyFrameRange(
  buffer: ScrollbackBuffer,
  startIndex: number,
  endIndex: number,
): string {
  const startIdx = Math.max(0, startIndex - buffer.offset);
  const endIdx = Math.min(buffer.visibleFrames.length - 1, endIndex - buffer.offset);

  if (startIdx > endIdx || startIdx < 0 || endIdx >= buffer.visibleFrames.length) {
    return "";
  }

  const text = buffer.visibleFrames
    .slice(startIdx, endIdx + 1)
    .map((f) => f.text)
    .join("\n");

  return text;
}

/**
 * Get memory usage estimate for the buffer.
 */
export function estimateMemoryUsage(buffer: ScrollbackBuffer): number {
  // Rough estimate: each frame ~100 bytes average
  const frameSize = buffer.visibleFrames.length * 100;
  const textData = buffer.visibleFrames.reduce((sum, f) => sum + f.text.length * 2, 0); // UTF-16
  return frameSize + textData + 256; // Base overhead
}
