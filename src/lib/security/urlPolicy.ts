const ALLOWED_OPEN_SCHEMES = new Set([
  "http:",
  "https:",
  "file:",
  // Cursor's MCP-install deep link, used by the AI Integrations setup surface.
  // The URL is app-constructed from the backend-resolved binary path, never
  // from untrusted content. (VS Code uses an https redirect, so no extra scheme.)
  "cursor:",
]);

/**
 * Add an `https://` scheme to a bare host/path so `new URL()` can parse it.
 * `example.com` and `example.com/x` become `https://…`; anything that already
 * carries a scheme (`http:`, `https:`, `mailto:`, `cursor:`, `file:`) is left
 * untouched. Mirrors the backend's `normalize_link_url`.
 */
export function normalizeUrl(raw: string): string {
  const t = raw.trim();
  if (!t) return t;
  // A leading `scheme:` (letters/digits/+-.) — but not a bare `host:port` — means
  // a scheme is already present. `://` is the unambiguous case.
  const hasScheme = /^[a-z][a-z0-9+.-]*:/i.test(t) && (t.includes("://") || !/^[^/]+:\d/.test(t));
  return hasScheme ? t : `https://${t}`;
}

export function assertSafeOpenUrl(raw: string): string {
  const parsed = new URL(normalizeUrl(raw));
  if (!ALLOWED_OPEN_SCHEMES.has(parsed.protocol)) {
    throw new Error(`Blocked unsupported URL scheme: ${parsed.protocol}`);
  }
  return parsed.toString();
}
