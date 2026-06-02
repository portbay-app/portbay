/**
 * credentialPrompt — a promise-based store for the VS Code-style SSH credential
 * prompt. When a connect needs a key passphrase or a host password, the calling
 * store opens this, the user types the secret, and the promise resolves with
 * `{ secret, remember }` — or `null` if they cancel.
 *
 * It drives a single <SshCredentialPrompt> mounted at the layout root, rendered
 * as a top-anchored input like VS Code's Quick Input. The secret is used **only
 * for that one connect** — it is passed inline to the retried command. When the
 * user opts in via the "Remember on this device" checkbox, the caller persists
 * it to the OS keychain after a successful connect (see `connectWithPrompt`).
 */
export type SshCredentialKind = "passphrase" | "password";

export interface CredentialRequest {
  /** Which secret to ask for (drives the label). */
  kind: SshCredentialKind;
  /** Host label shown in the prompt (e.g. "deploy@bastion.example.com"). */
  hostLabel: string;
}

export interface CredentialAnswer {
  secret: string;
  /**
   * The user pressed "Skip" on a passphrase prompt with an empty field — i.e.
   * this key has no passphrase. Distinct from cancelling (which resolves
   * `null`): a skip means "continue the connect with no passphrase" so the
   * caller falls through to the next auth method / the password prompt.
   */
  skipped?: boolean;
  /**
   * The user opted in to keychain persistence via the "Remember on this device"
   * checkbox. The caller (`connectWithPrompt`) saves the secret to the OS
   * keychain after a successful connect — never on a failed attempt.
   * Defaults to `false`; never set by `skip()` or `cancel()`.
   */
  remember?: boolean;
}

interface PromptState extends CredentialRequest {
  open: boolean;
}

function createCredentialPromptStore() {
  let state = $state<PromptState>({ open: false, kind: "password", hostLabel: "" });
  let resolver: ((value: CredentialAnswer | null) => void) | null = null;
  let inflight: Promise<CredentialAnswer | null> | null = null;

  function request(req: CredentialRequest): Promise<CredentialAnswer | null> {
    // Coalesce concurrent requests: when a prompt is already open, a second
    // caller waits on the *same* answer rather than stealing focus or spawning
    // a duplicate. On entry the terminal and the file tree both connect to the
    // same host at once — they should share one prompt, not fight over two.
    if (state.open && inflight) return inflight;
    state = { ...req, open: true };
    inflight = new Promise<CredentialAnswer | null>((resolve) => {
      resolver = resolve;
    });
    return inflight;
  }

  function settle(value: CredentialAnswer | null) {
    if (!state.open) return;
    state = { ...state, open: false };
    const r = resolver;
    resolver = null;
    inflight = null;
    r?.(value);
  }

  return {
    get isOpen() {
      return state.open;
    },
    get kind() {
      return state.kind;
    },
    get hostLabel() {
      return state.hostLabel;
    },
    request,
    /**
     * Resolve with the entered secret. `remember` reflects the user's keychain
     * opt-in checkbox (default false). The caller persists to the OS keychain
     * only after a successful connect.
     */
    submit(secret: string, remember: boolean) {
      settle({ secret, remember });
    },
    /**
     * Resolve a passphrase prompt as "skipped" — no passphrase on this key.
     * The caller continues the connect with an empty passphrase (which the
     * backend treats as declined), rather than aborting like `cancel()`.
     */
    skip() {
      settle({ secret: "", skipped: true });
    },
    /** Resolve as cancelled (`null`). */
    cancel() {
      settle(null);
    },
  };
}

export const credentialPrompt = createCredentialPromptStore();
