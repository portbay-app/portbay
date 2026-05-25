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
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import type {
    PortbayStatus,
  } from "$lib/types/status";
  import type { SidecarKey, SidecarStatus } from "$lib/types/sidecars";
  import { sidecarTitle } from "$lib/types/sidecars";
  import { openUrl } from "$lib/security/openUrl";

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

  // Action button shape per (sidecar × state). Each sidecar has a
  // status-aware action (Start when stopped, Restart when running,
  // Open inbox for Mailpit, Install CA for mkcert, etc.).
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
        if (info.status === "running") {
          return {
            label: "Restart",
            icon: "rotate-cw",
            tone: "neutral",
            run: async () => {
              try {
                await safeInvoke("restart_caddy");
                await sidecars.refresh();
              } catch {
                /* toast pushed */
              }
            },
          };
        }
        return {
          label: "Start",
          icon: "play",
          tone: "accent",
          run: async () => {
            try {
              await safeInvoke("restart_caddy");
              await sidecars.refresh();
            } catch {
              /* toast pushed */
            }
          },
        };

      case "mkcertCa":
        if (info.status === "not_installed") {
          // Bundled binary missing entirely — point at upstream docs as
          // a last-resort hint.
          return {
            label: "Install docs",
            icon: "external-link",
            tone: "warn",
            run: () => void openUrl("https://github.com/FiloSottile/mkcert"),
          };
        }
        if (info.status === "stopped") {
          // Binary present, CA not yet installed in system keychain.
          return {
            label: "Install local CA",
            icon: "check",
            tone: "accent",
            run: async () => {
              try {
                await safeInvoke("install_mkcert_ca");
                await sidecars.refresh();
              } catch {
                // safeInvoke already pushed the toast (likely user-cancelled).
              }
            },
          };
        }
        return null;

      case "dnsmasq":
        if (info.status === "running") {
          return {
            label: "Restart",
            icon: "rotate-cw",
            tone: "neutral",
            run: async () => {
              try {
                await safeInvoke("restart_dnsmasq");
                await sidecars.refresh();
              } catch {
                /* toast pushed */
              }
            },
          };
        }
        if (info.status === "stopped") {
          // Binary on PATH but the daemon isn't running. The settings
          // page exposes the install/uninstall resolver flow; for now,
          // surface a "Start" affordance via restart_dnsmasq.
          return {
            label: "Start",
            icon: "play",
            tone: "accent",
            run: async () => {
              try {
                await safeInvoke("restart_dnsmasq");
                await sidecars.refresh();
              } catch {
                /* toast pushed */
              }
            },
          };
        }
        return null;

      case "mailpit":
        if (info.status === "running") {
          // Extract the UI port from `smtp :1025 · ui :8025` so the
          // button knows where to point.
          const m = info.detail?.match(/ui :(\d+)/);
          const uiPort = m ? Number(m[1]) : 8025;
          return {
            label: "Open inbox",
            icon: "external-link",
            tone: "accent",
            run: () => void openUrl(`http://127.0.0.1:${uiPort}`),
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
