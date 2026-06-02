<!--
  IdeEditor — a CodeMirror 6 editor for one remote file. Loads via
  `sftp_read_text`, saves via `sftp_write_text`, tracks dirty against the
  last-saved baseline, and saves on Cmd/Ctrl+S. Language is picked from the file
  extension (JSON / SQL packs, else plaintext); the one-dark theme bundles its
  own syntax highlighting.

  The component stays mounted while its tab is open (the editor area only hides
  inactive editors with CSS), so its undo history and scroll survive tab
  switches and the cached SFTP session is reused without re-authenticating.
-->
<script lang="ts">
  import { EditorState } from "@codemirror/state";
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
    completionKeymap,
    closeBrackets,
    closeBracketsKeymap,
  } from "@codemirror/autocomplete";
  import { oneDark } from "@codemirror/theme-one-dark";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { languageFor, languageLabel } from "$lib/ide/codemirror";
  import { sftpReadText, sftpWriteText } from "$lib/sftp";
  import { ideEditor } from "$lib/stores/ideEditor.svelte";

  interface Props {
    connectionId: string;
    path: string;
    name: string;
    /** Whether this editor's tab is the active (visible) one. */
    active: boolean;
  }
  let { connectionId, path, name, active }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);
  let view: EditorView | null = null;
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let saving = $state(false);
  let saved = $state<string>(""); // last-saved baseline, for dirty tracking

  const langLabel = $derived(languageLabel(name));

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
          saveKeymap,
          keymap.of([
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...historyKeymap,
            ...completionKeymap,
            indentWithTab,
          ]),
          ...languageFor(name),
          oneDark,
          EditorView.lineWrapping,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) markDirty();
          }),
          EditorView.theme({
            "&": { height: "100%" },
            ".cm-scroller": { fontFamily: "var(--font-mono, ui-monospace, monospace)" },
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
      view?.destroy();
      view = null;
    };
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
