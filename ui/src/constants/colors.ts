/** Maps event categories to their CSS variable names */
export const CATEGORY_COLORS: Record<string, string> = {
  boot: "var(--event-boot)",
  error: "var(--event-error)",
  memory: "var(--event-memory)",
  ctp: "var(--event-ctp)",
  user: "var(--event-user)",
  default: "var(--event-default)",
};

/** Maps subsystem status to CSS variable names */
export const STATUS_COLORS: Record<string, string> = {
  Ready: "var(--status-ready)",
  Degraded: "var(--status-degraded)",
  Unknown: "var(--status-unknown)",
  Unavailable: "var(--status-unavailable)",
};

/** Maps toast types to their accent colors */
export const TOAST_TYPE_COLORS: Record<string, string> = {
  info: "var(--toast-info)",
  success: "var(--toast-success)",
  warning: "var(--toast-warning)",
  error: "var(--toast-error)",
};

/** Maps capability status to CSS variable names */
export const CAPABILITY_COLORS: Record<string, string> = {
  granted: "var(--capability-granted)",
  degraded: "var(--capability-degraded)",
  denied: "var(--capability-denied)",
};
