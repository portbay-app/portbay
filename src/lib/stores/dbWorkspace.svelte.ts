/**
 * Database workspace store — the open-document model for the tabbed DB IDE.
 *
 * The global sidebar drives navigation (instance → tables / features); this
 * store holds what's actually open in the main area. Tabs are global but each
 * carries its `instanceId`, and the tab bar shows only the tabs for the active
 * instance — so each database keeps its own set of open documents. Re-opening
 * a table / ERD / overview focuses the existing tab instead of duplicating.
 *
 * Schemas are loaded once per instance and cached here so the sidebar tree and
 * every open doc share a single fetch.
 */
import { safeInvoke } from "$lib/ipc";
import type { DbClientSchema } from "$lib/types/databases";

export type DbTabKind = "overview" | "table" | "query" | "erd" | "explain" | "build";

export interface DbTab {
  id: string;
  instanceId: string;
  kind: DbTabKind;
  title: string;
  /** Closeable in the UI (Overview is pinned per instance). */
  closable: boolean;
  /** For `table`: the table name and its schema. */
  table?: string;
  schema?: string | null;
  /** For `query` / `explain`: the SQL the doc operates on. */
  sql?: string;
}

interface SchemaEntry {
  schema: DbClientSchema | null;
  loading: boolean;
  error: string | null;
}

