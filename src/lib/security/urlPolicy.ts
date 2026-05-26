const ALLOWED_OPEN_SCHEMES = new Set([
  "http:",
  "https:",
  "file:",
  // Cursor's MCP-install deep link, used by the AI Integrations setup surface.
  // The URL is app-constructed from the backend-resolved binary path, never
  // from untrusted content. (VS Code uses an https redirect, so no extra scheme.)
  "cursor:",
]);

export function assertSafeOpenUrl(raw: string): string {
  const parsed = new URL(raw);
  if (!ALLOWED_OPEN_SCHEMES.has(parsed.protocol)) {
    throw new Error(`Blocked unsupported URL scheme: ${parsed.protocol}`);
  }
  return parsed.toString();
}
