/**
 * Setup requirements — turns raw sidecar health into a concrete, *actionable*
 * to-do list. The dashboard banner and the Settings "Setup" surface both read
 * from this one function so they can never disagree about what's missing.
 *
 * Each requirement carries a remedy: either an in-app **fix** (an IPC command
 * we can run on the user's behalf — install the CA, restart a sidecar) or a
 * **route** to the page that owns the fix (DNS, Services). No requirement is
 * ever a dead end.
 */
import type {
  SidecarHealth,
  SidecarKey,
  SidecarState,
} from "$lib/types/sidecars";
import { sidecarTitle, SIDECAR_ORDER } from "$lib/types/sidecars";

export type SetupRemedy =
  | { kind: "fix"; label: string; command: string; busyLabel: string }
  | { kind: "route"; label: string; href: string };

export interface SetupRequirement {
  key: SidecarKey;
  title: string;
  /** What's wrong, in the sidecar's own words. */
  detail: string;
  remedy: SetupRemedy;
}

/** How each sidecar gets fixed. A `fix` runs the IPC command inline; a `route`
    sends the user to the page that owns the longer flow. */
const REMEDY: Record<SidecarKey, SetupRemedy> = {
  processCompose: {
    kind: "fix",
    label: "Restart",
    busyLabel: "Restarting…",
    command: "restart_pc",
  },
  caddy: {
    kind: "fix",
    label: "Restart Caddy",
    busyLabel: "Restarting…",
    command: "restart_caddy",
  },
  mkcertCa: {
    kind: "fix",
    label: "Install local CA",
    busyLabel: "Installing…",
    command: "install_mkcert_ca",
  },
  dnsmasq: { kind: "route", label: "Set up DNS", href: "/dns" },
  mailpit: { kind: "route", label: "Open Services", href: "/services" },
  hostsHelper: {
    kind: "fix",
    label: "Install helper",
    busyLabel: "Installing…",
    command: "install_privileged_helper",
  },
};

/** A sidecar needs the user's attention when it's missing or unreachable — or,
    for the local CA, present but not yet trusted (it reports `stopped`). A
    benign `stopped` (an optional sidecar the user simply hasn't started) does
    NOT count, so the banner never nags about something that's actually fine. */
function needsAttention(key: SidecarKey, status: SidecarState): boolean {
  if (status === "not_installed" || status === "unreachable") return true;
  if (key === "mkcertCa" && status === "stopped") return true;
  return false;
}

/** The unmet setup requirements, in dashboard order. Empty when everything is
    healthy — both the banner and the Settings surface hide themselves then. */
export function setupRequirements(
  health: SidecarHealth,
): SetupRequirement[] {
  return SIDECAR_ORDER.flatMap((key) => {
    const s = health[key];
    if (!s || !needsAttention(key, s.status)) return [];
    return [
      {
        key,
        title: sidecarTitle[key],
        detail: s.detail ?? s.lastError ?? "Needs setup.",
        remedy: REMEDY[key],
      },
    ];
  });
}
