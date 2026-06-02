<!--
  SqlEditor — CodeMirror 6 editor tuned for the database workbench's SQL
  scratchpad. Syntax highlighting, schema-aware autocompletion, undo history,
  and a Cmd/Ctrl+Enter "run" keybinding. Kept separate from the L3-config
  CodeEditor atom so each can evolve independently.
-->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  import { autocompletion } from "@codemirror/autocomplete";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { sql, type SQLNamespace } from "@codemirror/lang-sql";
  import { EditorState } from "@codemirror/state";
  import { oneDark } from "@codemirror/theme-one-dark";
  import {
    EditorView,
    keymap,
    lineNumbers,
    placeholder as placeholderExt,
    type ViewUpdate,
  } from "@codemirror/view";

  interface Props {
    value: string;
    oninput: (value: string) => void;
    /** Invoked on Cmd/Ctrl+Enter. */
    onRun?: () => void;
    /** Table → columns map for autocompletion. */
    schema?: SQLNamespace;
    minHeight?: number;
    placeholder?: string;
  }

  let {
    value = "",
    oninput,
    onRun,
    schema,
    minHeight = 80,
    placeholder = "SELECT * FROM …",
  }: Props = $props();

  let host: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let syncingFromEditor = false;

  const lightTheme = EditorView.theme({
    "&": {
      backgroundColor: "var(--color-surface-2)",
      color: "var(--color-fg)",
    },
    ".cm-content": {
      caretColor: "var(--color-fg)",
    },
    ".cm-gutters": {
      backgroundColor: "var(--color-surface-2)",
      color: "var(--color-fg-subtle)",
      borderRight: "1px solid var(--color-border)",
    },
    ".cm-activeLine, .cm-activeLineGutter": {
      backgroundColor: "color-mix(in oklab, var(--color-accent) 8%, transparent)",
    },
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
      backgroundColor: "color-mix(in oklab, var(--color-accent) 24%, transparent)",
    },
  });

  function frameTheme(height: number) {
    return EditorView.theme({
      "&": {
        minHeight: `${height}px`,
        maxHeight: "240px",
        border: "1px solid var(--color-border)",
        borderRadius: "6px",
        overflow: "hidden",
        fontSize: "12px",
      },
      "&.cm-focused": {
        outline: "none",
        borderColor: "color-mix(in oklab, var(--color-accent) 60%, transparent)",
      },
      ".cm-scroller": {
        overflow: "auto",
        fontFamily: 'ui-monospace, "SF Mono", Menlo, Monaco, "Cascadia Code", monospace',
        lineHeight: "1.6",
      },
      ".cm-content": {
        padding: "8px 0",
      },
    });
  }

  const updateListener = EditorView.updateListener.of((update: ViewUpdate) => {
    if (!update.docChanged) return;
    syncingFromEditor = true;
    oninput(update.state.doc.toString());
  });

  function extensions() {
    return [
      lineNumbers(),
      history(),
      sql(schema ? { schema, upperCaseKeywords: true } : { upperCaseKeywords: true }),
      autocompletion(),
      placeholderExt(placeholder),
      keymap.of([
        {
          key: "Mod-Enter",
          preventDefault: true,
          run: () => {
            onRun?.();
            return true;
          },
        },
        indentWithTab,
        ...defaultKeymap,
        ...historyKeymap,
      ]),
      oneDark,
      lightTheme,
      frameTheme(minHeight),
      updateListener,
    ];
  }

  onMount(() => {
    if (!host) return;
    view = new EditorView({
      parent: host,
      state: EditorState.create({ doc: value, extensions: extensions() }),
    });
  });

  // Keep the editor in sync when the value is changed from outside (e.g. picking
  // a query from history or the visual builder), without clobbering local typing.
  $effect(() => {
    if (!view) return;
    if (syncingFromEditor) {
      syncingFromEditor = false;
      return;
    }
    const current = view.state.doc.toString();
    if (current === value) return;
    view.dispatch({ changes: { from: 0, to: current.length, insert: value } });
  });

  onDestroy(() => {
    view?.destroy();
    view = null;
  });
</script>

<div bind:this={host} class="w-full"></div>
