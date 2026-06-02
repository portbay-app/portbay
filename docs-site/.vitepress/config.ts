import { existsSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitepress";

const base = process.env.DOCS_BASE ?? "/";

const OG_ORIGIN = "https://docs.portbay.app";
const PUBLIC_DIR = fileURLToPath(new URL("../public", import.meta.url));
// Existing shipped image, used until the cohesive /og/ art is generated (see
// the Obsidian image-gen cards) so previews never unfurl a broken image.
const FALLBACK_OG = "/screenshots/projects-dark.png";

// First path segment of a page → its section Open Graph image (1200x630, under
// docs-site/public/og/). Pages in any other section use the docs home image.
const DEFAULT_DOCS_OG = "/og/docs-home.png";
const SECTION_OG: Record<string, string> = {
  "getting-started": "/og/docs-getting-started.png",
  guides: "/og/docs-guides.png",
  reference: "/og/docs-reference.png",
  agents: "/og/docs-agents.png",
  comparisons: "/og/docs-comparisons.png",
};

/** Absolute image URL, falling back to a shipped image if the art isn't built yet. */
function ogImageUrl(rel: string): string {
  const resolved = existsSync(join(PUBLIC_DIR, rel.replace(/^\//, "")))
    ? rel
    : FALLBACK_OG;
  return `${OG_ORIGIN}${resolved}`;
}

// Social-preview length limits. Beyond these, unfurlers (WhatsApp, Slack,
// iMessage, X, LinkedIn, Facebook, Discord) truncate with an ellipsis, so we
// keep og:/twitter: tags inside them — fitting in full rather than being cut.
const SOCIAL_TITLE_MAX = 60;
const SOCIAL_DESC_MAX = 155;

/**
 * Trim to `max` without cutting a word. Prefers ending on a sentence boundary
 * inside the window; else falls back to the last whole word + an ellipsis.
 * Returns the input untouched when it already fits.
 */
function clip(input: string, max: number): string {
  const s = input.trim();
  if (s.length <= max) return s;
  const window = s.slice(0, max);
  const sentenceEnd = Math.max(
    window.lastIndexOf(". "),
    window.lastIndexOf("? "),
    window.lastIndexOf("! "),
  );
  if (sentenceEnd >= max * 0.55) return window.slice(0, sentenceEnd + 1).trim();
  const wordEnd = window.lastIndexOf(" ");
  return `${window.slice(0, wordEnd > 0 ? wordEnd : max).trim()}…`;
}

/**
 * A concise social title: the part before the first " — " (drops long SEO
 * tails) and no " — PortBay Docs" suffix, since og:site_name already carries
 * the brand. A page may override with frontmatter `ogTitle`. Hard-capped as a
 * final guard.
 */
function socialTitle(rawTitle: string, override?: unknown): string {
  if (typeof override === "string" && override.trim()) {
    return clip(override, SOCIAL_TITLE_MAX);
  }
  const core = rawTitle.split(" — ")[0];
  return clip(core, SOCIAL_TITLE_MAX);
}

export default defineConfig({
  title: "PortBay",
  description: "Local development environment manager for projects, ports, DNS, HTTPS, and sidecars.",
  lang: "en-US",
  base,
  cleanUrls: true,
  lastUpdated: true,
  sitemap: {
    hostname: "https://docs.portbay.app",
  },
  // Per-page social metadata. VitePress prerenders every page to static HTML,
  // so crawlers DO see these — but only if they're page-specific. We derive
  // title/description from the page itself and pick the section image, then
  // append the og:* / twitter:* tags to the page's own <head>.
  transformPageData(pageData) {
    const isHome = pageData.relativePath === "index.md";
    const title = isHome
      ? "PortBay Documentation"
      : socialTitle(pageData.title, pageData.frontmatter.ogTitle);
    const description = clip(
      pageData.description ||
        (pageData.frontmatter.description as string | undefined) ||
        "Install, configure, and operate PortBay for local projects, HTTPS, runtime services, and troubleshooting.",
      SOCIAL_DESC_MAX,
    );
    const section = pageData.relativePath.split("/")[0];
    const image = ogImageUrl(SECTION_OG[section] ?? DEFAULT_DOCS_OG);
    const path = pageData.relativePath
      .replace(/(^|\/)index\.md$/, "$1")
      .replace(/\.md$/, "");
    const url = `${OG_ORIGIN}/${path}`;

    pageData.frontmatter.head ??= [];
    pageData.frontmatter.head.push(
      ["meta", { property: "og:title", content: title }],
      ["meta", { property: "og:description", content: description }],
      ["meta", { property: "og:url", content: url }],
      ["meta", { property: "og:image", content: image }],
      ["meta", { name: "twitter:title", content: title }],
      ["meta", { name: "twitter:description", content: description }],
      ["meta", { name: "twitter:image", content: image }],
    );
  },
  head: [
    ["link", { rel: "icon", type: "image/png", href: "/favicon.png" }],
    ["link", { rel: "apple-touch-icon", sizes: "180x180", href: "/apple-touch-icon.png" }],
    ["meta", { name: "theme-color", content: "#0b0f14" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:site_name", content: "PortBay" }],
    ["meta", { name: "twitter:card", content: "summary_large_image" }],
    // og:title / og:description / og:url / og:image and their twitter:*
    // equivalents are emitted PER PAGE by transformPageData below, so each doc
    // unfurls with its own title, description, and section image instead of one
    // global pair.
  ],
  markdown: {
    lineNumbers: true,
  },
  themeConfig: {
    logo: { src: "/portbay-logo.png", alt: "PortBay" },
    siteTitle: "PortBay",
    search: {
      provider: "local",
    },
    nav: [
      { text: "Start", link: "/getting-started/" },
      { text: "Guides", link: "/guides/" },
      { text: "Reference", link: "/reference/cli" },
      { text: "AI Agents", link: "/agents/" },
      { text: "Compare", link: "/comparisons/" },
      { text: "Architecture", link: "/architecture/" },
      { text: "Pro", link: "/pro/" },
      { text: "Troubleshooting", link: "/troubleshooting/" },
    ],
    sidebar: [
      {
        text: "Getting Started",
        items: [
          { text: "Overview", link: "/getting-started/" },
          { text: "Install", link: "/getting-started/install" },
          { text: "Linux Support", link: "/getting-started/linux" },
          { text: "First Run", link: "/getting-started/first-run" },
          { text: "Add a Project", link: "/getting-started/add-project" },
        ],
      },
      {
        text: "Guides",
        items: [
          { text: "Overview", link: "/guides/" },
          { text: "Caddy and HTTPS", link: "/guides/caddy-https" },
          { text: "Custom Domain Suffix", link: "/guides/custom-domain-suffix" },
          { text: "PHP Setup", link: "/guides/php-setup" },
          { text: "Environment Variables", link: "/guides/environment-variables" },
          { text: "Project Groups", link: "/guides/project-groups" },
          { text: "CLI Usage", link: "/guides/cli-usage" },
        ],
      },
      {
        text: "Features",
        items: [
          { text: "Task Board & Agents", link: "/guides/task-board" },
          { text: "HTTP Inspector", link: "/guides/http-inspector" },
          { text: "Databases", link: "/guides/databases" },
          { text: "Languages & Runtimes", link: "/guides/languages" },
          { text: "Stack Recipes", link: "/guides/recipes" },
          { text: "Mailpit", link: "/guides/mailpit" },
          { text: "Cloudflare Tunnels", link: "/guides/tunnels" },
          { text: "SSH Workspace", link: "/guides/ssh-tunnels" },
          { text: "Sandboxed Projects", link: "/guides/sandbox" },
        ],
      },
      {
        text: "The App",
        items: [
          { text: "Command Palette", link: "/guides/command-palette" },
          { text: "Log Viewer", link: "/guides/log-viewer" },
          { text: "Tray Mode", link: "/guides/tray-mode" },
        ],
      },
      {
        text: "Reference",
        items: [
          { text: "CLI", link: "/reference/cli" },
          { text: "Registry Schema", link: "/reference/registry-schema" },
          { text: "Keyboard Shortcuts", link: "/reference/keyboard-shortcuts" },
          { text: "Capabilities", link: "/reference/capabilities" },
        ],
      },
      {
        text: "AI Agents (MCP)",
        items: [
          { text: "Overview", link: "/agents/" },
          { text: "Tool Reference", link: "/agents/tools" },
        ],
      },
      {
        text: "Comparisons",
        items: [
          { text: "Overview", link: "/comparisons/" },
          { text: "vs Laravel Herd", link: "/comparisons/portbay-vs-laravel-herd" },
          { text: "vs ServBay", link: "/comparisons/portbay-vs-servbay" },
          { text: "vs MAMP / XAMPP", link: "/comparisons/portbay-vs-mamp" },
          { text: "vs Docker / OrbStack", link: "/comparisons/portbay-vs-docker" },
          { text: "vs Laravel Valet", link: "/comparisons/portbay-vs-laravel-valet" },
          { text: "vs DDEV", link: "/comparisons/portbay-vs-ddev" },
          { text: "vs Local", link: "/comparisons/portbay-vs-local" },
        ],
      },
      {
        text: "Architecture",
        items: [{ text: "System Design", link: "/architecture/" }],
      },
      {
        text: "Pro",
        items: [{ text: "PortBay Pro", link: "/pro/" }],
      },
      {
        text: "Troubleshooting",
        items: [{ text: "Error Codes", link: "/troubleshooting/" }],
      },
      {
        text: "Migration Guides",
        items: [
          { text: "ServBay", link: "/migrations/servbay" },
          { text: "Laravel Herd", link: "/migrations/laravel-herd" },
          { text: "MAMP", link: "/migrations/mamp" },
        ],
      },
      {
        text: "Project",
        items: [{ text: "Contributing", link: "/contributing" }],
      },
    ],
    socialLinks: [
      { icon: "github", link: "https://github.com/portbay-app/portbay" },
    ],
    footer: {
      message: "PortBay is pre-MVP software. Use the docs as an operating guide, not a stability guarantee.",
      copyright: "Released under the MIT License.",
    },
  },
});
