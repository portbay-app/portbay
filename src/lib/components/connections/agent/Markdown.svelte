<!--
  Markdown — production markdown rendering for the Agent chat, the equivalent of
  Void's ChatMarkdownRender (markdown/ChatMarkdownRender.tsx). Parses GFM with
  `marked`, sanitizes with DOMPurify, and renders with our design tokens (we have
  no Tailwind-typography plugin, so the prose styles are hand-written below).

  - Links are routed through our Tauri `openUrl` (never navigated in-webview).
  - Fenced code gets a language label + Copy button.
  - Re-parses on every `source` change, so it streams cleanly token-by-token.

  SPA mode (`ssr = false`) means this only ever runs in the browser, so the
  static DOMPurify import is safe.
-->
<script lang="ts" module>
  import hljs from "highlight.js/lib/common";
  import { marked } from "marked";

  function escapeHtml(s: string): string {
    return s
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // Configure marked ONCE for the whole app (module scope, not per instance):
  // GFM, single-newline breaks (chat text expects them), and a code renderer
  // that emits our header bar (lang + Copy). The copy button carries no data —
  // the click handler reads the sibling <pre>'s text.
  marked.use({
    gfm: true,
    breaks: true,
    renderer: {
      code({ text, lang }: { text: string; lang?: string }) {
        const language = (lang || "").split(/\s+/)[0];
        // Highlight only when a known language is labelled — auto-detection is
        // expensive to re-run per streamed token and unreliable on partial code.
        const inner =
          language && hljs.getLanguage(language)
            ? hljs.highlight(text, { language, ignoreIllegals: true }).value
            : escapeHtml(text);
        const shown = language || "text";
        return (
          `<div class="md-code">` +
          `<div class="md-code-head"><span class="md-code-lang">${escapeHtml(shown)}</span>` +
          `<button type="button" class="md-copy" aria-label="Copy code">Copy</button></div>` +
          `<pre><code class="hljs">${inner}</code></pre>` +
          `</div>`
        );
      },
    },
  });
</script>

<script lang="ts">
  import DOMPurify from "dompurify";

  import { openUrl } from "$lib/security/openUrl";

  let { source, small = false }: { source: string; small?: boolean } = $props();

  const html = $derived.by(() => {
    const raw = marked.parse(source ?? "", { async: false }) as string;
    return DOMPurify.sanitize(raw, { ADD_ATTR: ["target", "rel"] });
  });

  /** Delegate clicks: route links through Tauri openUrl, and copy code blocks. */
  function onClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    const link = target.closest("a[href]") as HTMLAnchorElement | null;
    if (link) {
      e.preventDefault();
      const href = link.getAttribute("href");
      if (href) void openUrl(href);
      return;
    }
    const copy = target.closest(".md-copy") as HTMLButtonElement | null;
    if (copy) {
      const pre = copy.closest(".md-code")?.querySelector("pre code");
      const text = pre?.textContent ?? "";
      void navigator.clipboard.writeText(text);
      const prev = copy.textContent;
      copy.textContent = "Copied";
      setTimeout(() => {
        copy.textContent = prev;
      }, 1200);
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="md {small ? 'md-small' : ''}" onclick={onClick}>
  {@html html}
</div>

<style>
  .md {
    color: var(--color-fg);
    font-size: 12.5px;
    line-height: 1.55;
    word-break: break-word;
    overflow-wrap: anywhere;
  }
  .md-small {
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--color-fg-muted);
  }

  /* {@html} content is unscoped — target it with :global within .md. */
  .md :global(> :first-child) {
    margin-top: 0;
  }
  .md :global(> :last-child) {
    margin-bottom: 0;
  }
  .md :global(p),
  .md :global(ul),
  .md :global(ol),
  .md :global(blockquote),
  .md :global(table) {
    margin: 0.5em 0;
  }
  .md :global(h1),
  .md :global(h2),
  .md :global(h3),
  .md :global(h4) {
    margin: 0.8em 0 0.4em;
    font-weight: 600;
    line-height: 1.3;
  }
  .md :global(h1) {
    font-size: 1.25em;
  }
  .md :global(h2) {
    font-size: 1.15em;
  }
  .md :global(h3) {
    font-size: 1.05em;
  }
  .md :global(ul),
  .md :global(ol) {
    padding-left: 1.4em;
  }
  .md :global(ul) {
    list-style: disc;
  }
  .md :global(ol) {
    list-style: decimal;
  }
  .md :global(li) {
    margin: 0.15em 0;
  }
  .md :global(li > p) {
    margin: 0.15em 0;
  }
  .md :global(a) {
    color: var(--color-accent);
    text-decoration: none;
    cursor: pointer;
  }
  .md :global(a:hover) {
    text-decoration: underline;
  }
  .md :global(strong) {
    font-weight: 600;
    color: var(--color-fg);
  }
  .md :global(em) {
    font-style: italic;
  }
  .md :global(hr) {
    border: none;
    border-top: 1px solid var(--color-border);
    margin: 1em 0;
  }
  .md :global(blockquote) {
    border-left: 2px solid var(--color-border-strong);
    padding-left: 0.75em;
    color: var(--color-fg-muted);
  }
  /* inline code */
  .md :global(:not(pre) > code) {
    font-family:
      ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
    font-size: 0.9em;
    background: var(--color-surface-2);
    border-radius: 4px;
    padding: 0.1em 0.35em;
  }
  /* fenced code block */
  .md :global(.md-code) {
    margin: 0.6em 0;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    overflow: hidden;
    background: var(--color-surface-2);
  }
  .md :global(.md-code-head) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.2em 0.6em;
    border-bottom: 1px solid var(--color-border);
    background: color-mix(in oklch, var(--color-surface-2) 70%, var(--color-surface) 30%);
  }
  .md :global(.md-code-lang) {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 10.5px;
    color: var(--color-fg-subtle);
    text-transform: lowercase;
  }
  .md :global(.md-copy) {
    font-size: 10.5px;
    color: var(--color-fg-subtle);
    padding: 0.1em 0.4em;
    border-radius: 4px;
    cursor: pointer;
  }
  .md :global(.md-copy:hover) {
    color: var(--color-fg);
    background: var(--color-surface);
  }
  .md :global(.md-code pre) {
    margin: 0;
    padding: 0.6em 0.7em;
    overflow-x: auto;
  }
  .md :global(.md-code pre code) {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--color-fg);
  }

  /* highlight.js token theme — mapped onto our semantic color vars so it tracks
     light / dark / high-contrast automatically (no per-theme stylesheet). */
  .md :global(.hljs) {
    color: var(--color-fg);
    background: transparent;
  }
  .md :global(.hljs-comment),
  .md :global(.hljs-quote) {
    color: var(--color-fg-subtle);
    font-style: italic;
  }
  .md :global(.hljs-keyword),
  .md :global(.hljs-selector-tag),
  .md :global(.hljs-built_in),
  .md :global(.hljs-literal),
  .md :global(.hljs-doctag),
  .md :global(.hljs-meta),
  .md :global(.hljs-meta .hljs-keyword) {
    color: var(--color-accent);
  }
  .md :global(.hljs-string),
  .md :global(.hljs-regexp),
  .md :global(.hljs-addition),
  .md :global(.hljs-selector-attr),
  .md :global(.hljs-selector-pseudo) {
    color: var(--color-status-running);
  }
  .md :global(.hljs-number),
  .md :global(.hljs-symbol),
  .md :global(.hljs-bullet),
  .md :global(.hljs-link) {
    color: var(--color-status-port-conflict);
  }
  .md :global(.hljs-title),
  .md :global(.hljs-title.function_),
  .md :global(.hljs-section),
  .md :global(.hljs-name) {
    color: var(--color-status-starting);
  }
  .md :global(.hljs-type),
  .md :global(.hljs-class .hljs-title),
  .md :global(.hljs-title.class_),
  .md :global(.hljs-attr),
  .md :global(.hljs-attribute),
  .md :global(.hljs-variable),
  .md :global(.hljs-template-variable),
  .md :global(.hljs-property),
  .md :global(.hljs-params) {
    color: var(--color-status-unhealthy);
  }
  .md :global(.hljs-deletion) {
    color: var(--color-status-crashed);
  }
  .md :global(.hljs-tag),
  .md :global(.hljs-punctuation),
  .md :global(.hljs-operator) {
    color: var(--color-fg-muted);
  }
  .md :global(.hljs-emphasis) {
    font-style: italic;
  }
  .md :global(.hljs-strong) {
    font-weight: 600;
  }
  .md :global(table) {
    border-collapse: collapse;
    display: block;
    overflow-x: auto;
  }
  .md :global(th),
  .md :global(td) {
    border: 1px solid var(--color-border);
    padding: 0.3em 0.6em;
    text-align: left;
  }
  .md :global(th) {
    background: var(--color-surface-2);
    font-weight: 600;
  }
  .md :global(img) {
    max-width: 100%;
    border-radius: 6px;
  }
</style>
