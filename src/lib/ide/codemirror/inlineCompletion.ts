/**
 * CodeMirror 6 inline (ghost-text) completion — the editor surface of PortBay's
 * shared completion engine. Renders a faint suggestion after the cursor and
 * wires the accept/dismiss keys; the actual debounce/cache/cancel/postprocess
 * lives in `$lib/autocomplete/engine` (the caller passes a `request`/`cancel`).
 *
 * Keys (matching the product spec, shared with the terminal hint):
 *   • Tab            — accept the whole suggestion
 *   • Esc            — dismiss it
 *   • Cmd/Ctrl + →   — accept the next word only
 *
 * Behaviour borrowed from Tabby/Continue: one suggestion at a time, invalidated
 * on every doc/selection change, re-requested after a pause, and never shown
 * when there's a selection (not a bare caret).
 */
import { Annotation, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
  keymap,
} from "@codemirror/view";

export interface InlineCompletionSource {
  /** Returns the ghost text for the cursor position, or null. Already
      debounced/cached/cancelled/post-processed by the engine. */
  request: (prefix: string, suffix: string) => Promise<string | null>;
  /** Abort any pending/in-flight request. */
  cancel: () => void;
}

interface Suggestion {
  text: string;
  /** Document offset the ghost is anchored at (the caret when requested). */
  pos: number;
}

// Marks transactions we create when accepting, so the requester plugin doesn't
// immediately fire a fresh request and clobber a partial-accept remainder.
const fromAccept = Annotation.define<boolean>();
const setSuggestion = StateEffect.define<Suggestion | null>();

const suggestionField = StateField.define<Suggestion | null>({
  create() {
    return null;
  },
  update(value, tr) {
    // An explicit set/clear effect always wins (incl. the partial-accept case,
    // where the same transaction also edits the doc).
    for (const e of tr.effects) if (e.is(setSuggestion)) return e.value;
    // Any other doc/selection change invalidates the stale ghost; the plugin
    // re-requests a fresh one.
    if (tr.docChanged || tr.selection) return null;
    return value;
  },
});

class GhostWidget extends WidgetType {
  constructor(readonly text: string) {
    super();
  }
  eq(other: GhostWidget) {
    return other.text === this.text;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-ghost-text";
    span.textContent = this.text;
    return span;
  }
  get estimatedHeight() {
    return -1;
  }
  ignoreEvent() {
    return true;
  }
}

const ghostDecorations = EditorView.decorations.compute([suggestionField], (state): DecorationSet => {
  const s = state.field(suggestionField);
  if (!s || !s.text) return Decoration.none;
  // `side: 1` keeps the widget after the caret; `block: false` inline.
  const deco = Decoration.widget({ widget: new GhostWidget(s.text), side: 1 });
  return Decoration.set([deco.range(s.pos)]);
});

const ghostTheme = EditorView.baseTheme({
  ".cm-ghost-text": {
    opacity: "0.4",
    color: "var(--color-fg-muted, inherit)",
    whiteSpace: "pre-wrap",
  },
});

/** The leading word / whitespace / punctuation run of `text` (for Cmd/Ctrl+→). */
function nextWord(text: string): string {
  const m = text.match(/^(\s*[A-Za-z0-9_$]+|\s*[^A-Za-z0-9_$\s]+|\s+)/);
  return m ? m[0] : text;
}

function currentSuggestion(view: EditorView): Suggestion | null {
  return view.state.field(suggestionField, false) ?? null;
}

function acceptFull(view: EditorView): boolean {
  const s = currentSuggestion(view);
  if (!s) return false;
  view.dispatch({
    changes: { from: s.pos, insert: s.text },
    selection: { anchor: s.pos + s.text.length },
    effects: setSuggestion.of(null),
    annotations: fromAccept.of(true),
    userEvent: "input.complete",
  });
  return true;
}

function acceptWord(view: EditorView): boolean {
  const s = currentSuggestion(view);
  if (!s) return false;
  const word = nextWord(s.text);
  const rest = s.text.slice(word.length);
  const nextPos = s.pos + word.length;
  view.dispatch({
    changes: { from: s.pos, insert: word },
    selection: { anchor: nextPos },
    effects: setSuggestion.of(rest ? { text: rest, pos: nextPos } : null),
    annotations: fromAccept.of(true),
    userEvent: "input.complete",
  });
  return true;
}

function dismiss(view: EditorView): boolean {
  if (!currentSuggestion(view)) return false;
  view.dispatch({ effects: setSuggestion.of(null) });
  return true;
}

/** Build the inline-completion extension bound to a completion source. */
export function inlineCompletion(source: InlineCompletionSource) {
  const requester = ViewPlugin.fromClass(
    class {
      constructor(readonly view: EditorView) {}

      update(update: ViewUpdate) {
        if (!update.docChanged && !update.selectionSet) return;
        // Don't re-request off our own accept edits — keep the remainder shown.
        if (update.transactions.some((t) => t.annotation(fromAccept))) return;

        const state = update.state;
        const sel = state.selection.main;
        if (!sel.empty) {
          source.cancel();
          return;
        }
        const pos = sel.head;
        const prefix = state.doc.sliceString(0, pos);
        const suffix = state.doc.sliceString(pos);
        void source.request(prefix, suffix).then((text) => {
          if (!text) return;
          // Only show it if the caret is still exactly where we asked.
          const cur = this.view.state.selection.main;
          if (cur.empty && cur.head === pos && this.view.state.doc.sliceString(0, pos) === prefix) {
            this.view.dispatch({ effects: setSuggestion.of({ text, pos }) });
          }
        });
      }

      destroy() {
        source.cancel();
      }
    },
  );

  const keys = keymap.of([
    { key: "Tab", run: acceptFull },
    { key: "Mod-ArrowRight", run: acceptWord },
    { key: "Escape", run: dismiss },
  ]);

  // The keymap goes first so Tab accepts a ghost before `indentWithTab` indents.
  return [suggestionField, ghostDecorations, ghostTheme, keys, requester];
}
