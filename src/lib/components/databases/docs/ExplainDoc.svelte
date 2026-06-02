<script lang="ts">
  /**
   * ExplainDoc — Visual EXPLAIN for a SQL query.
   * Runs database_client_explain on mount and when sql changes.
   * Header: SQL preview + Re-run button + ANALYZE toggle.
   */
  import Icon from "$lib/components/atoms/Icon.svelte";
  import ExplainView from "$lib/components/databases/ExplainView.svelte";

  import { safeInvoke } from "$lib/ipc";
  import type { DatabaseInstanceView, DbExplainPlan } from "$lib/types/databases";

  interface Props {
    instance: DatabaseInstanceView;
    sql: string | undefined;
    schema: string | null | undefined;
  }

  let { instance, sql = "", schema }: Props = $props();

  let plan = $state<DbExplainPlan | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let analyze = $state(false);

  async function runExplain() {
    if (!sql.trim()) return;
    loading = true;
    error = null;
    try {
      plan = await safeInvoke<DbExplainPlan>("database_client_explain", {
        id: instance.id,
        schema: schema ?? null,
        sql,
        analyze,
      });
    } catch (err) {
      plan = null;
      const msg = err instanceof Error ? err.message : (typeof err === "string" ? err : null);
      error = msg || "Could not explain this query.";
    } finally {
      loading = false;
    }
  }

  // Run whenever sql, analyze, or instance changes (also fires on mount).
  $effect(() => {
    void sql;
    void analyze;
    void instance.id;
    void runExplain();
  });

  const sqlPreview = $derived(
    sql.length > 120 ? `${sql.slice(0, 120)}…` : sql,
  );
</script>

<div class="h-full flex flex-col min-h-0">
  <!-- Header -->
  <div
    class="shrink-0 px-4 py-3 border-b border-border/60 bg-surface/60
           flex items-start justify-between gap-3 flex-wrap"
  >
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2 mb-1">
        <Icon name="activity" size={13} class="text-fg-muted shrink-0" />
        <span class="text-[13px] font-semibold text-fg">Visual Explain</span>
      </div>
      {#if sql}
        <p
          class="text-[11px] font-mono text-fg-subtle truncate max-w-xl"
          title={sql}
        >
          {sqlPreview}
        </p>
      {/if}
    </div>

    <div class="flex items-center gap-3 shrink-0">
      <label
        class="inline-flex items-center gap-1.5 text-[11px] text-fg-subtle
               select-none cursor-pointer"
        title="Run EXPLAIN ANALYZE for real timings (PostgreSQL)"
      >
        <input
          type="checkbox"
          bind:checked={analyze}
          class="accent-accent"
        />
        ANALYZE
      </label>

      <button
        type="button"
        onclick={runExplain}
        disabled={loading || !sql.trim()}
        class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
               border border-border bg-surface text-[12px] text-fg-muted
               hover:bg-surface-2 hover:text-fg transition-colors
               disabled:opacity-50 disabled:cursor-not-allowed"
      >
        <Icon
          name={loading ? "refresh-cw" : "play"}
          size={11}
          class={loading ? "animate-spin" : ""}
        />
        Re-run
      </button>
    </div>
  </div>

  <!-- ExplainView fills remaining height -->
  <div class="flex-1 min-h-0 overflow-hidden">
    <ExplainView {plan} isLoading={loading} {error} />
  </div>
</div>
