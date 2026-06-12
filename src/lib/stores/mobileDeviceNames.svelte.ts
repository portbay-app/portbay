/**
 * Resolved device-id → name cache, fed by every destination enumeration
 * (MobileDestinationPicker.refresh). Row chips render outside the picker and
 * have no enumeration of their own — without this they show raw UDIDs /
 * adb serials. Persisted to localStorage so a pinned device's name appears
 * immediately on boot, before the first (multi-second) simctl/devicectl scan
 * lands; the scan then refreshes any rename.
 */
import { deviceShortLabel, type MobileRunTarget } from "$lib/types/mobile";

const KEY = "portbay:mobile-device-names";

function load(): Record<string, string> {
  try {
    const raw = localStorage.getItem(KEY);
    const obj = raw ? (JSON.parse(raw) as unknown) : {};
    if (obj && typeof obj === "object" && !Array.isArray(obj)) {
      return Object.fromEntries(
        Object.entries(obj as Record<string, unknown>).filter(
          (e): e is [string, string] => typeof e[1] === "string",
        ),
      );
    }
  } catch {
    /* corrupt / unavailable — start empty, it's a display nicety */
  }
  return {};
}

const names = $state<Record<string, string>>(load());

export const mobileDeviceNames = {
  /** Fold an enumeration into the cache (and persist on change). */
  remember(targets: MobileRunTarget[]) {
    let changed = false;
    for (const t of targets) {
      if (names[t.id] !== t.name) {
        names[t.id] = t.name;
        changed = true;
      }
    }
    if (changed) {
      try {
        localStorage.setItem(KEY, JSON.stringify(names));
      } catch {
        /* storage full / unavailable */
      }
    }
  },

  /** Best label available: cached real name, else the heuristic short label. */
  label(device: string | null | undefined): string {
    if (device && names[device]) return names[device];
    return deviceShortLabel(device);
  },
};
