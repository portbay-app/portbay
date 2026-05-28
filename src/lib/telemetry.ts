/**
 * Opt-in product telemetry — named funnel events.
 *
 * These complement the per-command success/failure telemetry already recorded
 * in Rust; this is the small, deliberate set of *product* moments we care about
 * for the Pro funnel (activation → cap → upgrade). Each event is forwarded by
 * the Rust `record_telemetry_event` command to the PortBay Cloud Worker, which
 * is a no-op unless the user has explicitly enabled telemetry — so callers can
 * fire unconditionally and never leak data from a user who hasn't opted in.
 *
 * Fire-and-forget by design: a telemetry call must never toast, throw into the
 * caller, or block the interaction it is measuring.
 */
import { invokeQuiet } from "$lib/ipc";

export type TelemetryEventName =
  /** A project was successfully started/served (activation). */
  | "project_started"
  /** The add-project flow was blocked by the tier's project cap. */
  | "project_limit_reached"
  /** The user opened the upgrade / license detail surface. */
  | "upgrade_dialog_viewed";

/** Record a funnel event. Never throws; silently no-ops when telemetry is off. */
export function trackEvent(name: TelemetryEventName, ok = true): void {
  void invokeQuiet("record_telemetry_event", { commandName: name, ok }).catch(
    () => {},
  );
}
