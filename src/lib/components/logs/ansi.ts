import Convert from "ansi-to-html";

const converter = new Convert({
  escapeXML: true,
  fg: "var(--color-fg-muted)",
  bg: "transparent",
  colors: {
    0: "var(--color-fg-subtle)",
    1: "var(--color-status-crashed)",
    2: "var(--color-status-running)",
    3: "var(--color-status-unhealthy)",
    4: "var(--color-accent)",
    5: "#c084fc",
    6: "var(--color-status-starting)",
    7: "var(--color-fg-muted)",
    8: "var(--color-fg-subtle)",
    9: "var(--color-status-crashed)",
    10: "var(--color-status-running)",
    11: "var(--color-status-unhealthy)",
    12: "var(--color-accent-hover)",
    13: "#d946ef",
    14: "var(--color-status-starting)",
    15: "var(--color-fg)",
  },
});

export function formatLogLine(raw: string): string {
  return converter.toHtml(raw || " ");
}
