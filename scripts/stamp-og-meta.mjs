#!/usr/bin/env node
/**
 * Per-route static meta stamping for the hosted web simulator (try.portbay.app).
 *
 * Why this exists: the app is a pure client-rendered SPA (`ssr = false` in
 * src/routes/+layout.ts) served by adapter-static with a single `index.html`
 * fallback. Link unfurlers (WhatsApp, Slack, iMessage, Facebook, LinkedIn,
 * Twitter/X, Discord) do NOT execute JavaScript — they read the raw HTML
 * response. So the `<svelte:head>` tags injected at runtime are invisible to
 * them, and every URL would otherwise unfurl with the same bare
 * `<title>PortBay</title>` from app.html.
 *
 * This post-build step writes a static `build/<route>/index.html` for each
 * known route — a verbatim copy of the SPA fallback with route-specific
 * <title> / description / canonical / Open Graph / Twitter tags stamped into
 * the <head>. Crawlers get correct per-page metadata; the SPA still boots and
 * hydrates identically regardless of which HTML document loaded it.
 *
 * Run automatically by `pnpm build:web` (see package.json). Idempotent.
 */
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BUILD = join(ROOT, "build");
const ORIGIN = "https://try.portbay.app";

// The shared social image until per-page art lands (see the Obsidian image-gen
// cards). Curated pages override `ogImage` with their own 1200x630 file under
// /og/; everything else falls back to this existing hero.
const DEFAULT_OG = "/og-image.png";

/**
 * Route → unfurl metadata. `path` is the URL path (no trailing slash except
 * root). Pages without a dedicated `ogImage` inherit DEFAULT_OG. Keep titles
 * under ~60 chars and descriptions under ~155 so they don't truncate in
 * previews.
 */
const ROUTES = [
  {
    path: "/",
    title: "PortBay — Local development environment manager",
    description:
      "Manage every local dev project behind clean .test domains with automatic HTTPS and one-click start/stop — no Docker, no config files. Try the live demo.",
    ogImage: DEFAULT_OG,
  },
  {
    path: "/tasks",
    title: "Task Board & AI Agents — PortBay",
    description:
      "A per-project Kanban board your AI coding agents can read and drive over MCP. Hand off work to Claude, Codex, or Cursor and watch cards move. Live demo.",
    ogImage: "/og/tasks.png",
  },
  {
    path: "/databases",
    title: "Databases — PortBay",
    description:
      "Provision MySQL, Postgres, Redis, and more per project with one click — managed engines, connection details, backups and restore. Try it in the browser.",
    ogImage: "/og/databases.png",
  },
  {
    path: "/domains",
    title: "Domains & HTTPS — PortBay",
    description:
      "Give every project a clean .test domain with locally-trusted HTTPS — no hosts-file editing, no certificate wrangling. Explore the live demo.",
    ogImage: "/og/domains.png",
  },
  {
    path: "/web-servers",
    title: "Web Servers — PortBay",
    description:
      "Run Caddy, nginx, or Apache per project with sane defaults and editable raw config. Switch servers without touching system installs. Live demo.",
    ogImage: "/og/web-servers.png",
  },
  {
    path: "/dns",
    title: "DNS — PortBay",
    description:
      "Wildcard local DNS resolution for your .test projects via a managed dnsmasq resolver — inspect records and resolver status. Try the interactive demo.",
    ogImage: "/og/dns.png",
  },
  {
    path: "/inspector",
    title: "HTTP Inspector — PortBay",
    description:
      "Watch live HTTP traffic through your local projects — requests, responses, timings — straight from the proxy. Explore it in the browser demo.",
    ogImage: "/og/inspector.png",
  },
  {
    path: "/tunnels",
    title: "Cloudflare Tunnels — PortBay",
    description:
      "Share any local project on a public HTTPS URL through Cloudflare Tunnels — per-project, honest lifecycle, no port forwarding. Try the live demo.",
    ogImage: "/og/tunnels.png",
  },
  // ── Non-curated routes: per-page text, shared hero image ──────────────────
  {
    path: "/certificates",
    title: "Certificates — PortBay",
    description:
      "Locally-trusted TLS certificates for your .test projects via mkcert — issued and renewed automatically. Explore the live PortBay demo.",
  },
  {
    path: "/languages",
    title: "Languages & Runtimes — PortBay",
    description:
      "Detect-first management of PHP, Node, Bun and more — per-project versions without global installs. Try the interactive PortBay demo.",
  },
  {
    path: "/services",
    title: "Services — PortBay",
    description:
      "See every sidecar and project process at a glance with live health and one-click start/stop. Explore the live PortBay demo.",
  },
  {
    path: "/logs",
    title: "Logs — PortBay",
    description:
      "Tail combined project and sidecar logs with ANSI color and search, all in one place. Try the interactive PortBay demo.",
  },
  {
    path: "/sandbox",
    title: "Sandboxed Projects — PortBay",
    description:
      "Run projects in isolated sandboxes with their own runtimes and environment. Explore the live PortBay demo.",
  },
  {
    path: "/download",
    title: "Download PortBay for macOS",
    description:
      "Download PortBay — the local development environment manager for macOS. Clean .test domains, automatic HTTPS, one-click start/stop.",
  },
  {
    path: "/settings",
    title: "Settings — PortBay",
    description:
      "Configure default web server, domain suffix, AI integrations, appearance and more. Explore the live PortBay demo.",
  },
];

