<!-- CodeMirror-backed editor for L3 raw config surfaces. -->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  import { json } from "@codemirror/lang-json";
  import { EditorState } from "@codemirror/state";
  import { oneDark } from "@codemirror/theme-one-dark";
  import { EditorView, lineNumbers, type ViewUpdate } from "@codemirror/view";

  interface Props {
    value: string;
    language?: "json";
    oninput: (value: string) => void;
    onblur?: () => void;
    minHeight?: number;
  }

  let {
    value = "",
    language = "json",
    oninput,
    onblur,
    minHeight = 240,
  }: Props = $props();

  let host: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let syncingFromEditor = false;

  const lightTheme = EditorView.theme({
    "&": {
      backgroundColor: "var(--color-bg)",
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
        minHeight: `${height}px`,
        fontFamily:
          'ui-monospace, "SF Mono", Menlo, Monaco, "Cascadia Code", monospace',
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

  const blurHandler = EditorView.domEventHandlers({
    blur: () => {
      onblur?.();
    },
  });

  function extensions() {
    return [
      lineNumbers(),
      language === "json" ? json() : [],
      oneDark,
      lightTheme,
      frameTheme(minHeight),
      updateListener,
      blurHandler,
    ];
  }

  onMount(() => {
    if (!host) return;
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: value,
        extensions: extensions(),
      }),
    });
  });

  $effect(() => {
    if (!view) return;
    if (syncingFromEditor) {
      syncingFromEditor = false;
      return;
    }
    const current = view.state.doc.toString();
    if (current === value) return;
    view.dispatch({
      changes: { from: 0, to: current.length, insert: value },
    });
  });

  onDestroy(() => {
    view?.destroy();
    view = null;
  });
</script>

<div bind:this={host} class="w-full"></div>
