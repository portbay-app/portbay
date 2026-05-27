/**
 * Shared `cert_info(id)` loader.
 *
 * Both the project detail panel and the compact detail rail need to fetch a
 * project's certificate with identical semantics: only https projects have a
 * cert, and a `PROJECT_NOT_FOUND` error means "no cert issued yet" (the empty
 * state) rather than a hard failure. This factory owns that loading/error
 * state so the two callers can't drift apart.
 *
 * Usage (in a `.svelte` component):
 * ```ts
 * const cert = createCertInfo();
 * const certInfo = $derived(cert.info);
 * $effect(() => { void cert.load(project); });
 * ```
 */
import { invokeQuiet } from "$lib/ipc";
import type { CertInfo } from "$lib/types/certs";
import type { CommandError } from "$lib/types/error";

export interface CertInfoStore {
  readonly info: CertInfo | null;
  readonly loading: boolean;
  /** User-facing error message, or null for the empty/no-cert state. */
  readonly error: string | null;
  load(project: { id: string; https: boolean } | null): Promise<void>;
  clear(): void;
}

export function createCertInfo(): CertInfoStore {
  let info = $state<CertInfo | null>(null);
  let loading = $state<boolean>(false);
  let error = $state<string | null>(null);

  async function load(
    project: { id: string; https: boolean } | null,
  ): Promise<void> {
    // No cert to show for a missing or plain-HTTP project.
    if (!project || !project.https) {
      info = null;
      error = null;
      return;
    }
    loading = true;
    error = null;
    try {
      info = await invokeQuiet<CertInfo>("cert_info", { id: project.id });
    } catch (e) {
      info = null;
      const err = e as CommandError | undefined;
      // PROJECT_NOT_FOUND means "no cert issued yet" — the empty state.
      error =
        err && err.code !== "PROJECT_NOT_FOUND" ? err.whatHappened : null;
    } finally {
      loading = false;
    }
  }

  function clear(): void {
    info = null;
    error = null;
    loading = false;
  }

  return {
    get info() {
      return info;
    },
    get loading() {
      return loading;
    },
    get error() {
      return error;
    },
    load,
    clear,
  };
}
