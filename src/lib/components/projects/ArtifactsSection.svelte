<!--
  ArtifactsSection — "Artifacts" section for the project detail panel.

  Scans the project's known build-output dirs (.next, dist, node_modules,
  vendor, …) via `scan_artifacts` and shows each with its size, file count,
  and age. Per-row "Clean" and a "Clean all" use the same 2-second arm/confirm
  pattern as Remove project; cleaning re-scans and toasts the space freed.

  Renders nothing when no artifact dirs are present, so it's safe to always
  mount.
-->
<script lang="ts">
  import { DashboardCard, Icon } from "$lib/components/atoms";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import type { ArtifactDir } from "$lib/types/artifacts";
  import type { ProjectView } from "$lib/types/projects";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let artifacts = $state<ArtifactDir[]>([]);
  let scanning = $state<boolean>(false);
  /** rel keys currently being cleaned ("*" = clean-all). */
  let cleaning = $state<Set<string>>(new Set());
  /** rel keys armed for confirm ("*" = clean-all). */
  let armed = $state<Set<string>>(new Set());
  const armTimers = new Map<string, ReturnType<typeof setTimeout>>();

  const totalBytes = $derived(
    artifacts.reduce((sum, a) => sum + a.sizeBytes, 0),
  );

  async function scan() {
    scanning = true;
    disarmAll();
    try {
      artifacts = await safeInvoke<ArtifactDir[]>("scan_artifacts", {
        id: project.id,
      });
    } catch {
      artifacts = [];
    } finally {
      scanning = false;
    }
  }

  // Re-scan when the open project changes.
  $effect(() => {
    const _id = project.id;
    void scan();
  });

  function disarmAll() {
    for (const t of armTimers.values()) clearTimeout(t);
    armTimers.clear();
    armed = new Set();
  }

  function setCleaning(key: string, on: boolean) {
    const next = new Set(cleaning);
    if (on) next.add(key);
    else next.delete(key);
    cleaning = next;
  }

  /** First click arms (2s window); second click within the window runs `run`. */
  function armOrRun(key: string, run: () => Promise<void>) {
    if (armed.has(key)) {
      const t = armTimers.get(key);
      if (t) clearTimeout(t);
      armTimers.delete(key);
      armed = new Set([...armed].filter((k) => k !== key));
      void run();
      return;
    }
    armed = new Set([...armed, key]);
    const timer = setTimeout(() => {
      armed = new Set([...armed].filter((k) => k !== key));
      armTimers.delete(key);
    }, 2000);
    armTimers.set(key, timer);
  }

  function freedToast(bytes: number) {
    errorBus.push({
      code: "ARTIFACTS_CLEANED",
      whatHappened: `Freed ${formatBytes(bytes)}.`,
      whyItMatters: "",
      whoCausedIt: "system",
      severity: "success",
      actions: [],
    });
  }

  async function cleanOne(dir: ArtifactDir) {
    setCleaning(dir.rel, true);
    try {
      const freed = await safeInvoke<number>("clean_artifact", {
        id: project.id,
        rel: dir.rel,
      });
      freedToast(freed);
      await scan();
    } catch {
      /* safeInvoke toasted */
    } finally {
      setCleaning(dir.rel, false);
    }
  }

  async function cleanAll() {
    setCleaning("*", true);
    try {
      const freed = await safeInvoke<number>("clean_all_artifacts", {
        id: project.id,
      });
      freedToast(freed);
      await scan();
    } catch {
      /* safeInvoke toasted */
    } finally {
      setCleaning("*", false);
    }
  }

  function formatBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(
      Math.floor(Math.log(n) / Math.log(1024)),
      units.length - 1,
    );
    const v = n / 1024 ** i;
    return `${i === 0 || v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
  }

  function formatAge(unixSeconds: number | null): string {
    if (unixSeconds === null) return "—";
    return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  }
</script>

{#if scanning || artifacts.length > 0}
  <DashboardCard title="Artifacts" flush>
    {#if scanning}
      <p class="text-xs text-fg-subtle">Scanning build output…</p>
    {:else}
      <div class="rounded-md border border-border overflow-hidden">
        {#each artifacts as dir (dir.rel)}
          <div
            class="flex items-center gap-2 px-2.5 py-2 text-xs border-b border-border last:border-b-0"
          >
            <div class="flex-1 min-w-0">
              <div class="text-fg font-medium truncate">
                {dir.label}
                <span class="font-mono text-[10px] text-fg-subtle ml-1">{dir.rel}</span>
              </div>
              <div class="text-[11px] text-fg-muted">
                {dir.fileCount.toLocaleString()} file{dir.fileCount === 1 ? "" : "s"}
                · modified {formatAge(dir.lastModified)}
              </div>
            </div>
            <span class="font-mono text-fg tabular-nums shrink-0">
              {formatBytes(dir.sizeBytes)}
            </span>
            <button
              type="button"
              disabled={cleaning.has(dir.rel) || cleaning.has("*")}
              onclick={() => armOrRun(dir.rel, () => cleanOne(dir))}
              title={armed.has(dir.rel) ? "Click again to delete" : `Delete ${dir.rel}`}
              class="inline-flex items-center gap-1 px-2 py-1 rounded-md shrink-0 transition-colors
                     disabled:opacity-50
                     {armed.has(dir.rel)
                ? 'text-status-crashed border border-status-crashed/50 bg-status-crashed/10'
                : 'text-fg-subtle border border-border hover:text-fg hover:bg-surface-2'}"
            >
              {#if cleaning.has(dir.rel)}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="x" size={11} />
              {/if}
              {armed.has(dir.rel) ? "Confirm" : "Clean"}
            </button>
          </div>
        {/each}
      </div>

      <div class="flex items-center justify-between gap-2 mt-2">
        <span class="text-[11px] text-fg-muted">
          {formatBytes(totalBytes)} across {artifacts.length} director{artifacts.length === 1 ? "y" : "ies"}
        </span>
        <button
          type="button"
          disabled={cleaning.size > 0}
          onclick={() => armOrRun("*", cleanAll)}
          class="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md transition-colors
                 disabled:opacity-50
                 {armed.has('*')
            ? 'text-status-crashed border border-status-crashed/50 bg-status-crashed/10'
            : 'text-fg-muted border border-border hover:text-fg hover:bg-surface-2'}"
        >
          {#if cleaning.has("*")}
            <Icon name="refresh-cw" size={11} class="animate-spin" />
            Cleaning…
          {:else}
            <Icon name="x" size={11} />
            {armed.has("*") ? "Confirm clean all" : "Clean all"}
          {/if}
        </button>
      </div>
    {/if}
  </DashboardCard>
{/if}
