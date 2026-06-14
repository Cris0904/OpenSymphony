import type { RunAction, RunDetail, SafeActions, ActionReceipt } from "@opensymphony/gateway-schema";

/** Display model for a single action button in the run action bar. */
export interface ActionBarItem {
  action: RunAction;
  label: string;
  enabled: boolean;
  warning?: string;
}

/** Build action bar items from a run detail and safe actions. */
export function buildActionBarItems(run: RunDetail): ActionBarItem[] {
  const safe: SafeActions = run.safe_actions ?? {
    retry: true,
    cancel: true,
    rehydrate: true,
    detach: false,
  };
  const allowed = new Set(run.allowed_actions ?? []);
  const phase = run.liveness?.phase;

  const items: ActionBarItem[] = [];

  const push = (action: RunAction, label: string, requiresSafe: boolean) => {
    const allowedAction = allowed.has(action);
    const safeAction = safe[action as keyof SafeActions] ?? true;
    let enabled = allowedAction && (requiresSafe ? safeAction : true);
    let warning: string | undefined;
    if (!allowedAction) {
      enabled = false;
    } else if (!safeAction) {
      warning = `Unsafe to ${action} while run is ${phase ?? run.status}`;
    }
    // Duplicate-run retry prevention: retry on an active/owned run is unsafe
    // and should be blocked unless explicitly allowed by the safe_actions gate.
    if (action === "retry" && enabled && !safeAction) {
      enabled = false;
      warning = `Prevented duplicate-run retry: run is ${phase ?? run.status}`;
    }
    items.push({ action, label, enabled, warning });
  };

  push("retry", "Retry", true);
  push("cancel", "Cancel", true);
  push("rehydrate", "Rehydrate", true);
  push("detach", "Detach", false);
  push("comment", "Comment", false);
  push("create_followup", "Follow-up", false);
  push("open_workspace", "Workspace", false);
  push("debug", "Debug", false);
  return items;
}

/** Render the run action bar as a lightweight HTML string. */
export function renderActionBar(
  items: ActionBarItem[],
  opts?: { onAction?: (action: RunAction) => void },
): string {
  if (items.length === 0) {
    return `<div class="os-run-action-bar os-empty" data-testid="run-action-bar">No actions available</div>`;
  }
  const buttons = items
    .map((item) => {
      const warning = item.warning
        ? `<span class="os-action-warning" data-testid="action-warning" data-action="${item.action}">${escapeHtml(item.warning)}</span>`
        : "";
      return `<div class="os-action-item">
        <button class="os-run-action" data-testid="run-action-button" data-action="${item.action}" ${item.enabled ? "" : "disabled"}>${item.label}</button>
        ${warning}
      </div>`;
    })
    .join("");
  return `<div class="os-run-action-bar" data-testid="run-action-bar">${buttons}</div>`;
}

/** Render an action receipt as a lightweight HTML string. */
export function renderActionReceipt(receipt: ActionReceipt): string {
  const events = receipt.expected_events.length
    ? receipt.expected_events.map((e) => `<span class="os-expected-event">${escapeHtml(e)}</span>`).join(" ")
    : "none";
  return `<div class="os-action-receipt" data-testid="action-receipt" data-action-id="${receipt.action_id}" data-status="${receipt.status}">
    <span class="os-action-id">${receipt.action_id}</span>
    <span class="os-receipt-status os-receipt-status-${receipt.status}">${receipt.status}</span>
    <span class="os-expected-events">expected: ${events}</span>
    ${receipt.reason ? `<span class="os-receipt-reason">${escapeHtml(receipt.reason)}</span>` : ""}
  </div>`;
}

/** Render a compact audit trail entry from an action receipt or event summary. */
export function renderAuditTrailEntry(
  event: {
    timestamp: string;
    actor: string;
    action: string;
    target: string;
    status: string;
    details?: string;
  },
): string {
  return `<div class="os-audit-trail-entry" data-testid="audit-trail-entry" data-action="${event.action}" data-status="${event.status}">
    <span class="os-audit-timestamp">${event.timestamp}</span>
    <span class="os-audit-actor">${escapeHtml(event.actor)}</span>
    <span class="os-audit-action">${event.action}</span>
    <span class="os-audit-target">${escapeHtml(event.target)}</span>
    <span class="os-audit-status os-audit-status-${event.status}">${event.status}</span>
    ${event.details ? `<span class="os-audit-details">${escapeHtml(event.details)}</span>` : ""}
  </div>`;
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
