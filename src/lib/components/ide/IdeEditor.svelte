<!--
  IdeEditor — a CodeMirror 6 editor for one remote file. Loads via
  `sftp_read_text`, saves via `sftp_write_text`, tracks dirty against the
  last-saved baseline, and saves on Cmd/Ctrl+S. Language is picked from the file
  extension (see `languageFor`); the editor surface is painted from the app's CSS
  theme tokens and the syntax-highlight palette swaps live with the light/dark
  theme via a compartment.

  The component stays mounted while its tab is open (the editor area only hides
  inactive editors with CSS), so its undo history and scroll survive tab
  switches and the cached SFTP session is reused without re-authenticating.
-->
<script lang="ts">
  import { EditorState, Compartment } from "@codemirror/state";
  import {
    EditorView,
    keymap,
    lineNumbers,
    highlightActiveLine,
    highlightActiveLineGutter,
    drawSelection,
    dropCursor,
    highlightSpecialChars,
  } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import {
    autocompletion,
    completeAnyWord,
    completionKeymap,
    closeBrackets,
    closeBracketsKeymap,
    type CompletionSource,
  } from "@codemirror/autocomplete";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { CompletionEngine } from "$lib/autocomplete/engine";
  import {
    languageFor,
    languageLabel,
    editorChromeTheme,
    highlightFor,
  } from "$lib/ide/codemirror";
  import {
    smartCompletionSourceFor,
    smartLanguageExtensions,
    smartLanguageSummary,
  } from "$lib/ide/languageIntelligence";
  import { inlineCompletion } from "$lib/ide/codemirror/inlineCompletion";
  import { sftpReadText, sftpWriteText } from "$lib/sftp";
  import { detectCompletionModel, fetchCompletion, type CompletionModel } from "$lib/ssh/complete";
  import { ideEditor } from "$lib/stores/ideEditor.svelte";
  import { theme } from "$lib/stores/theme.svelte";

  interface Props {
    connectionId: string;
    path: string;
    name: string;
    /** Whether this editor's tab is the active (visible) one. */
    active: boolean;
    /** Host label, used only as the credential-prompt title if model detection
        has to open the agent session. */
    label?: string;
  }
  let { connectionId, path, name, active, label = "" }: Props = $props();

  // --- Inline (ghost-text) completion ---
  // Model detection is lazy (on first typing pause), so opening a file never
  // forces an agent connect; until a host code model is found there's no ghost.
  // The session is usually already warm from the workspace, so it rarely prompts.
  let completionModel: CompletionModel | null = null;
  let modelDetected = false;
  let detectPromise: Promise<void> | null = null;

  const engine = new CompletionEngine({
    fetcher: (ctx, signal) =>
      completionModel
        ? fetchCompletion(connectionId, completionModel, ctx.prefix, ctx.suffix, signal)
        : Promise.resolve(null),
    debounceMs: 220,
    minPrefix: 2,
  });

  async function ensureModel(): Promise<void> {
    if (modelDetected) return;
    detectPromise ??= (async () => {
      completionModel = await detectCompletionModel(connectionId, label || connectionId);
      modelDetected = true;
    })();
    await detectPromise;
  }

  const completionSource = {
    request: async (prefix: string, suffix: string) => {
      await ensureModel();
      if (!completionModel) return null;
      return engine.request({ scope: path, prefix, suffix, multiline: true });
    },
    cancel: () => engine.cancel(),
  };

  let host = $state<HTMLDivElement | null>(null);
  let view: EditorView | null = null;
  // Swaps the syntax-highlight palette when the app theme flips light/dark.
  const highlightComp = new Compartment();
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let saving = $state(false);
  let saved = $state<string>(""); // last-saved baseline, for dirty tracking

  const langLabel = $derived(languageLabel(name));
  // Smart-language detection gets the full remote path: profiles like
  // `.ssh/config` or `/etc/nginx/sites-available/*` are path-, not name-based.
  const smartSummary = $derived(smartLanguageSummary(path));

  function markDirty() {
    if (!view) return;
    const dirty = view.state.doc.toString() !== saved;
    ideEditor.setDirty(path, dirty);
  }

  async function save() {
    if (!view || saving) return;
    const contents = view.state.doc.toString();
    saving = true;
    try {
      await sftpWriteText(connectionId, path, contents);
      saved = contents;
      ideEditor.setDirty(path, false);
    } catch {
      /* sftp wrapper already toasted (e.g. permission denied) */
    } finally {
      saving = false;
    }
  }

  function buildEditor(initial: string) {
    if (!host) return;
    saved = initial;
    const saveKeymap = keymap.of([
      {
        key: "Mod-s",
        preventDefault: true,
        run: () => {
          void save();
          return true;
        },
      },
    ]);
    // Completion sources are registered as language data (not `override`) so
    // they merge with the language pack's own completions (HTML tags, SQL
    // keywords, …) instead of replacing them:
    //  - the curated smart source (boosted, so it ranks above raw words)
    //  - buffer words: every word already in the document, VS Code-style —
    //    skipped for prose (Markdown/plain text), where a popup mid-sentence
    //    is noise rather than help.
    const smartCompletion = smartCompletionSourceFor(path);
    const completionSources: CompletionSource[] = [];
    if (smartCompletion) completionSources.push(smartCompletion);
    if (langLabel !== "Markdown" && langLabel !== "Plain Text") {
      completionSources.push(completeAnyWord);
    }
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: initial,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          highlightActiveLine(),
          highlightSpecialChars(),
          history(),
          drawSelection(),
          dropCursor(),
          EditorState.allowMultipleSelections.of(true),
          closeBrackets(),
          autocompletion(),
          ...(completionSources.length > 0
            ? [
                EditorState.languageData.of(() =>
                  completionSources.map((source) => ({ autocomplete: source })),
                ),
              ]
            : []),
          // Inline ghost-text completion. Placed before the main keymap so its
          // Tab handler accepts a ghost before `indentWithTab` indents.
          ...inlineCompletion(completionSource),
          saveKeymap,
          keymap.of([
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...historyKeymap,
            ...completionKeymap,
            indentWithTab,
          ]),
          ...languageFor(name),
          ...smartLanguageExtensions(path),
          editorChromeTheme,
          highlightComp.of(highlightFor(theme.resolved)),
          EditorView.lineWrapping,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) markDirty();
          }),
        ],
      }),
    });
  }

  // Load the file once, then build the editor. Runs on mount; `path` is stable
  // for the life of a tab (a rename closes + reopens), so this fires once.
  $effect(() => {
    let cancelled = false;
    void (async () => {
      loading = true;
      loadError = null;
      try {
        const text = await sftpReadText(connectionId, path);
        if (cancelled) return;
        buildEditor(text);
      } catch (e) {
        if (cancelled) return;
        loadError =
          e instanceof Error ? e.message : "Couldn't open this file (it may not be UTF-8 text).";
      } finally {
        if (!cancelled) loading = false;
      }
    })();
    return () => {
      cancelled = true;
      engine.dispose();
      view?.destroy();
      view = null;
    };
  });

  // Swap the highlight palette when the app theme flips, so an open editor
  // re-colours in place instead of staying dark on a light surface.
  $effect(() => {
    const resolved = theme.resolved;
    view?.dispatch({ effects: highlightComp.reconfigure(highlightFor(resolved)) });
  });

  // When this tab becomes visible, CodeMirror needs to re-measure (it can't size
  // itself while display:none) and take focus.
  $effect(() => {
    if (active && view) {
      requestAnimationFrame(() => {
        view?.requestMeasure();
        view?.focus();
      });
    }
  });
</script>

<div class="flex h-full min-h-0 flex-col">
  {#if loadError}
    <div class="m-4 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-3 text-[12px] text-status-crashed">
      {loadError}
    </div>
  {:else}
    {#if loading}
      <p class="p-6 text-center text-[12px] text-fg-subtle">Loading {name}…</p>
    {/if}
    <div bind:this={host} class="min-h-0 flex-1 overflow-hidden text-[12.5px]" class:hidden={loading}></div>
    <footer class="flex shrink-0 items-center gap-2 border-t border-border/50 px-3 py-1 text-[11px] text-fg-subtle">
      <span class="truncate font-mono">{path}</span>
      <span class="ml-auto">{langLabel}</span>
      {#if smartSummary}
        <span class="rounded border border-border/70 px-1.5 py-0.5 text-[10px] text-fg-subtle" title={smartSummary}>
          Smart
        </span>
      {/if}
      <button
        type="button"
        onclick={save}
        disabled={saving}
        class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
        title="Save (Cmd/Ctrl+S)"
      >
        <Icon name={saving ? "refresh-cw" : "save"} size={12} class={saving ? "animate-spin" : ""} />
        {saving ? "Saving…" : "Save"}
      </button>
    </footer>
  {/if}
</div>
