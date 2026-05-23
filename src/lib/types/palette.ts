/**
 * Command palette types.
 *
 * One `PaletteCommand` per executable action — the registry yields a
 * fresh list every time the palette opens so per-project commands
 * always reflect the current registry. Each command's `run` is fired
 * with no args; commands that need confirmation own that internally
 * (e.g. delete-group's two-tap pattern in the filtered view).
 */
import type { IconName } from "$lib/components/atoms/Icon.svelte";

/** Section header shown above each cluster of commands in the result list. */
export type PaletteGroup =
  | "Projects"
  | "Groups"
  | "Sidecars"
  | "Navigation"
  | "PHP"
  | "Tunnels"
  | "App";

export interface PaletteCommand {
  /** Stable id — used for recency tracking + dedupe. */
  id: string;
  /** Display string shown to the user. */
  label: string;
  /** Optional secondary text shown right of the label (e.g. hostname). */
  detail?: string;
  /** Section the command appears under in the result list. */
  group: PaletteGroup;
  /** Optional icon shown left of the label. */
  icon?: IconName;
  /** Optional keyboard shortcut hint shown right of the row. */
  shortcut?: string;
  /** Words appended to the haystack used by the fuzzy matcher. */
  keywords?: string[];
  /**
   * Fired when the user selects the command. The palette closes
   * automatically before this runs unless the command needs to keep
   * the palette open (rare; current callers don't).
   */
  run: () => void | Promise<void>;
}
