/**
 * Palette command source — assembles the live `PaletteCommand[]` list
 * from app-level static actions + per-project + per-group + per-sidecar
 * + PHP + tunnel sources.
 *
 * Called once per palette render; treats each store's `value` reactively
 * so the palette in the open state stays in sync as the registry
 * updates (e.g. a project flips to running mid-search).
 */
import { goto } from "$app/navigation";
import { openUrl } from "@tauri-apps/plugin-opener";

import { safeInvoke } from "$lib/ipc";
import { addProjectWizard } from "$lib/stores/wizard.svelte";
import { density } from "$lib/stores/density.svelte";
import { errorBus } from "$lib/stores/errors.svelte";
import { groupEditor } from "$lib/stores/groupEditor.svelte";
import { groups } from "$lib/stores/groups.svelte";
import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
import { projects } from "$lib/stores/projects.svelte";
import { dns } from "$lib/stores/dns.svelte";
import { sidecars } from "$lib/stores/sidecars.svelte";
import { theme } from "$lib/stores/theme.svelte";
import { tunnels } from "$lib/stores/tunnels.svelte";
import type { PaletteCommand } from "$lib/types/palette";

/**
 * Build the full command list. Components call this every time the
 * palette opens or the query changes — the store reads underneath
 * make this reactive without an explicit subscription.
 */
