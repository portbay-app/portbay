import { defineConfig } from "vitepress";

const base = process.env.DOCS_BASE ?? "/";

export default defineConfig({
  title: "PortBay",
  description: "Local development environment manager for projects, ports, DNS, HTTPS, and sidecars.",
  lang: "en-US",
  base,
  cleanUrls: true,
  lastUpdated: true,
  head: [
    ["meta", { name: "theme-color", content: "#0b0f14" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:title", content: "PortBay Documentation" }],
    [
      "meta",
      {
        property: "og:description",
        content:
          "Install, configure, and operate PortBay for local projects, HTTPS, runtime services, and troubleshooting.",
      },
    ],
  ],
  markdown: {
    lineNumbers: true,
  },
  themeConfig: {
    logo: { src: "/logo.svg", alt: "PortBay" },
    siteTitle: "PortBay",
    search: {
      provider: "local",
    },
    nav: [
      { text: "Start", link: "/getting-started/" },
      { text: "Guides", link: "/guides/" },
      { text: "Reference", link: "/reference/cli" },
      { text: "AI Agents", link: "/agents/" },
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
