/**
 * RunStatusBadge component.
 *
 * Shared badge component for displaying run status colors.
 * Extracted to avoid duplication between Dashboard and RunDetail pages.
 */

import type { RunStatus } from "@opensymphony/gateway-schema";
import { RUN_STATUS_COLORS } from "../lib/ui-utils";

export function RunStatusBadge({ status }: { status: RunStatus }): React.ReactElement {
  const { bg, fg } = RUN_STATUS_COLORS[status] ?? RUN_STATUS_COLORS.unclaimed;

  return (
    <span
      style={{
        fontSize: "11px",
        fontWeight: 500,
        padding: "2px 8px",
        borderRadius: "10px",
        background: bg,
        color: fg,
        textTransform: "capitalize",
        whiteSpace: "nowrap",
        minWidth: "70px",
        textAlign: "center",
      }}
    >
      {status.replace("_", " ")}
    </span>
  );
}