export function collectCommands(): PaletteCommand[] {
  const cmds: PaletteCommand[] = [];

  // ────────── App-level ──────────
  cmds.push(
    {
      id: "app.add-project",
      label: "Add project",
      group: "App",
      icon: "plus",
      shortcut: "⌘N",
      keywords: ["new", "create", "register"],
      run: () => addProjectWizard.show(),
    },
    {
      id: "app.new-group",
      label: "New group",
      group: "App",
      icon: "folder",
      keywords: ["create", "cluster"],
      run: () => groupEditor.create(),
    },
    {
      id: "app.stop-all",
      label: "Stop all running projects",
      group: "App",
      icon: "square",
      shortcut: "⇧⌘.",
      keywords: ["kill", "halt"],
      run: async () => {
        try {
          await safeInvoke("stop_all");
        } catch {
          /* toast pushed */
        }
      },
    },
    {
      id: "app.toggle-density",
      label: `Switch to ${density.value === "compact" ? "comfortable" : "compact"} density`,
      group: "App",
      icon: "settings",
      keywords: ["layout", "spacing"],
      run: () => density.toggle(),
    },
    {
      id: "app.toggle-theme",
      label: `Switch to ${theme.value === "dark" ? "light" : "dark"} theme`,
      group: "App",
      icon: "settings",
      keywords: ["dark", "light", "appearance"],
      run: () => theme.toggle(),
    },
  );

  // ────────── Navigation ──────────
  for (const route of [
    { id: "/", label: "Projects", icon: "home" as const },
    { id: "/services", label: "Services", icon: "server" as const },
    { id: "/domains", label: "Domains", icon: "link" as const },
    { id: "/languages", label: "Languages", icon: "file-code" as const },
    { id: "/logs", label: "Logs", icon: "file-text" as const },
    { id: "/settings", label: "Settings", icon: "settings" as const },
  ]) {
    cmds.push({
      id: `nav.${route.id}`,
      label: `Go to ${route.label}`,
      group: "Navigation",
      icon: route.icon,
      keywords: ["open", "navigate"],
      run: () => void goto(route.id),
    });
  }

  // ────────── Per-project ──────────
  for (const p of projects.value) {
    const baseKw = [p.id, p.hostname, p.type];
    cmds.push(
      {
        id: `project.open.${p.id}`,
        label: `Open ${p.name}`,
        detail: p.hostname,
        group: "Projects",
        icon: "external-link",
        keywords: [...baseKw, "view", "detail"],
        run: () => projectDetailPanel.show(p.id),
      },
      {
        id: `project.url.${p.id}`,
        label: `Open ${p.name} in browser`,
        detail: p.url,
        group: "Projects",
        icon: "globe",
        keywords: [...baseKw, "browser"],
        run: () => void openUrl(p.url),
      },
      {
        id: `project.start.${p.id}`,
        label: `Start ${p.name}`,
        detail: p.hostname,
        group: "Projects",
        icon: "play",
        keywords: [...baseKw, "run", "boot"],
        run: async () => {
          try {
            await dns.ensureReady();
            await safeInvoke("start_project", { id: p.id });
          } catch {
            /* toast pushed */
          }
        },
      },
      {
        id: `project.stop.${p.id}`,
        label: `Stop ${p.name}`,
        detail: p.hostname,
        group: "Projects",
        icon: "square",
        keywords: [...baseKw, "halt", "kill"],
        run: async () => {
          try {
            await safeInvoke("stop_project", { id: p.id });
          } catch {
            /* toast pushed */
          }
        },
      },
      {
        id: `project.restart.${p.id}`,
        label: `Restart ${p.name}`,
        detail: p.hostname,
        group: "Projects",
        icon: "rotate-cw",
        keywords: [...baseKw, "reboot"],
        run: async () => {
          try {
            await safeInvoke("restart_project", { id: p.id });
          } catch {
            /* toast pushed */
          }
        },
      },
    );
    if (p.type === "php") {
      const xon = Boolean(
        p.env?.XDEBUG_MODE && p.env.XDEBUG_MODE !== "off",
      );
      cmds.push({
        id: `project.xdebug.${p.id}`,
        label: `${xon ? "Disable" : "Enable"} Xdebug for ${p.name}`,
        detail: p.hostname,
        group: "PHP",
        icon: "circle-alert",
        keywords: [...baseKw, "xdebug", "debug"],
        run: async () => {
          try {
            await safeInvoke("set_xdebug_mode", {
              id: p.id,
              mode: xon ? "off" : "develop,debug",
            });
            await projects.refresh();
          } catch {
            /* toast pushed */
          }
        },
      });
    }
  }

  // ────────── Groups ──────────
  for (const g of groups.value) {
    cmds.push(
      {
        id: `group.open.${g.id}`,
        label: `Open group ${g.name}`,
        detail: `${g.memberCount} member${g.memberCount === 1 ? "" : "s"}`,
        group: "Groups",
        icon: "folder",
        keywords: [g.id, "cluster"],
        run: () => void goto(`/groups/${g.id}`),
      },
      {
        id: `group.start.${g.id}`,
        label: `Start group ${g.name}`,
        detail: `${g.memberCount} member${g.memberCount === 1 ? "" : "s"}`,
        group: "Groups",
        icon: "play",
        keywords: [g.id, "start all"],
        run: async () => {
          try {
            await safeInvoke("start_group", { id: g.id });
          } catch {
            /* toast pushed */
          }
        },
      },
      {
        id: `group.stop.${g.id}`,
        label: `Stop group ${g.name}`,
        detail: `${g.memberCount} member${g.memberCount === 1 ? "" : "s"}`,
        group: "Groups",
        icon: "square",
        keywords: [g.id, "stop all"],
        run: async () => {
          try {
            await safeInvoke("stop_group", { id: g.id });
          } catch {
            /* toast pushed */
          }
        },
      },
      {
        id: `group.restart.${g.id}`,
        label: `Restart group ${g.name}`,
        detail: `${g.memberCount} member${g.memberCount === 1 ? "" : "s"}`,
        group: "Groups",
        icon: "rotate-cw",
        keywords: [g.id, "restart all"],
        run: async () => {
          try {
            await safeInvoke("restart_group", { id: g.id });
          } catch {
            /* toast pushed */
          }
        },
      },
    );
  }

  // ────────── Sidecars ──────────
  cmds.push(
    {
      id: "sidecar.restart-pc",
      label: "Restart process-compose",
      group: "Sidecars",
      icon: "refresh-cw",
      keywords: ["pc", "daemon"],
      run: async () => {
        try {
          await safeInvoke("restart_pc");
          await sidecars.refresh();
        } catch {
          /* toast pushed */
        }
      },
    },
    {
      id: "sidecar.restart-caddy",
      label: "Restart Caddy",
      group: "Sidecars",
      icon: "refresh-cw",
      keywords: ["proxy", "https"],
      run: async () => {
        try {
          await safeInvoke("restart_caddy");
          await sidecars.refresh();
        } catch {
          /* toast pushed */
        }
      },
    },
    {
      id: "sidecar.reconcile-hosts",
      label: "Reconcile /etc/hosts",
      group: "Sidecars",
      icon: "rotate-cw",
      keywords: ["hosts", "dns"],
      run: async () => {
        try {
          await safeInvoke<number>("reconcile_hosts");
          await sidecars.refresh();
        } catch {
          /* toast pushed */
        }
      },
    },
    {
      id: "sidecar.restart-dnsmasq",
      label: "Restart dnsmasq",
      group: "Sidecars",
      icon: "refresh-cw",
      keywords: ["dns", "wildcard"],
      run: async () => {
        try {
          await safeInvoke("restart_dnsmasq");
          await sidecars.refresh();
        } catch {
          /* toast pushed */
        }
      },
    },
    {
      id: "sidecar.refresh-status",
      label: "Refresh sidecar status",
      group: "Sidecars",
      icon: "refresh-cw",
      keywords: ["poll", "health"],
      run: () => void sidecars.refresh(),
    },
  );

  // ────────── Tunnels ──────────
  cmds.push({
    id: "tunnel.manage",
    label: "Manage public tunnels",
    detail:
      tunnels.count > 0
        ? `${tunnels.count} active`
        : "Share a project publicly",
    group: "Tunnels",
    icon: "cloud",
    keywords: ["cloudflare", "share", "public", "tunnel"],
    run: () => void goto("/tunnels"),
  });

  return cmds;
}

/** Wrap a command's `run` to also push a toast on uncaught errors —
 *  most commands already use safeInvoke so the toast is automatic, but
 *  navigation / store actions need a safety net. */
export async function executeCommand(cmd: PaletteCommand): Promise<void> {
  try {
    await Promise.resolve(cmd.run());
  } catch (e) {
    errorBus.push({
      code: "PALETTE_ACTION_FAILED",
      whatHappened: `Action "${cmd.label}" couldn't complete: ${String(e)}`,
      whyItMatters: "Re-run from the palette or use the in-app control.",
      whoCausedIt: "system",
      actions: [],
    });
  }
}
