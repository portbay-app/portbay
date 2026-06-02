import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import type {
  SaveSshIdentityInput,
  SshIdentityView,
} from "$lib/types/sshIdentities";

/** Saved reusable SSH identities. Loaded on demand and after each mutation. */
function createSshIdentitiesStore() {
  let entries = $state<SshIdentityView[]>([]);
  let loaded = $state(false);
  let busy = $state<Record<string, boolean>>({});

  async function refresh(): Promise<void> {
    if (!browser) return;
    try {
      entries = await safeInvoke<SshIdentityView[]>("ssh_identities_list");
      loaded = true;
    } catch {
      /* safeInvoke pushed the toast */
    }
  }

  function setBusy(id: string, value: boolean) {
    busy = { ...busy, [id]: value };
  }

  function isBusy(id: string): boolean {
    return busy[id] === true;
  }

  async function save(
    input: SaveSshIdentityInput,
  ): Promise<SshIdentityView | null> {
    const key = input.id ?? "__new";
    if (isBusy(key)) return null;
    setBusy(key, true);
    try {
      const saved = await safeInvoke<SshIdentityView>("ssh_identity_save", {
        input,
      });
      await refresh();
      return saved;
    } catch {
      return null;
    } finally {
      setBusy(key, false);
    }
  }

  async function remove(id: string): Promise<boolean> {
    if (isBusy(id)) return false;
    setBusy(id, true);
    try {
      await safeInvoke("ssh_identity_delete", { id });
      await refresh();
      return true;
    } catch {
      return false;
    } finally {
      setBusy(id, false);
    }
  }

  return {
    get value() {
      return entries;
    },
    get loaded() {
      return loaded;
    },
    get count() {
      return entries.length;
    },
    find(id: string): SshIdentityView | undefined {
      return entries.find((i) => i.id === id);
    },
    isBusy,
    refresh,
    save,
    remove,
  };
}

export const sshIdentities = createSshIdentitiesStore();
