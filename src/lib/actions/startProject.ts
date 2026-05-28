/**
 * Start a project, resolving a port conflict interactively.
 *
 * On `PORT_CONFLICT` it asks the user — naming the exact holder — whether to
 * force-quit it and start anyway, then calls `force_start_project`. Killing a
 * process PortBay didn't start is destructive, so the confirmation is mandatory
 * and the dialog spells out what happens.
 *
 * Returns a discriminated {@link StartResult} so callers can tell *started*
 * from *declined* (the user backed out of the force-quit) from *error*. That
 * distinction matters for the optimistic Play overlay: a decline must roll the
 * overlay back (nothing is starting), whereas a success leaves it for the real
 * status event to resolve. Centralising the conflict UX here keeps every Play
 * button behaving identically.
 */
import { invokeQuiet } from "$lib/ipc";
import { confirmDialog } from "$lib/stores/confirm.svelte";
import { trackEvent } from "$lib/telemetry";
import type { CommandError } from "$lib/types/error";

export type StartResult =
  | { kind: "started" }
  | { kind: "declined" }
  | { kind: "error"; error: CommandError };

export async function startProject(
  id: string,
  name: string,
): Promise<StartResult> {
  try {
    await invokeQuiet<void>("start_project", { id });
    trackEvent("project_started");
    return { kind: "started" };
  } catch (raw) {
    const err = raw as CommandError;
    if (err.code !== "PORT_CONFLICT") return { kind: "error", error: err };

    const choice = await confirmDialog.open({
      title: "Port already in use",
      message: `${err.whatHappened}\n\nForce-quit that process and start “${name}”? It's sent SIGTERM, then SIGKILL if it doesn't exit.`,
      actions: [
        { label: "Stop it & start", value: "force", tone: "destructive" },
      ],
      destructive: true,
    });
    if (choice !== "force") return { kind: "declined" }; // user backed out

    try {
      await invokeQuiet<void>("force_start_project", { id });
      trackEvent("project_started");
      return { kind: "started" };
    } catch (raw2) {
      // e.g. the holder is root-owned and couldn't be killed — surface it.
      return { kind: "error", error: raw2 as CommandError };
    }
  }
}
