import { defineConfig } from "vitepress";

const base = process.env.DOCS_BASE ?? "/";

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
  head: [
    ["link", { rel: "icon", type: "image/png", href: "/favicon.png" }],
    ["link", { rel: "apple-touch-icon", sizes: "180x180", href: "/apple-touch-icon.png" }],
    ["meta", { name: "theme-color", content: "#0b0f14" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:site_name", content: "PortBay" }],
    ["meta", { property: "og:title", content: "PortBay Documentation" }],
    [
      "meta",
      {
        property: "og:description",
        content:
          "Install, configure, and operate PortBay for local projects, HTTPS, runtime services, and troubleshooting.",
      },
    ],
    ["meta", { property: "og:url", content: "https://docs.portbay.app/" }],
    ["meta", { property: "og:image", content: "https://docs.portbay.app/screenshots/projects-dark.png" }],
    ["meta", { name: "twitter:card", content: "summary_large_image" }],
    ["meta", { name: "twitter:title", content: "PortBay Documentation" }],
    [
      "meta",
      {
        name: "twitter:description",
        content:
          "Install, configure, and operate PortBay for local projects, HTTPS, runtime services, and troubleshooting.",
      },
    ],
    ["meta", { name: "twitter:image", content: "https://docs.portbay.app/screenshots/projects-dark.png" }],
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
