/**
 * sshKbiPrompt — an event-driven store for the SSH keyboard-interactive (KBI)
 * / 2FA challenge dialog. When a connect triggers a server-side KBI exchange,
 * the Tauri backend emits `portbay://ssh-kbi-prompt`. This store listens for
 * that event, exposes the current prompt and its fields, and forwards the
 * user's answers (or a cancellation) back to the backend via
 * `ssh_interaction_respond` / `ssh_interaction_cancel`.
 *
 * Unlike `credentialPrompt` (which is promise-based), this store is entirely
 * event-driven: the backend owns the flow and fails closed on timeout (120s),
 * so the store need only relay whatever action the user takes.
 *
 * It drives a single <SshKbiPrompt> mounted at the layout root, rendered as a
 * top-anchored dialog like VS Code's Quick Input.
 */

import { browser } from "$app/environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

/** A single field in a keyboard-interactive challenge. */
export interface KbiPromptField {
  /** The label/question the server presents. */
  prompt: string;
  /** When true the input may be shown in plain text; when false, mask it. */
  echo: boolean;
}

/** Payload emitted by `portbay://ssh-kbi-prompt`. */
export interface KbiPrompt {
  /** Opaque identifier that must echo back in every response. */
  flowId: string;
  /** Host label, e.g. "deploy@bastion.example.com". */
  host: string;
  /** Server-provided title for the challenge (may be empty). */
  name: string;
  /** Server-provided instructions (may be empty; may contain newlines). */
  instructions: string;
  /** One or more fields the user must fill in, in order. */
  prompts: KbiPromptField[];
}

function createSshKbiPromptStore() {
  let prompt = $state<KbiPrompt | null>(null);
  let unlisten: UnlistenFn | null = null;

  /**
   * Attach the Tauri event listener. Called from the component's onMount so
   * the listener is only registered in a real browser/Tauri context.
   */
  async function start() {
    if (!browser) return;
    unlisten = await listen<KbiPrompt>("portbay://ssh-kbi-prompt", (event) => {
      // If a new challenge arrives while one is already open, replace it. The
      // old flow will time out backend-side (the backend fails closed).
      prompt = event.payload;
    });
  }

  /** Remove the Tauri event listener. Call from the component's onDestroy. */
  function stop() {
    unlisten?.();
    unlisten = null;
  }

  /** Clear local state after responding. */
  function clear() {
    prompt = null;
  }

  return {
    // ── Lifecycle ──────────────────────────────────────────────────────────
    start,
    stop,

    // ── Getters ────────────────────────────────────────────────────────────
    get isOpen() {
      return prompt !== null;
    },
    /** The full prompt payload, or null when no dialog is open. */
    get prompt(): KbiPrompt | null {
      return prompt;
    },
    get host() {
      return prompt?.host ?? "";
    },
    get name() {
      return prompt?.name ?? "";
    },
    get instructions() {
      return prompt?.instructions ?? "";
    },
    get prompts(): KbiPromptField[] {
      return prompt?.prompts ?? [];
    },

    // ── Actions ────────────────────────────────────────────────────────────
    /**
     * Submit answers for all fields. Captures `flowId` before clearing state
     * so the invoke reaches the backend even after the dialog is gone.
     * Fire-and-forget; no toast.
     */
    async submit(responses: string[]) {
      const flowId = prompt?.flowId;
      if (!flowId) return;
      clear();
      await invoke("ssh_interaction_respond", { flowId, action: "submit", responses });
    },
    /**
     * Dismiss the dialog (backdrop click / Esc / Cancel button). Sends
     * `ssh_interaction_cancel` to signal that no answer was given; the
     * backend aborts the connect.
     */
    async dismiss() {
      const flowId = prompt?.flowId;
      if (!flowId) return;
      clear();
      await invoke("ssh_interaction_cancel", { flowId });
    },
  };
}

export const sshKbiPrompt = createSshKbiPromptStore();
