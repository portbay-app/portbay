<!--
  IdeStatusBar — the slim VS Code-style status strip across the bottom of the
  host workspace: connection state on the left, port-forward count + a panel
  toggle on the right.

  Connection state is a single lock glyph (the browser-padlock convention):
  closed green lock = live encrypted session; yellow = session up but the host
  probe reports degraded; open lock = no session (gray when the host is fine,
  red when the probe says it's down). The label lives in a hover tooltip so
  the strip stays quiet.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { ProbeHealth } from "$lib/types/sshConnections";

  interface Props {
    hostName: string;
    dest: string;
    port: number;
    connected: boolean;
    health: ProbeHealth;
    tunnelCount: number;
    panelVisible: boolean;
    onTogglePanel: () => void;
  }
  let {
    hostName,
    dest,
    port,
    connected,
    health,
    tunnelCount,
    panelVisible,
    onTogglePanel,
  }: Props = $props();

  // connected × probe-health → one lock state. Connected always shows the
  // closed lock (the session is live and encrypted); the probe only shades it
  // when the host itself is struggling. Disconnected shows the open lock,
  // tinted by how the host last looked.
  const lock = $derived.by((): { icon: "lock" | "lock-open"; tone: string; label: string; detail: string } => {
    if (connected) {
      if (health === "degraded" || health === "down") {
        return {
          icon: "lock",
          tone: "text-status-unhealthy",
          label: "Connected — host degraded",
          detail: "The SSH session is live and encrypted, but the host's last health probe reported problems.",
        };
      }
      return {
        icon: "lock",
        tone: "text-status-running",
        label: "Connected",
        detail: `Encrypted SSH session to ${dest}:${port}.`,
      };
    }
    switch (health) {
      case "down":
        return {
          icon: "lock-open",
          tone: "text-status-crashed",
          label: "Unreachable",
          detail: "No session, and the last probe couldn't reach the host.",
        };
      case "degraded":
        return {
          icon: "lock-open",
          tone: "text-status-unhealthy",
          label: "Not connected — host degraded",
          detail: "No session; the host's last health probe reported problems.",
        };
      default:
        return {
          icon: "lock-open",
          tone: "text-fg-subtle",
          label: "Not connected",
          detail: "No open session. Your next action connects and authenticates.",
        };
    }
  });
</script>

<footer
  class="flex h-6 shrink-0 items-center gap-3 border-t border-border/60 bg-surface/40 px-3
         text-[11px] text-fg-subtle"
>
  <span class="group relative inline-flex items-center">
    <Icon name={lock.icon} size={12} class={lock.tone} />
    <!-- Styled tooltip — the strip sits on the window edge, so it opens upward. -->
    <span
      role="tooltip"
      class="pointer-events-none absolute bottom-full left-0 z-30 mb-2 w-56 rounded-lg border border-border
             bg-surface p-2.5 opacity-0 shadow-xl transition-opacity duration-100 group-hover:opacity-100"
    >
      <span class="flex items-center gap-1.5 text-[11.5px] font-medium {lock.tone}">
        <Icon name={lock.icon} size={11} /> {lock.label}
      </span>
      <span class="mt-1 block text-[11px] leading-snug text-fg-muted">{lock.detail}</span>
    </span>
  </span>

  <span class="truncate font-mono">{hostName}</span>
  <span class="truncate font-mono text-fg-subtle">{dest}:{port}</span>

  <div class="ml-auto flex items-center gap-3">
    {#if tunnelCount > 0}
      <span class="inline-flex items-center gap-1">
        <Icon name="link" size={12} />
        {tunnelCount}
      </span>
    {/if}
    <button
      type="button"
      onclick={onTogglePanel}
      title="Toggle panel (Ctrl+`)"
      aria-label="Toggle panel"
      class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-surface-2 hover:text-fg
        {panelVisible ? 'text-fg' : ''}"
    >
      <Icon name="terminal" size={12} />
    </button>
  </div>
</footer>
