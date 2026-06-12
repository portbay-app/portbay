/**
 * sshHostKeyPrompt — an event-driven store for the VS Code-style SSH host-key
 * accept dialog. When a connect hits an untrusted or changed host key, the
 * Tauri backend emits `portbay://ssh-hostkey-prompt`. This store listens for
 * that event, exposes the current prompt, and forwards the user's choice back
 * to the backend via `ssh_interaction_respond` / `ssh_interaction_cancel`.
 *
 * Unlike `credentialPrompt` (which is promise-based), this store is entirely
 * event-driven: the backend owns the flow and fails closed on timeout, so the
 * store need only relay whatever action the user takes.
 *
 * It drives a single <SshHostKeyPrompt> mounted at the layout root, rendered
 * as a top-anchored dialog like VS Code's Quick Input. The user's choice is
 * sent once; the dialog then closes and the connect either continues or aborts.
 */

import { browser } from "$app/environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { safeInvoke } from "$lib/ipc";

/** Payload emitted by `portbay://ssh-hostkey-prompt`. */
export interface HostKeyPrompt {
  /** Opaque identifier that must echo back in every response. */
  flowId: string;
  /** Hostname or IP being connected to. */
  host: string;
  /** SSH port (typically 22). */
  port: number;
  /**
   * "new" — first contact with this host, no entry in known_hosts.
   * "changed" — host key differs from the previously trusted fingerprint.
   */
  state: "new" | "changed";
  /** Algorithm name, e.g. "ssh-ed25519". */
  keyType: string;
  /** The key the server presented, in "SHA256:…" form. */
  fingerprint: string;
  /**
   * The fingerprint that was previously trusted (only sometimes present on
   * "changed"; may be absent even then).
   */
  expectedFingerprint?: string;
}

/** Action values accepted by `ssh_interaction_respond`. */
type InteractionAction = "trust_once" | "trust_save" | "reject";

function createSshHostKeyPromptStore() {
  let prompt = $state<HostKeyPrompt | null>(null);
  let unlisten: UnlistenFn | null = null;

  /**
   * Attach the Tauri event listener. Called from the component's onMount so
   * the listener is only registered in a real browser/Tauri context.
   */
  async function start() {
    if (!browser) return;
    unlisten = await listen<HostKeyPrompt>(
      "portbay://ssh-hostkey-prompt",
      (event) => {
        // If a new event arrives while one is already open, replace it. The old
        // flow will time out backend-side (the backend fails closed).
        prompt = event.payload;
      },
      // The backend emits this point-to-point to the main window (the payload
      // is secrets-adjacent); a targeted emit skips untargeted listeners.
      { target: "main" },
    );
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

  /** Send a response to the backend and clear local state. */
  async function respond(action: InteractionAction) {
    const flowId = prompt?.flowId;
    if (!flowId) return;
    clear();
    try {
      await safeInvoke("ssh_interaction_respond", { flowId, action, responses: null });
    } catch {
      // Toast pushed by safeInvoke — the user learns their answer never
      // reached the backend (the connect aborts on the backend timeout).
    }
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
    get prompt(): HostKeyPrompt | null {
      return prompt;
    },
    get host() {
      return prompt?.host ?? "";
    },
    get port() {
      return prompt?.port ?? 22;
    },
    get state() {
      return prompt?.state ?? "new";
    },
    get keyType() {
      return prompt?.keyType ?? "";
    },
    get fingerprint() {
      return prompt?.fingerprint ?? "";
    },
    get expectedFingerprint() {
      return prompt?.expectedFingerprint;
    },

    // ── Actions ────────────────────────────────────────────────────────────
    /**
     * Trust the key for this session only (do not persist to known_hosts).
     * Fire-and-forget; no toast.
     */
    trustOnce() {
      void respond("trust_once");
    },
    /**
     * Trust and persist the key to known_hosts so future connects are
     * accepted automatically.
     * Fire-and-forget; no toast.
     */
    trustSave() {
      void respond("trust_save");
    },
    /**
     * Reject the key — abort the connect. Equivalent to `dismiss()` from the
     * backend's perspective, but semantically the user has actively refused.
     * Fire-and-forget; no toast.
     */
    reject() {
      void respond("reject");
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
      try {
        await safeInvoke("ssh_interaction_cancel", { flowId });
      } catch {
        // Toast pushed by safeInvoke; the backend fails closed regardless.
      }
    },
  };
}

export const sshHostKeyPrompt = createSshHostKeyPromptStore();
