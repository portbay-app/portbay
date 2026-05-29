/**
 * sync — frontend wrapper over the Rust `sync` commands (Pro, E2E encrypted).
 *
 * The recovery key is generated and held by the Rust/keychain layer; the UI only
 * ever sees the display string (to show/copy) and pastes one back when adding a
 * device. Push/pull operate on the encrypted blob; the server never sees plaintext.
 */

import { safeInvoke } from "$lib/ipc";

export interface SyncStateDto {
  signed_in: boolean;
  is_pro: boolean;
  enabled: boolean;
  last_version: number;
}

export interface SyncDevice {
  id: string;
  name: string;
  platform: string;
  last_seen: string;
}

export type PushOutcome = { result: "ok"; version: number } | { result: "conflict"; remote_version: number };

export interface DeviceActivation {
  device_id: string;
  max_devices: number;
}

function createSyncStore() {
  let state = $state<SyncStateDto>({ signed_in: false, is_pro: false, enabled: false, last_version: 0 });
  let devices = $state<SyncDevice[]>([]);
  let busy = $state(false);

  async function load(): Promise<void> {
    state = await safeInvoke<SyncStateDto>("sync_state");
    if (state.enabled && state.is_pro) await refreshDevices();
  }

  async function refreshDevices(): Promise<void> {
    try {
      devices = await safeInvoke<SyncDevice[]>("list_sync_devices");
    } catch {
      devices = [];
    }
  }

  /** Generate (or return the existing) recovery key; returns the display string. */
  async function enable(): Promise<string> {
    busy = true;
    try {
      const key = await safeInvoke<string>("enable_sync");
      await load();
      return key;
    } finally {
      busy = false;
    }
  }

  async function getRecoveryKey(): Promise<string | null> {
    return safeInvoke<string | null>("get_recovery_key");
  }

  async function setRecoveryKey(key: string): Promise<void> {
    busy = true;
    try {
      await safeInvoke("set_recovery_key", { key });
      await load();
    } finally {
      busy = false;
    }
  }

  async function disable(): Promise<void> {
    busy = true;
    try {
      await safeInvoke("disable_sync");
      devices = [];
      await load();
    } finally {
      busy = false;
    }
  }

  async function push(force = false): Promise<PushOutcome> {
    busy = true;
    try {
      const out = await safeInvoke<PushOutcome>("sync_push", { force });
      if (out.result === "ok") await load();
      return out;
    } finally {
      busy = false;
    }
  }

  /** Returns true if a remote document was pulled, false if there was nothing yet. */
  async function pull(): Promise<boolean> {
    busy = true;
    try {
      const pulled = await safeInvoke<boolean>("sync_pull");
      await load();
      return pulled;
    } finally {
      busy = false;
    }
  }

  /**
   * Activate this device against the 2-device license cap. Throws (and
   * `safeInvoke` toasts) a `DEVICE_LIMIT_REACHED` error when the account is
   * already at its device limit, so the caller can prompt a deactivation.
   */
  async function activate(): Promise<DeviceActivation> {
    const out = await safeInvoke<DeviceActivation>("activate_device");
    await refreshDevices();
    return out;
  }

  async function revokeDevice(id: string): Promise<void> {
    await safeInvoke("revoke_sync_device", { deviceId: id });
    await refreshDevices();
  }

  return {
    get state() {
      return state;
    },
    get devices() {
      return devices;
    },
    get busy() {
      return busy;
    },
    load,
    refreshDevices,
    enable,
    getRecoveryKey,
    setRecoveryKey,
    disable,
    push,
    pull,
    activate,
    revokeDevice,
  };
}

export const sync = createSyncStore();
