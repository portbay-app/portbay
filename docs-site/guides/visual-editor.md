---
title: Visual Editor Precise Editing — Dev-time Source Location Map
description: Enable precise editing in PortBay's visual editor by stamping each rendered element with its authored source location, so visual edits resolve to the exact file, line, and column instead of a best-effort text search.
---

# Visual Editor — Precise Editing

PortBay's visual editor lets you click an element on your running `.test` site and edit its text, classes, and styles, writing the change back into your real source files. By default it resolves a clicked element back to source with a **text search** — it looks for the rendered text or `class=` value in your project files and patches it when it finds exactly one match.

That works well for static HTML and unique CSS selectors, but it has hard limits:

- React `className` can't be written back (the rewriter only handles `class=`).
- Vue/Svelte **scoped** styles drop to an override stylesheet instead of the real rule.
- **Dynamic / loop** content (`.map()`, `v-for`, `{#each}`) is ambiguous — N rendered copies look identical to a text search, so the edit is refused.
- Repeated text across the page can resolve to the wrong element.

**Precise editing** removes those limits. A small dev-time build plugin stamps each rendered element with its authored source location:

```html
<button data-pb-loc="src/components/Hero.vue:42:7">Get started</button>
```

PortBay reads that attribute, opens the exact file at that line, verifies the element still matches what you clicked, and patches that one element — so repeated `v-for` / `{#each}` items and scoped component styles resolve deterministically.

## Dev-only, by design

The `data-pb-loc` attribute is emitted **only in development**. It never reaches a production build, so there's no DOM bloat and no leaking of local file paths. The gate is `NODE_ENV !== 'production'`; set `PORTBAY_LOC=1` to force it on, or `PORTBAY_LOC=0` to force it off.

## Is it on?

For **Astro, Vue and Svelte projects running on Vite, it is already on.** PortBay
launches your dev server, so it adds its own source-location plugin to that
launch from a config it generates outside your project. There is nothing to
install and nothing to paste, and your `package.json`, `node_modules` and config
files are byte-identical with or without PortBay — remove PortBay and not one
byte of your project changes.

Open the edit bar on your site to see the state:

- **Precise** (green dot) — elements are stamped; edits resolve to the exact
  source line.
- **Precise: off** (amber dot) — click the chip and PortBay tells you exactly
  what is degraded for your stack, and why.

## Where it is on, and where it is not

PortBay's stamper reads markup textually, so it covers the file types whose
markup is separate from their code.

| Stack | Precise editing | How |
|---|---|---|
| Astro | ✅ | PortBay injects its plugin into `astro dev` |
| Vue (Vite) | ✅ | PortBay injects its plugin into `vite` |
| Svelte / SvelteKit (Vite) | ✅ | PortBay injects its plugin into `vite` |
| React / Preact / Solid (Vite) | ❌ | JSX and TSX are refused — the markup lives inside JavaScript and matching it needs a parser, not a scanner |
| Next.js (SWC — Turbopack **and** webpack) | ✅ | `@portbay/swc-plugin-loc` 0.1.1+, which you install yourself |
| Babel pipelines (CRA, custom webpack) | ❌ | No injection seam, and no plugin |
| Laravel (Blade) | ✅ | `portbay/blade-stamper`, which you install yourself |
| ERB, Django, WordPress, Hugo, Jekyll, Eleventy | ❌ | Server-rendered; instrumentation would be a template-engine concern |

Where precise editing is off, text, class and attribute edits still work by
searching your source. Structural edits are refused rather than guessed, style
edits go to PortBay's override sheet, and CMS-driven elements cannot be detected
as CMS-driven.

## Next.js (SWC)

```bash
pnpm add -D @portbay/swc-plugin-loc@^0.1.1
```

```js
// next.config.js — dev only; `next dev --turbopack` reads the same key
module.exports = {
  experimental: {
    swcPlugins: [
      ['@portbay/swc-plugin-loc', { root: process.cwd(), enabled: process.env.NODE_ENV !== 'production' }],
    ],
  },
}
```

The browser bar's precise-editing chip can write that registration for you.

> **Version floor: 0.1.1.** `0.1.0` stamps nothing under Turbopack, silently,
> whatever you put in the config — Turbopack hands the plugin a project-relative
> filename where webpack hands it an absolute one, and 0.1.0 gives up on that
> without saying so. `0.1.1` handles both, so `next dev` works as it is on Next
> 16 (Turbopack by default) and on Next 15 (webpack by default). If your
> lockfile pins `0.1.0`, upgrade it.

## Laravel (Blade)

```bash
composer require --dev portbay/blade-stamper
php artisan view:clear
```

Laravel's package auto-discovery registers the service provider, so there is no
config file to edit. Two things worth knowing:

- Stamping is decided at Blade **compile** time and frozen into
  `storage/framework/views`, which Blade only rebuilds when a template's mtime
  changes — hence the `view:clear`.
