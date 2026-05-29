<!--
  CustomTunnelField — attach a bring-your-own named Cloudflare tunnel to a
  project (Pro). Shared by the project's Advanced settings and the Tunnels page.

  When a tunnel is attached, the project's "Share" routes through the user's
  named tunnel (stable hostname) instead of a random *.trycloudflare.com link.
  PortBay generates its own ingress config from the picked tunnel + hostname and
  never touches `~/.cloudflared/`. Pro-gated, mirroring the domain-suffix gate:
  non-Pro sees the control locked with an upgrade affordance; an already-attached
  tunnel keeps working (Share still falls back to Quick if Pro lapses).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import type { ProjectView } from "$lib/types/projects";
  import type { DetectedTunnel } from "$lib/types/tunnel";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  const locked = $derived(!entitlements.isPro);
  const attached = $derived(!!project.tunnel?.tunnelId);

  let detected = $state<DetectedTunnel[]>([]);
  let loaded = $state(false);
  // Seeded once in onMount from the attached config (a reactive `$state`
  // initializer reading `project` would only capture its initial value).
  let selectedUuid = $state("");
  let hostname = $state("");
  let busy = $state(false);

  onMount(() => {
    selectedUuid = project.tunnel?.tunnelId ?? "";
    hostname = project.tunnel?.hostname ?? "";
    void tunnels.listNamedTunnels().then((list) => {
      detected = list;
      loaded = true;
      if (!hostname && selectedUuid) {
        const m = list.find((t) => t.uuid === selectedUuid);
        if (m?.suggestedHostname) hostname = m.suggestedHostname;
      }
    });
  });

  function onSelect(uuid: string) {
    selectedUuid = uuid;
    const m = detected.find((t) => t.uuid === uuid);
    if (m?.suggestedHostname && !hostname.trim()) hostname = m.suggestedHostname;
  }

  // Credentials file for the selected tunnel (or the already-attached one).
  const credentialsFile = $derived(
    detected.find((t) => t.uuid === selectedUuid)?.credentialsFile ??
      project.tunnel?.credentialsFile ??
      "",
  );

  const canSave = $derived(
    !locked && !busy && !!selectedUuid && !!hostname.trim() && !!credentialsFile,
  );

  async function save() {
    if (!canSave) return;
    busy = true;
    try {
      await safeInvoke<ProjectView>("update_project", {
        id: project.id,
        patch: {
          tunnel: {
            tunnelId: selectedUuid,
            credentialsFile,
            hostname: hostname.trim(),
          },
        },
      });
      await projects.refresh();
      errorBus.push({
        code: "TUNNEL_ATTACHED",
        whatHappened: `Custom tunnel attached at ${hostname.trim()}.`,
        whyItMatters: "Sharing this project now uses your stable hostname.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* safeInvoke toasts (incl. ProRequired) */
    } finally {
      busy = false;
    }
  }

  async function remove() {
    busy = true;
    try {
      // A blank (inactive) config clears the attachment server-side.
      await safeInvoke<ProjectView>("update_project", {
        id: project.id,
        patch: { tunnel: { tunnelId: "", credentialsFile: "", hostname: "" } },
      });
      selectedUuid = "";
      hostname = "";
      await projects.refresh();
    } catch {
      /* toast */
    } finally {
      busy = false;
    }
  }
</script>

<div class="space-y-2.5">
  <div class="flex items-center justify-between gap-3">
    <div class="min-w-0">
      <span class="text-[13px] text-fg">Custom domain (named tunnel)</span>
      <p class="text-[11px] text-fg-subtle mt-0.5 max-w-md leading-relaxed">
        Attach your own Cloudflare named tunnel so Share uses a stable hostname
        (for OAuth callbacks, webhooks, staging). PortBay runs it from its own
        config — your <code class="font-mono">~/.cloudflared</code> is never touched.
      </p>
    </div>
    {#if locked}
      <button
        type="button"
        onclick={() => account.open({ intent: "pro" })}
        class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium
               text-accent border border-accent/40 hover:bg-accent/10 transition-colors shrink-0"
      >
        <Icon name="lock" size={12} /> Pro
      </button>
    {/if}
  </div>

  {#if !locked}
    {#if loaded && detected.length === 0}
      <p class="text-[11.5px] text-fg-subtle">
        No named tunnels found in <code class="font-mono">~/.cloudflared</code>. Create one with
        <code class="font-mono">cloudflared tunnel create</code> and route your hostname to it first.
      </p>
    {:else}
      <div class="flex flex-wrap items-center gap-2">
        <select
          value={selectedUuid}
          onchange={(e) => onSelect((e.currentTarget as HTMLSelectElement).value)}
          disabled={busy}
          class="h-8 w-64 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg
                 focus:outline-none focus:border-accent/60 disabled:opacity-50"
        >
          <option value="">Select a tunnel…</option>
          {#each detected as t (t.uuid)}
            <option value={t.uuid}>{t.suggestedHostname ?? t.uuid}</option>
          {/each}
        </select>
        <input
          type="text"
          bind:value={hostname}
          placeholder="app.example.com"
          disabled={busy}
          class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg font-mono
                 focus:outline-none focus:border-accent/60 disabled:opacity-50"
        />
        <button
          type="button"
          onclick={save}
          disabled={!canSave}
          class="h-8 px-3 rounded-md text-[12px] text-accent border border-accent/40
                 hover:bg-accent/10 transition-colors disabled:opacity-50"
        >
          {busy ? "Saving…" : attached ? "Update" : "Attach"}
        </button>
        {#if attached}
          <button
            type="button"
            onclick={remove}
            disabled={busy}
            class="h-8 px-3 rounded-md text-[12px] text-fg-subtle hover:text-status-crashed
                   transition-colors disabled:opacity-50"
          >
            Remove
          </button>
        {/if}
      </div>
      {#if attached}
        <p class="text-[11px] text-status-running">
          Attached — Share opens <span class="font-mono">https://{project.tunnel?.hostname}</span>.
        </p>
      {/if}
    {/if}
  {/if}
</div>