const escape = (s) =>
  String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

/**
 * Resolve a route's OG image. Curated pages point at per-page art under /og/;
 * until those files are generated and shipped (see the Obsidian image-gen
 * cards), the file won't be in the build, so we fall back to the shipped hero
 * rather than emit a 404'ing image URL that unfurls as a broken preview.
 */
function resolveImage(ogImage) {
  const rel = ogImage ?? DEFAULT_OG;
  return existsSync(join(BUILD, rel.replace(/^\//, ""))) ? rel : DEFAULT_OG;
}

/** Build the <head> meta block for one route. */
function metaBlock({ path, title, description, ogImage }) {
  const url = path === "/" ? `${ORIGIN}/` : `${ORIGIN}${path}`;
  const image = `${ORIGIN}${resolveImage(ogImage)}`;
  const t = escape(title);
  const d = escape(description);
  return [
    `<title>${t}</title>`,
    `<meta name="description" content="${d}" />`,
    `<link rel="canonical" href="${url}" />`,
    `<meta property="og:type" content="website" />`,
    `<meta property="og:site_name" content="PortBay" />`,
    `<meta property="og:url" content="${url}" />`,
    `<meta property="og:title" content="${t}" />`,
    `<meta property="og:description" content="${d}" />`,
    `<meta property="og:image" content="${image}" />`,
    `<meta property="og:image:width" content="1200" />`,
    `<meta property="og:image:height" content="630" />`,
    `<meta name="twitter:card" content="summary_large_image" />`,
    `<meta name="twitter:title" content="${t}" />`,
    `<meta name="twitter:description" content="${d}" />`,
    `<meta name="twitter:image" content="${image}" />`,
  ].join("\n    ");
}

const fallbackPath = join(BUILD, "index.html");
let fallback;
try {
  fallback = readFileSync(fallbackPath, "utf8");
} catch {
  console.error(
    `[stamp-og-meta] ${fallbackPath} not found — run the web build first (pnpm build:web).`,
  );
  process.exit(1);
}

const NEEDLE = "<title>PortBay</title>";
if (!fallback.includes(NEEDLE)) {
  console.error(
    `[stamp-og-meta] could not find ${NEEDLE} in build/index.html — did app.html change?`,
  );
  process.exit(1);
}

let stamped = 0;
for (const route of ROUTES) {
  const html = fallback.replace(NEEDLE, metaBlock(route));
  const outPath =
    route.path === "/"
      ? fallbackPath
      : join(BUILD, route.path.replace(/^\//, ""), "index.html");
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, html, "utf8");
  stamped += 1;
}

console.log(
  `[stamp-og-meta] stamped per-route meta into ${stamped} HTML document(s).`,
);