- Nothing is emitted when `APP_ENV=production`. To force it on or off, set
  `PORTBAY_LOC` in `.env`, **not** in your shell: `php artisan serve` passes only
  a fixed allow-list of environment variables to the PHP process and drops the
  rest, so `PORTBAY_LOC=0 php artisan serve` has no effect.

## What gets stamped

Only real **host** elements (the lowercase intrinsic tags that produce a DOM node — `<div>`, `<li>`, `<button>`) are stamped. Components (`<Hero>`), TypeScript generics (`useState<string>()`), and framework wrappers (`<template>`, `<slot>`, `<script>`, `<style>`) are never touched. Each element carries the location of its own opening `<`, with the line as the firm anchor (the column is a tie-breaker, since compilers disagree on column bases).

## Precise vs. fallback at a glance

| Edit | Fallback (text search) | Precise (`data-pb-loc`) |
|---|---|---|
| Static HTML text / `class=` | ✅ | ✅ |
| React `className` | ❌ | ❌ — JSX isn't stamped (see the table above) |
| `.map()` / `v-for` / `{#each}` item | ❌ refused (ambiguous) | ✅ patches the one template element |
| Repeated text across the page | ⚠️ may mis-resolve | ✅ exact element |
| Vue / Svelte scoped styles | ⚠️ drops to override CSS | ✅ resolves in the component file |
| A computed class (`cx(...)`, `:class`) | ❌ | refused with a precise `file:line` message |

Where stamping isn't available, PortBay falls back to the text-search resolver with **zero behavior change** — nothing you have working today regresses. It says so rather than degrading quietly.

## Server-rendered templates

There's no JS build step to hook for Blade, Twig, ERB, or Liquid, so the build plugins don't cover them. The resolver itself is framework-agnostic — it consumes `data-pb-loc` no matter who emitted it — so the moment any instrumentation stamps the attribute in your rendered HTML, those edits resolve precisely too. Blade has that instrumentation now (see [Laravel (Blade)](#laravel-blade) above); Twig, ERB and Liquid are on the roadmap, and until then they use the text-search fallback.

## Troubleshooting

- **Chip says "off" after installing.** Restart the dev server, then reload the page so the freshly stamped HTML is served. The chip reflects whether `data-pb-loc` is present in the current DOM.
- **No chip at all.** PortBay couldn't detect a supported bundler. Confirm `vite` (or a Babel config) is present in the project root.
- **An edit refuses with a `file:line` message.** The element's class is built from an expression (e.g. `className={cx('a', b)}` or Vue `:class`). Edit it in code — PortBay won't rewrite a dynamic expression as a string.

## Inspecting the editor over MCP

A BYO agent (Claude Code, Cursor) can read the editor's persisted state — and roll it back — through four MCP tools in the opt-in `editor` toolset:

- `portbay_editor_working_diff` — the project's uncommitted source changes vs `HEAD` (the same set the in-app **Diff** review shows): per-file project-relative path, `created` flag, kind, and exact before/after line counts. Pass `includeContent: true` to also get each file's before/after text (capped per file). Read-only.
- `portbay_editor_history` — the visual-edit apply ledger, newest first: the human-readable edit summaries, file count, and the surface-minted transaction id — the agent-attributed edit trail. Read-only.
- `portbay_editor_restore_preview` — exactly what restoring one history entry would change, without writing anything: every file the restore would touch, whether it would be deleted, and whether the entry is still restorable at all. Read-only.
- `portbay_editor_restore` — **destructive.** Rolls the tree back to the state one history entry captured; the same restore the human runs from the in-app **History** panel.

```bash
portbay-mcp --toolsets editor
# or alongside the board + verify tools
portbay-mcp --toolsets tasks,verify,editor
```

All four only exist on a Pro build compiled with both the `tasks` and `visual-editor` features, like the `verify` and `browser` toolsets. They work off disk (the git working tree and `.portbay/visual-history`); they do not touch the live page. Driving the live canvas from an agent — reading the selection, querying elements, applying guarded edits — is a separate, approval-gated surface that needs an app-process bridge and is not part of this layer.

### Restoring from an agent

`portbay_editor_restore` runs the same engine as the in-app History restore, so it inherits every guard: the reviewed-bytes pin, a re-read of each target immediately before the write, staged-then-renamed writes, a crash journal, and a roll-forward snapshot that makes the restore itself restorable (its id comes back as `rollbackEntryId`).

A restore is **entry-scoped, not file-scoped**: it writes every file the entry captured and cannot restore one file while leaving the entry's others alone. So the tool takes `paths` as an exact *consent list* rather than a filter — call `portbay_editor_restore_preview`, then pass back every project-relative path it listed. A mismatch refuses and writes nothing, returning `notConfirmed` with `unnamed` (files the restore would change that you did not name) and `notInRestore` (paths you named that it would not touch). `notConfirmed` is a decision to make, never a call to retry unchanged.

The three read-only tools survive `--read-only`; `portbay_editor_restore` does not.
