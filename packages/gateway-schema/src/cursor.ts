/** Monotonic stream cursor for replay and resumable subscriptions. */
export interface StreamCursor {
  sequence: number;
  partition: string;
  timestamp_anchor?: number;
}

export function streamCursor(
  sequence: number,
  partition: string,
  timestamp_anchor?: number,
): StreamCursor {
  return { sequence, partition, timestamp_anchor };
}

/** Pagination cursor for detail reads. */
export interface PageCursor {
  page_token: string | null;
  page_size: number;
}

/** Return a cursor that requests the first page (null token = start). */
export function pageCursorFirst(pageSize: number): PageCursor {
  return { page_token: null, page_size: pageSize };
}
