<!--
  SidecarCard — one large status card in the dashboard row.

  Visual parallel to the screenshot's NGINX / PHP / MySQL / NoSQL cards.
  Big title, status pill, an action button keyed off the current state,
  and a small detail line in the footer.
-->
<script lang="ts">
  import { DashboardCard, Icon, StatusPill } from "$lib/components/atoms";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { sidecars } from "$lib/stores/sidecars";
  import type {
    PortbayStatus,
  } from "$lib/types/status";
  import type { SidecarKey, SidecarStatus } from "$lib/types/sidecars";
  import { sidecarTitle } from "$lib/types/sidecars";
  import { openUrl } from "@tauri-apps/plugin-opener";

  interface Props {
    sidecarKey: SidecarKey;
    info: SidecarStatus;
  }
  let { sidecarKey, info }: Props = $props();

  // Map sidecar state → PortbayStatus (the taxonomy our pills are typed for).
  const pillStatus = $derived.by<PortbayStatus>(() => {
    switch (info.status) {
      case "running":
        return "running";
      case "stopped":
        return "stopped";
      case "not_installed":
        return "port_conflict";
      case "unreachable":
        return "crashed";
    }
  });

  const cardTone = $derived.by<"default" | "critical" | "warn">(() => {
    if (info.status === "unreachable") return "critical";
    if (info.status === "not_installed") return "warn";
    return "default";
  });

  // Action button shape per (sidecar × state). For Phase 2 only
  // process-compose's restart and the hosts reconcile are real backend
  // calls. The rest are stubs that surface a hint via the error envelope.
  interface CardAction {
    label: string;
    icon: IconName;
    /** Background tone: maps to a status color via Tailwind utilities. */
    tone: "neutral" | "danger" | "warn" | "accent";
    run: () => Promise<void> | void;
  }

  const action = $derived.by<CardAction | null>(() => {
    switch (sidecarKey) {
      case "processCompose":
        if (info.status === "running") {
          return {
            label: "Restart",
            icon: "rotate-cw",
            tone: "neutral",
            run: () => void safeInvoke("restart_pc"),
          };
        }
        return {
          label: "Start",
          icon: "play",
          tone: "accent",
          run: () => void safeInvoke("restart_pc"),
        };

      case "caddy":
        return {
          label: "Wire in card #5b",
          icon: "info",
          tone: "neutral",
          run: () => {
            // Caddy sidecar startup is a separate follow-up; this card
            // wires the visual surface but Caddy's setup is in another card.
            console.info("[caddy] startup wired in a follow-up card");
          },
        };

      case "mkcertCa":
        if (info.status === "not_installed") {
          return {
            label: "Install docs",
            icon: "external-link",
            tone: "warn",
            run: () => void openUrl("https://github.com/FiloSottile/mkcert"),
          };
        }
        return null;

      case "hostsHelper":
        return {
          label: "Reconcile",
          icon: "rotate-cw",
          tone: "neutral",
          run: async () => {
            try {
              await safeInvoke<number>("reconcile_hosts");
            } catch {
              // safeInvoke already pushed the toast (likely permission-denied).
            }
          },
        };
    }
  });

  const buttonClass = $derived.by(() => {
    if (!action) return "";
    const base =
      "inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs font-medium border transition-colors";
    switch (action.tone) {
      case "accent":
        return `${base} text-accent border-accent/40 hover:bg-accent/10 hover:border-accent/60`;
      case "danger":
        return `${base} text-status-crashed border-status-crashed/40 hover:bg-status-crashed/10`;
      case "warn":
        return `${base} text-status-unhealthy border-status-unhealthy/40 hover:bg-status-unhealthy/10`;
      default:
        return `${base} text-fg-muted border-border hover:text-fg hover:bg-surface-2 hover:border-border-strong`;
    }
  });

  // Suppress the "loading…" placeholder until we have real data.
  const hasRealData = $derived(info.detail !== "loading…");
</script>

<DashboardCard title={sidecarTitle[sidecarKey]} tone={cardTone} flush>
  {#snippet badge()}
    <StatusPill status={pillStatus} />
  {/snippet}
  {#snippet footer()}
    {#if info.lastError}
      <span class="text-xs text-status-crashed">{info.lastError}</span>
    {:else if info.detail}
      <span class="text-xs text-fg-muted">{info.detail}</span>
    {:else}
      <span class="text-xs text-fg-subtle">—</span>
    {/if}
  {/snippet}

  <div class="flex items-center justify-between gap-3">
    <span class="text-xs text-fg-muted truncate">{info.name}</span>
    {#if action && hasRealData}
      <button
        type="button"
        onclick={() => void action.run()}
        title={action.label}
        class={buttonClass}
      >
        <Icon name={action.icon} size={12} />
        {action.label}
      </button>
    {:else if !hasRealData}
      <span class="text-xs text-fg-subtle animate-pulse">…</span>
    {/if}
  </div>
</DashboardCard>
