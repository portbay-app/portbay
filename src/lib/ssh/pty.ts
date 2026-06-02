/**
 * Frontend bridge to the interactive PTY shell commands.
 *
 * `openPty` wires a Tauri `Channel<PtyEvent>` for the output stream and opens
 * the shell through `connectWithPrompt`, so a password/passphrase-needing host
 * is asked once (VS Code-style) with a one-shot secret — same flow as the file
 * browser and deploy panel. The returned pty id addresses input / resize /
 * close. Those three are fire-and-forget: a shell that has already exited just
 * drops them, which is not worth a toast.
 */
import { Channel } from "@tauri-apps/api/core";

import { invokeQuiet } from "$lib/ipc";
import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";

/** An event streamed from a live pty (mirrors the Rust `PtyEvent`). */
export type PtyEvent =
  | { type: "data"; bytes: number[] }
  | { type: "exit"; code: number | null };

/** Tab/split-management chord a terminal session bubbles up to its tab strip. */
export type TerminalShortcut =
  | { action: "new" }
  | { action: "close" }
  | { action: "next" }
  | { action: "prev" }
  | { action: "jump"; index: number }
  /** Split the active tab; `row` = side-by-side, `col` = stacked. */
  | { action: "split"; direction: "row" | "col" };

/**
 * Open an interactive shell on a connection. `onEvent` receives output + the
 * final exit. Resolves to the pty id, or throws (after the prompt loop) if the
 * connect ultimately fails.
 */
export async function openPty(
  connectionId: string,
  label: string,
  cols: number,
  rows: number,
  onEvent: (event: PtyEvent) => void,
  /** Optional program to run under the pty instead of a login shell (Logs). */
  command?: string,
): Promise<string> {
  const channel = new Channel<PtyEvent>();
  channel.onmessage = onEvent;
  return connectWithPrompt(connectionId, label, (cred) =>
    invokeQuiet<string>("ssh_pty_open", {
      input: {
        connectionId,
        cols,
        rows,
        command: command || undefined,
        password: cred?.kind === "password" ? cred.secret : undefined,
        passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
      },
      onEvent: channel,
    }),
  );
}

/** Send typed input (xterm-encoded) to a pty. Best-effort. */
export function ptyInput(id: string, data: string): void {
  void invokeQuiet("ssh_pty_input", { id, data }).catch(() => {});
}

/** Tell a pty its terminal was resized. Best-effort. */
export function ptyResize(id: string, cols: number, rows: number): void {
  void invokeQuiet("ssh_pty_resize", { id, cols, rows }).catch(() => {});
}

/** Close a pty and free its backend session. Best-effort. */
export function ptyClose(id: string): void {
  void invokeQuiet("ssh_pty_close", { id }).catch(() => {});
}
