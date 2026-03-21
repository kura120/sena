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
