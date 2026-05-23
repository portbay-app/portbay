/**
 * Groups store — registry-side list of project clusters.
 *
 * Refreshed manually (no polling — groups don't change without a user
 * action) and consumed by:
 *   - Sidebar "Groups" submenu
 *   - /groups/[id] filtered route header
 *   - GroupEditorModal (create / edit flow)
 */
import { safeInvoke } from "$lib/ipc";
import type { GroupInput, GroupPatch, GroupView } from "$lib/types/groups";

function createGroupsStore() {
  let items = $state<GroupView[]>([]);
  let loading = $state<boolean>(false);

  async function refresh(): Promise<void> {
    loading = true;
    try {
      items = await safeInvoke<GroupView[]>("list_groups");
    } finally {
      loading = false;
    }
  }

  async function add(input: GroupInput): Promise<GroupView> {
    const view = await safeInvoke<GroupView>("add_group", { input });
    items = [...items, view];
    return view;
  }

  async function update(id: string, patch: GroupPatch): Promise<GroupView> {
    const view = await safeInvoke<GroupView>("update_group", { id, patch });
    items = items.map((g) => (g.id === id ? view : g));
    return view;
  }

  async function remove(id: string): Promise<void> {
    await safeInvoke<void>("remove_group", { id });
    items = items.filter((g) => g.id !== id);
  }

  function get(id: string): GroupView | null {
    return items.find((g) => g.id === id) ?? null;
  }

  return {
    get value() {
      return items;
    },
    get loading() {
      return loading;
    },
    refresh,
    add,
    update,
    remove,
    get,
  };
}

export const groups = createGroupsStore();
