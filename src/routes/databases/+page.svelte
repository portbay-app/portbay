<!--
  /databases — tabbed IDE workspace.
  Left column: DbNavigator (the in-section sidebar) — lists database instances,
  each expanding into its table tree + feature shortcuts, driving dbWorkspace.
  Right column: DbTabBar + the active doc, keyed by tab id.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import DbNavigator from "$lib/components/databases/DbNavigator.svelte";
  import DbTabBar from "$lib/components/databases/DbTabBar.svelte";
  import OverviewDoc from "$lib/components/databases/docs/OverviewDoc.svelte";
  import TableDoc from "$lib/components/databases/docs/TableDoc.svelte";
  import QueryDoc from "$lib/components/databases/docs/QueryDoc.svelte";
  import QueryBuilderDoc from "$lib/components/databases/docs/QueryBuilderDoc.svelte";
  import ErdDoc from "$lib/components/databases/docs/ErdDoc.svelte";
  import ExplainDoc from "$lib/components/databases/docs/ExplainDoc.svelte";

  import { databases } from "$lib/stores/databases.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { dbWorkspace } from "$lib/stores/dbWorkspace.svelte";

  onMount(() => {
    void databases.refresh();
    void projects.start();

    // Honor ?db=<instanceId> query param for deep-linking.
    const dbParam = $page.url.searchParams.get("db");
    if (dbParam) {
      dbWorkspace.selectInstance(dbParam);
    } else if (databases.selectedId) {
      dbWorkspace.selectInstance(databases.selectedId);
    }
  });

  const activeTab = $derived(dbWorkspace.activeTab);
  const activeInstanceId = $derived(dbWorkspace.activeInstanceId);
  const visibleTabs = $derived(dbWorkspace.visibleTabs);

  /** Look up the full DatabaseInstanceView for a given instanceId. */
  function getInstance(instanceId: string) {
    return databases.value.find((d) => d.id === instanceId) ?? null;
  }

  const showEmptyState = $derived(
    activeInstanceId === null || visibleTabs.length === 0,
  );
</script>

<svelte:head>
  <title>Databases — PortBay</title>
</svelte:head>

<div class="h-full flex">
  <DbNavigator />

  <div class="flex-1 min-w-0 flex flex-col">
    {#if showEmptyState}
      <!-- No instance selected -->
      <div class="h-full flex items-center justify-center">
        <div class="text-center max-w-sm px-6">
          <Icon name="database" size={28} class="text-fg-subtle mx-auto" />
          <p class="mt-3 text-[13px] text-fg-muted">
            {databases.value.length === 0
              ? "Add a database to get started."
              : "Select a database on the left to open its workspace."}
          </p>
        </div>
      </div>
    {:else}
      <!-- Tab bar -->
      <DbTabBar />

      <!-- Doc host — keyed so each tab keeps its own state -->
      <div class="flex-1 min-h-0 overflow-hidden">
        {#if activeTab}
          {#key activeTab.id}
            {@const instance = getInstance(activeTab.instanceId)}
            {#if instance}
              {#if activeTab.kind === "overview"}
                <OverviewDoc {instance} />
              {:else if activeTab.kind === "table"}
                <TableDoc {instance} schema={activeTab.schema} table={activeTab.table ?? ""} />
              {:else if activeTab.kind === "query"}
                <QueryDoc {instance} schema={activeTab.schema} initialSql={activeTab.sql ?? ""} />
              {:else if activeTab.kind === "build"}
                <QueryBuilderDoc {instance} />
              {:else if activeTab.kind === "erd"}
                <ErdDoc {instance} />
              {:else if activeTab.kind === "explain"}
                <ExplainDoc {instance} sql={activeTab.sql ?? ""} schema={activeTab.schema} />
              {/if}
            {:else}
              <!-- Instance not yet loaded (brief flash on first mount) -->
              <div class="h-full flex items-center justify-center">
                <p class="text-[12px] text-fg-subtle">Loading…</p>
              </div>
            {/if}
          {/key}
        {/if}
      </div>
    {/if}
  </div>
</div>
