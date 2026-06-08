/**
 * terminalLaunch — a one-shot channel for "open this command in a new terminal
 * tab." Some interactions (attaching a tmux/screen session, wrapping a shell in
 * a fresh tmux) can't run over the snapshot exec layer because they're
 * *interactive* — they need a real PTY. They also originate from a different
 * bottom-panel tab (Jobs) than the one that owns the shells (Terminal).
 *
 * So the Jobs panel calls `launch()`, which (a) flips the bottom panel to the
 * Terminal tab and (b) parks a request the live `SshTerminalTabs` consumes via
 * an `$effect`, opening a new tab whose pane runs `command` under its pty.
 * The monotonic `seq` lets the same command be launched twice in a row and
 * still retrigger (a plain value wouldn't change). The request is addressed by
 * `connectionId` so a stale request can't land in the wrong host's workspace.
 */
import { ideLayout } from "$lib/stores/ideLayout.svelte";

export interface TerminalLaunchRequest {
  connectionId: string;
  /** Program to run under the pty instead of a login shell (e.g. tmux attach). */
  command: string;
  /** Seed title for the new tab until the pty reports its own. */
  title: string;
  /** Monotonic id so identical back-to-back launches still fire. */
  seq: number;
}

function createTerminalLaunch() {
  let request = $state<TerminalLaunchRequest | null>(null);
  let seq = 0;

  return {
    get request() {
      return request;
    },
    /**
     * Ask the host's Terminal panel to open a new tab running `command`. Brings
     * the Terminal tab forward so the launched session is visible immediately.
     */
    launch(connectionId: string, command: string, title: string) {
      request = { connectionId, command, title, seq: ++seq };
      ideLayout.showPanelTab("terminal");
    },
    /** Consumed by `SshTerminalTabs` once it has opened the tab. */
    clear() {
      request = null;
    },
  };
}

export const terminalLaunch = createTerminalLaunch();
