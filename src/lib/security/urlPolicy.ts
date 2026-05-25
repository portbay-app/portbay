const ALLOWED_OPEN_SCHEMES = new Set(["http:", "https:", "file:"]);

export function assertSafeOpenUrl(raw: string): string {
  const parsed = new URL(raw);
  if (!ALLOWED_OPEN_SCHEMES.has(parsed.protocol)) {
    throw new Error(`Blocked unsupported URL scheme: ${parsed.protocol}`);
  }
  return parsed.toString();
}