function createDbWorkspace() {
  let tabs = $state<DbTab[]>([]);
  let activeTabId = $state<string | null>(null);
  let activeInstanceId = $state<string | null>(null);

  /** Last-focused tab per instance, so switching instances restores context. */
  const lastActiveByInstance = new Map<string, string>();

  /** Per-instance schema cache (shared by the sidebar tree and open docs). */
  let schemas = $state<Record<string, SchemaEntry>>({});

  let tabSeq = 0;
  function nextId(kind: DbTabKind): string {
    tabSeq += 1;
    return `${kind}-${tabSeq}`;
  }

  function focus(id: string) {
    const tab = tabs.find((t) => t.id === id);
    if (!tab) return;
    activeTabId = id;
    activeInstanceId = tab.instanceId;
    lastActiveByInstance.set(tab.instanceId, id);
  }

  function find(predicate: (t: DbTab) => boolean): DbTab | undefined {
    return tabs.find(predicate);
  }

  /** Open (or focus) the pinned Overview tab for an instance and select it. */
  function selectInstance(instanceId: string) {
    activeInstanceId = instanceId;
    const remembered = lastActiveByInstance.get(instanceId);
    if (remembered && tabs.some((t) => t.id === remembered)) {
      focus(remembered);
      return;
    }
    const existingForInstance = tabs.find((t) => t.instanceId === instanceId);
    if (existingForInstance) {
      focus(existingForInstance.id);
      return;
    }
    openOverview(instanceId);
  }

  function openOverview(instanceId: string) {
    const existing = find((t) => t.instanceId === instanceId && t.kind === "overview");
    if (existing) return focus(existing.id);
    const tab: DbTab = {
      id: nextId("overview"),
      instanceId,
      kind: "overview",
      title: "Overview",
      closable: false,
    };
    tabs = [...tabs, tab];
    focus(tab.id);
  }

  function openTable(instanceId: string, schema: string | null, table: string) {
    const existing = find(
      (t) =>
        t.instanceId === instanceId &&
        t.kind === "table" &&
        t.table === table &&
        (t.schema ?? null) === (schema ?? null),
    );
    if (existing) return focus(existing.id);
    const tab: DbTab = {
      id: nextId("table"),
      instanceId,
      kind: "table",
      title: table,
      closable: true,
      table,
      schema,
    };
    tabs = [...tabs, tab];
    focus(tab.id);
  }

  /** Always opens a fresh query scratchpad (multiple are allowed). */
  function openQuery(instanceId: string, schema: string | null = null, sql = "") {
    const count = tabs.filter((t) => t.instanceId === instanceId && t.kind === "query").length;
    const tab: DbTab = {
      id: nextId("query"),
      instanceId,
      kind: "query",
      title: count === 0 ? "Query" : `Query ${count + 1}`,
      closable: true,
      schema,
      sql,
    };
    tabs = [...tabs, tab];
    focus(tab.id);
  }

  /** Visual query builder is a singleton per instance. */
  function openBuilder(instanceId: string) {
    const existing = find((t) => t.instanceId === instanceId && t.kind === "build");
    if (existing) return focus(existing.id);
    const tab: DbTab = {
      id: nextId("build"),
      instanceId,
      kind: "build",
      title: "Query Builder",
      closable: true,
    };
    tabs = [...tabs, tab];
    focus(tab.id);
  }

  /** ERD is a singleton per instance. */
  function openErd(instanceId: string) {
    const existing = find((t) => t.instanceId === instanceId && t.kind === "erd");
    if (existing) return focus(existing.id);
    const tab: DbTab = {
      id: nextId("erd"),
      instanceId,
      kind: "erd",
      title: "ERD",
      closable: true,
    };
    tabs = [...tabs, tab];
    focus(tab.id);
  }

  /** Explain is a singleton per instance; opening it updates the SQL it runs. */
  function openExplain(instanceId: string, sql: string, schema: string | null = null) {
    const existing = find((t) => t.instanceId === instanceId && t.kind === "explain");
    if (existing) {
      existing.sql = sql;
      existing.schema = schema;
      tabs = [...tabs];
      return focus(existing.id);
    }
    const tab: DbTab = {
      id: nextId("explain"),
      instanceId,
      kind: "explain",
      title: "Visual Explain",
      closable: true,
      sql,
      schema,
    };
    tabs = [...tabs, tab];
    focus(tab.id);
  }

  function closeTab(id: string) {
    const idx = tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    const closing = tabs[idx];
    tabs = tabs.filter((t) => t.id !== id);
    if (lastActiveByInstance.get(closing.instanceId) === id) {
      lastActiveByInstance.delete(closing.instanceId);
    }
    if (activeTabId !== id) return;
    // Focus the nearest remaining tab in the same instance, else any tab.
    const sameInstance = tabs.filter((t) => t.instanceId === closing.instanceId);
    const fallback = sameInstance[Math.min(idx, sameInstance.length - 1)] ?? tabs[tabs.length - 1];
    if (fallback) focus(fallback.id);
    else {
      activeTabId = null;
    }
  }

  /** Close every tab belonging to an instance (e.g. when it's removed). */
  function closeInstance(instanceId: string) {
    tabs = tabs.filter((t) => t.instanceId !== instanceId);
    lastActiveByInstance.delete(instanceId);
    if (activeInstanceId === instanceId) {
      activeInstanceId = tabs[0]?.instanceId ?? null;
      activeTabId = tabs[0]?.id ?? null;
    }
    delete schemas[instanceId];
    schemas = { ...schemas };
  }

  /** Load (or return cached) schema for an instance. */
  async function loadSchema(instanceId: string, force = false): Promise<DbClientSchema | null> {
    const current = schemas[instanceId];
    if (!force && current && current.schema) return current.schema;
    if (current?.loading) return current.schema;
    schemas = {
      ...schemas,
      [instanceId]: { schema: current?.schema ?? null, loading: true, error: null },
    };
    try {
      const schema = await safeInvoke<DbClientSchema>("database_client_schema", {
        id: instanceId,
      });
      schemas = { ...schemas, [instanceId]: { schema, loading: false, error: null } };
      return schema;
    } catch {
      schemas = {
        ...schemas,
        [instanceId]: {
          schema: current?.schema ?? null,
          loading: false,
          error: "Could not inspect this database.",
        },
      };
      return null;
    }
  }

  return {
    get tabs() {
      return tabs;
    },
    get activeTabId() {
      return activeTabId;
    },
    get activeInstanceId() {
      return activeInstanceId;
    },
    get activeTab(): DbTab | null {
      return tabs.find((t) => t.id === activeTabId) ?? null;
    },
    /** Tabs for the currently-active instance, in open order. */
    get visibleTabs(): DbTab[] {
      if (!activeInstanceId) return [];
      return tabs.filter((t) => t.instanceId === activeInstanceId);
    },
    schemaEntry(instanceId: string): SchemaEntry {
      return schemas[instanceId] ?? { schema: null, loading: false, error: null };
    },
    selectInstance,
    openOverview,
    openTable,
    openQuery,
    openBuilder,
    openErd,
    openExplain,
    closeTab,
    closeInstance,
    focus,
    loadSchema,
  };
}

export const dbWorkspace = createDbWorkspace();
