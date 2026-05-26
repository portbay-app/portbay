/**
 * Client-side OS detection + download metadata for the marketing/web-demo
 * download surfaces (TopBar button + /download page). Web-only — the desktop
 * app never shows these.
 *
 * macOS is the only built target today; Windows + Linux are listed as
 * "coming soon" so the download page reads as a real cross-platform product.
 */
export type DesktopOS = "macos" | "windows" | "linux" | "unknown";

/** GitHub releases — the latest release page (lists every asset). */
export const RELEASES_URL =
  "https://github.com/portbay-app/portbay/releases/latest";

/** Stable, version-independent direct link to the macOS DMG. The release
 *  workflow attaches a fixed-name copy (`PortBay-macos-arm64.dmg`) alongside
 *  the versioned one, so this resolves to the newest build once published. */
export const MACOS_DMG_URL =
  "https://github.com/portbay-app/portbay/releases/latest/download/PortBay-macos-arm64.dmg";

export interface PlatformMeta {
  os: DesktopOS;
  /** Display name, e.g. "macOS". */
  label: string;
  /** Installer extension shown in copy, e.g. ".dmg". */
  ext: string;
  /** Whether a build exists to download today. */
  available: boolean;
  /** Direct download URL when available, else the releases page. */
  href: string;
  /** Short note for unavailable platforms. */
  note?: string;
}

export const PLATFORMS: Record<Exclude<DesktopOS, "unknown">, PlatformMeta> = {
  macos: {
    os: "macos",
    label: "macOS",
    ext: ".dmg",
    available: true,
    href: MACOS_DMG_URL,
  },
  windows: {
    os: "windows",
    label: "Windows",
    ext: ".exe",
    available: false,
    href: RELEASES_URL,
    note: "Coming soon",
  },
  linux: {
    os: "linux",
    label: "Linux",
    ext: ".AppImage",
    available: false,
    href: RELEASES_URL,
    note: "Coming soon",
  },
};

/** Best-effort OS detection from the user agent. Marketing-grade, not security. */
export function detectOS(): DesktopOS {
  if (typeof navigator === "undefined") return "unknown";
  const ua = (navigator.userAgent || "").toLowerCase();
  // iPadOS reports as Mac; treat all Apple platforms as macOS for the download.
  if (/macintosh|mac os x|iphone|ipad|ipod/.test(ua)) return "macos";
  if (/windows|win32|win64/.test(ua)) return "windows";
  if (/linux|x11|ubuntu|fedora|debian/.test(ua)) return "linux";
  return "unknown";
}

/** Metadata for the detected OS; falls back to macOS (the only built target)
 *  when the OS is unknown, so the primary CTA is always actionable. */
export function detectedPlatform(): PlatformMeta {
  const os = detectOS();
  if (os === "unknown") return PLATFORMS.macos;
  return PLATFORMS[os];
}
