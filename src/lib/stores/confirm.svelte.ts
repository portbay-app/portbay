/**
 * confirm — a promise-based replacement for the browser's native
 * `confirm()`. Native confirm only offers OK/Cancel, which forces
 * destructive flows to overload those two words and (as in the database
 * delete dialog) invites data-loss when OK and "delete" don't line up.
 *
 * This store drives a single <ConfirmDialog> mounted at the layout root.
 * Call `confirmDialog.open({...})` and await the chosen action's value
 * (or `null` when the user cancels / dismisses), e.g.
 *
 *   const choice = await confirmDialog.open({
 *     title: "Remove database?",
 *     message: "…",
 *     actions: [
 *       { label: "Delete data + deregister", value: "delete", tone: "destructive" },
 *       { label: "Deregister only", value: "deregister" },
 *     ],
 *   });
 *   if (choice === null) return; // cancelled
 */
import type { IconName } from "$lib/components/atoms/Icon.svelte";

export type ConfirmTone = "default" | "primary" | "destructive";

export interface ConfirmAction {
  /** Button text. Make it describe the outcome, never just "OK". */
  label: string;
  /** Returned from `open()` when this action is chosen. */
  value: string;
  /** Visual weight. `destructive` is red; `primary` is the accent. */
  tone?: ConfirmTone;
  icon?: IconName;
}

export interface ConfirmOptions {
  title: string;
  /** Body text. Newlines render as paragraph breaks. */
  message: string;
  /** Explicit, labeled choices (Cancel is added automatically). */
  actions: ConfirmAction[];
  /** Cancel button text. Defaults to "Cancel". */
  cancelLabel?: string;
  /** Header icon; defaults to a warning glyph for destructive dialogs. */
  icon?: IconName;
  /** Whether the dialog is framed as destructive (header tint). */
  destructive?: boolean;
}

interface ConfirmState extends ConfirmOptions {
  open: boolean;
}

function createConfirmStore() {
  let state = $state<ConfirmState>({
    open: false,
    title: "",
    message: "",
    actions: [],
  });

  let resolver: ((value: string | null) => void) | null = null;

  function open(opts: ConfirmOptions): Promise<string | null> {
    // If a dialog is somehow already open, resolve it as cancelled so we
    // never leave a dangling promise.
    resolver?.(null);
    state = { ...opts, open: true };
    return new Promise<string | null>((resolve) => {
      resolver = resolve;
    });
  }

  function settle(value: string | null) {
    if (!state.open) return;
    state = { ...state, open: false };
    const r = resolver;
    resolver = null;
    r?.(value);
  }

  return {
    get isOpen() {
      return state.open;
    },
    get title() {
      return state.title;
    },
    get message() {
      return state.message;
    },
    get actions() {
      return state.actions;
    },
    get cancelLabel() {
      return state.cancelLabel ?? "Cancel";
    },
    get icon() {
      return state.icon ?? "circle-alert";
    },
    get destructive() {
      return state.destructive ?? false;
    },
    open,
    /** Resolve with an action's value. */
    choose(value: string) {
      settle(value);
    },
    /** Resolve as cancelled (`null`). */
    cancel() {
      settle(null);
    },
  };
}

export const confirmDialog = createConfirmStore();
