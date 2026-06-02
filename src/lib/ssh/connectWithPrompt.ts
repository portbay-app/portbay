/**
 * connectWithPrompt — wrap a connect-bearing SSH invoke so that, when the host
 * needs a credential, the user is asked for it inline (VS Code-style) and the
 * action is retried — with no intermediate steps and **nothing persisted**
 * unless the user explicitly opted in via the "Remember on this device" checkbox.
 *
 * Flow: run `action()` (which must use `invokeQuiet` so it doesn't toast on its
 * own). If it fails with `SSH_NEEDS_PASSPHRASE` / `SSH_NEEDS_PASSWORD`, open the
 * credential prompt and retry `action(cred)` with the entered secret passed
 * **inline** to the command. Any other failure (or a failed retry) is toasted
 * and rethrown, exactly as a normal `safeInvoke` would.
 *
 * Authenticate-once: an entered secret is kept in an in-memory, idle-expiring
 * per-host cache (see `sessionSecrets`) and reused to seed later connects, so
 * opening a host's shell, file tree, and exec panes only prompts a single time.
 *
 * Keychain persistence: when the user checked "Remember on this device", the
 * secret is saved to the OS keychain via `ssh_set_credential` **only after** a
 * successful connect — so a wrong secret is never persisted. Persist failures
 * are swallowed (best-effort); they do not affect the connect result.
 */
import { invokeQuiet, normalise } from "$lib/ipc";
import {
  credentialPrompt,
  type SshCredentialKind,
} from "$lib/stores/credentialPrompt.svelte";
import { errorBus } from "$lib/stores/errors.svelte";
import { recallSecret, rememberSecret } from "$lib/ssh/sessionSecrets";

const NEEDS: Record<string, SshCredentialKind> = {
  SSH_NEEDS_PASSPHRASE: "passphrase",
  SSH_NEEDS_PASSWORD: "password",
};

/** A one-shot secret the user typed, handed back to the retried action. */
export interface PromptedCredential {
  kind: SshCredentialKind;
  secret: string;
}

/**
 * A single connect can surface more than one credential gap in sequence. The
 * common case: a key-auth host first needs its key passphrase, and if the key
 * still won't load (wrong passphrase, or a format we can't parse) the backend
 * falls back to asking for the host password. We loop so each distinct
 * requirement gets its own inline prompt instead of giving up after the first.
 */
const MAX_PROMPT_CYCLES = 3;

export async function connectWithPrompt<T>(
  hostId: string,
  hostLabel: string,
  action: (cred?: PromptedCredential) => Promise<T>,
): Promise<T> {
  // Seed from the in-memory secret cache so a second subsystem (the SFTP tree
  // after the shell, exec after either, …) reuses the secret entered moments
  // ago for this host instead of prompting again. Memory-only, idle-expiring.
  let cred: PromptedCredential | undefined = recallCred(hostId);
  // Track whether the user asked to remember the credential for keychain persist.
  let pendingRemember = false;

  for (let cycle = 0; ; cycle++) {
    try {
      // First pass carries any cached secret (or `undefined`); each later pass
      // carries the most recently prompted credential, inline and never
      // persisted to disk unless the user opted in.
      const result = await action(cred);

      // Connect succeeded. If the user opted in to keychain persistence for
      // the credential used on this cycle, persist it now (best-effort).
      if (pendingRemember && cred && cred.secret) {
        try {
          await invokeQuiet<void>("ssh_set_credential", {
            id: hostId,
            kind: cred.kind,
            secret: cred.secret,
          });
        } catch {
          // Persist failure is non-fatal — the connect already succeeded.
        }
      }

      return result;
    } catch (raw) {
      const err = normalise(raw);
      const kind = NEEDS[err.code];
      if (!kind || cycle >= MAX_PROMPT_CYCLES) {
        // Not a credential gap, or we've already prompted too many times —
        // surface it like any other failure.
        errorBus.push(err);
        throw err;
      }

      const answer = await credentialPrompt.request({ kind, hostLabel });
      if (!answer) {
        // Cancelled (Escape / backdrop / Cancel): abort the action.
        throw err;
      }
      if (answer.skipped) {
        // "Skip" on a passphrase prompt: retry with an explicit empty
        // passphrase. The backend reads that as "declined" and falls through
        // to the password prompt instead of asking for the passphrase again.
        cred = { kind, secret: "" };
        rememberSecret(hostId, kind, "");
        pendingRemember = false;
        continue;
      }
      if (!answer.secret) {
        // Empty, non-skip answer (shouldn't happen — the password button is
        // disabled when blank): nothing new to try, so abort.
        throw err;
      }
      cred = { kind, secret: answer.secret };
      // Cache for the session so other subsystems on this host don't re-prompt.
      rememberSecret(hostId, kind, answer.secret);
      // Record keychain opt-in so we persist after a successful connect.
      pendingRemember = answer.remember === true;
    }
  }
}

/** Seed a connect from the per-host secret cache, preferring a saved password. */
function recallCred(hostId: string): PromptedCredential | undefined {
  const password = recallSecret(hostId, "password");
  if (password !== undefined) return { kind: "password", secret: password };
  const passphrase = recallSecret(hostId, "passphrase");
  if (passphrase !== undefined) return { kind: "passphrase", secret: passphrase };
  return undefined;
}
