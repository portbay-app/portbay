/**
 * Start a project, resolving a port conflict interactively.
 *
 * On `PORT_CONFLICT` it asks the user — naming the exact holder — whether to
 * force-quit it and start anyway, then calls `force_start_project`. Killing a
 * process PortBay didn't start is destructive, so the confirmation is mandatory
 * and the dialog spells out what happens.
 *
 * Returns the **unresolved** error for the caller to surface however it likes
 * (an inline row on the dashboard, a toast in the detail panel), or `null` on
 * success or when the user declines the force-quit. Centralising the conflict
 * UX here keeps every Play button behaving identically.
 */
import { invokeQuiet } from "$lib/ipc";
import { confirmDialog } from "$lib/stores/confirm.svelte";
import type { CommandError } from "$lib/types/error";

export async function startProject(
  id: string,
  name: string,
): Promise<CommandError | null> {
  try {
    await invokeQuiet<void>("start_project", { id });
    return null;
  } catch (raw) {
    const err = raw as CommandError;
    if (err.code !== "PORT_CONFLICT") return err;

    const choice = await confirmDialog.open({
      title: "Port already in use",
      message: `${err.whatHappened}\n\nForce-quit that process and start “${name}”? It's sent SIGTERM, then SIGKILL if it doesn't exit.`,
      actions: [
        { label: "Stop it & start", value: "force", tone: "destructive" },
      ],
      destructive: true,
    });
    if (choice !== "force") return null; // user declined — not an error

    try {
      await invokeQuiet<void>("force_start_project", { id });
      return null;
    } catch (raw2) {
      // e.g. the holder is root-owned and couldn't be killed — surface it.
      return raw2 as CommandError;
    }
  }
}
