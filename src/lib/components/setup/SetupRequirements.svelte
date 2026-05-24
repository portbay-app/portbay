<!--
  SetupRequirements — the actionable companion to the dashboard's
  "tools need setup" banner. Lists every unmet requirement with what's wrong
  and a one-click remedy: either run the fix inline (install the CA, restart a
  sidecar) or route to the page that owns the longer flow (DNS, Services).

  Self-hides when everything is healthy. Reads the same `setupRequirements`
  derivation the banner uses, so the two never disagree.
-->
<script lang="ts">
  import { goto } from "$app/navigation";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { setupRequirements } from "$lib/stores/setup";
  import { safeInvoke } from "$lib/ipc";

  const reqs = $derived(setupRequirements(sidecars.value));
  let busy = $state<string | null>(null);

  async function runFix(key: string, command: string) {
    busy = key;
    try {
      await safeInvoke(command);
      // Reflect the result immediately rather than waiting for the 3 s poll;
      // a successful fix drops the row, a failure already toasted.
      await sidecars.refresh();
    } finally {
      busy = null;
    }
  }
</script>

{#if reqs.length > 0}
  <section
    id="setup"
    class="bg-surface border border-amber-500/30 rounded-2xl p-5 scroll-mt-4"
  >
    <div class="flex items-center gap-2.5 mb-3">
      <span
        class="inline-flex items-center justify-center w-8 h-8 rounded-lg
               bg-amber-500/15 text-amber-400"
      >
        <Icon name="circle-alert" size={15} />
      </span>
      <div class="flex flex-col">
        <span class="text-[14px] font-semibold text-fg">Setup required</span>
        <span class="text-[11px] text-fg-subtle"
          >{reqs.length}
          {reqs.length === 1 ? "tool needs" : "tools need"} attention before everything
          works.</span
        >
      </div>
    </div>

    <div class="divide-y divide-border/60">
      {#each reqs as r (r.key)}
        <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0">
          <div class="flex flex-col min-w-0">
            <span class="text-[13px] text-fg">{r.title}</span>
            <span class="text-[11px] text-fg-subtle truncate">{r.detail}</span>
          </div>
          {#if r.remedy.kind === "fix"}
            {@const fix = r.remedy}
            <button
              type="button"
              disabled={busy === r.key}
              onclick={() => runFix(r.key, fix.command)}
              class="shrink-0 h-8 px-3 rounded-md text-[12px] text-accent border border-accent/40
                     hover:bg-accent/10 transition-colors disabled:opacity-50"
            >
              {busy === r.key ? fix.busyLabel : fix.label}
            </button>
          {:else}
            {@const route = r.remedy}
            <button
              type="button"
              onclick={() => goto(route.href)}
              class="shrink-0 h-8 px-3 rounded-md text-[12px] text-fg-muted border border-border
                     hover:text-fg hover:bg-surface-2 transition-colors"
            >
              {route.label} →
            </button>
          {/if}
        </div>
      {/each}
    </div>
  </section>
{/if}
