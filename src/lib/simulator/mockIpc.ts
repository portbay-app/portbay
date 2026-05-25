/**
 * Mock Tauri IPC layer for the PortBay simulator + screenshot + e2e harness.
 *
 * The whole app talks to Rust through `window.__TAURI_INTERNALS__.invoke`
 * (see `$lib/ipc`). This installs a fake `__TAURI_INTERNALS__` that answers
 * from the dummy fixture roster and *simulates* lifecycle transitions (Play →
 * "Starting…" → "Running") by emitting `portbay://status` events through the
 * same `transformCallback` seam Tauri uses — so the real frontend runs
 * unmodified against canned data, with no backend.
 *
 * `installSimulatorIpcBrowser` is intentionally **self-contained** (no
 * references to module scope): it is serialized with `.toString()` and
 * re-evaluated in the page by Playwright's `addInitScript`, and is also called
 * directly by the web-simulator build. Pass the fixtures + options as its one
 * argument; never close over outer variables.
 */
import { DEMO_FIXTURES, type DemoFixtures } from "./fixtures";

export interface SimulatorOptions {
  /** How long `start_project` stays pending before resolving (ms). */
  startDelayMs?: number;
  /** Emit a `running` status event after a start (default true). */
  autoRunOnStart?: boolean;
  /** Delay before the simulated `running` event fires (ms, default 1200). */
  runDelayMs?: number;
}

/**
 * Install the mock onto `window.__TAURI_INTERNALS__`. Self-contained so it can
 * be injected via `page.addInitScript(installSimulatorIpcBrowser, payload)` or
 * called from the app bundle. Run before the SPA boots.
 */
export function installSimulatorIpcBrowser(payload: {
  fixtures: DemoFixtures;
  options?: SimulatorOptions;
}): void {
  /* eslint-disable @typescript-eslint/no-explicit-any */
  const fixtures = payload.fixtures;
  const opts = payload.options ?? {};
  const startDelayMs = opts.startDelayMs ?? 300;
  const runDelayMs = opts.runDelayMs ?? 1200;
  const autoRun = opts.autoRunOnStart !== false;

  const w = window as any;
  // Mutable working copy so lifecycle transitions persist across calls within
  // a session (and never mutate the shared fixture object).
  const projects: any[] = JSON.parse(JSON.stringify(fixtures.projects));

  // In-memory preferences for the demo so get/set round-trips — the Web Server
  // page's "Set as default" and Settings toggles reflect within the session.
  const prefs: Record<string, unknown> = {
    showTrayIcon: true,
    closeToMenuBar: true,
    closeToMenuBarToastSeen: true,
    telemetryEnabled: false,
    earlyAccessOptIn: false,
    launchAtLogin: false,
    reopenPreviousProjects: false,
    confirmBeforeStopAll: true,
    desktopNotifications: false,
    accentColor: "blue",
    defaultWorkspaceFolder: "",
    autoDetectProjects: false,
    defaultSort: "name-asc",
    defaultStartBehavior: "manual",
    defaultWebServer: null,
    manageHostsAutomatically: true,
    autoRenewCertificates: true,
    storeLogsLocally: true,
    logRetentionDays: 7,
    cliPath: "/usr/local/bin/portbay",
    autoCleanSchedule: "off",
    lastAutoClean: 0,
    autoCleanExtraDirs: [],
  };

  let nextCb = 1;
  let nextListenerId = 1;
  let nextEventId = 1;
  const listeners: Array<{ event: string; cbId: number }> = [];

  function emit(event: string, eventPayload: unknown): void {
    for (const l of listeners) {
      if (l.event !== event) continue;
      const cb = w["_" + l.cbId];
      if (typeof cb === "function") {
        cb({ event, id: nextEventId++, payload: eventPayload });
      }
    }
  }

  function project(id: unknown): any {
    return projects.find((p) => p.id === id);
  }

  function runtimeFor(p: any): unknown {
    if (p && p.runtime) return p.runtime;
    return {
      pid: 40000 + Math.floor(Math.random() * 9999),
      restarts: 0,
      isReady: "true",
      hasReadyProbe: true,
      exitCode: 0,
      age: 1_000_000_000,
      memBytes: 80 * 1024 * 1024,
      cpuPercent: 0.6,
    };
  }

  function startProject(id: unknown): Promise<unknown> {
    const p = project(id);
    if (p) p.status = "starting";
    if (autoRun && p) {
      setTimeout(() => {
        p.status = "running";
        p.runtime = runtimeFor(p);
        emit("portbay://status", {
          id: p.id,
          status: "running",
          runtime: p.runtime,
          ts: Date.now(),
        });
      }, runDelayMs);
    }
    // Resolve the invoke itself after startDelayMs — deliberately slow for the
    // latency guard, snappy for the demo.
    return new Promise((resolve) => setTimeout(() => resolve(null), startDelayMs));
  }

  function stopProject(id: unknown): Promise<unknown> {
    const p = project(id);
    if (p) {
      p.status = "stopped";
      delete p.runtime;
      emit("portbay://status", { id: p.id, status: "stopped", ts: Date.now() });
    }
    return Promise.resolve(null);
  }

  function invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
    switch (cmd) {
      case "list_projects":
        return Promise.resolve(projects);
      case "list_groups":
        return Promise.resolve(fixtures.groups);
      case "sidecar_status":
        return Promise.resolve(fixtures.sidecars);
      case "webserver_overview":
        return Promise.resolve(fixtures.webServers);
      case "get_preferences":
        return Promise.resolve({ ...prefs });
      case "set_preferences": {
        const next = args && (args.prefs as Record<string, unknown>);
        if (next) Object.assign(prefs, next);
        return Promise.resolve({ ...prefs });
      }
      case "mark_close_toast_seen":
        prefs.closeToMenuBarToastSeen = true;
        return Promise.resolve(null);
      case "get_entitlement":
      case "refresh_entitlement":
        return Promise.resolve(fixtures.entitlement);
      case "recent_requests":
        return Promise.resolve(fixtures.requests);
      case "list_runtimes":
        return Promise.resolve(fixtures.runtimes);
      case "list_database_engines":
        return Promise.resolve(fixtures.databaseEngines);
      case "list_database_instances":
        return Promise.resolve(fixtures.databaseInstances);
      case "list_dns_records":
        return Promise.resolve(fixtures.dnsRecords);
      case "dns_preflight":
        return Promise.resolve(fixtures.dnsPreflight);
      case "dnsmasq_resolver_status":
        return Promise.resolve(fixtures.resolverStatus);
      case "get_dnsmasq_settings":
        return Promise.resolve({
          cacheSize: 150,
          localTtl: 0,
          disableNegativeCache: false,
        });
      case "list_managed_hosts":
        return Promise.resolve(
          fixtures.dnsRecords
            .filter((r) => r.kind === "project")
            .map((r) => ({ ip: "127.0.0.1", hostname: r.hostname })),
        );
      case "installed_dev_tools":
        return Promise.resolve([]);
      case "onboarding_status":
        return Promise.resolve({ onboarded: true, seenCloseToast: true });
      case "start_project":
      case "force_start_project":
        return startProject(args && args.id);
      case "stop_project":
        return stopProject(args && args.id);
      case "restart_project":
        return startProject(args && args.id);
      case "stop_all":
        for (const p of projects) {
          if (p.status === "running" || p.status === "starting") {
            p.status = "stopped";
            delete p.runtime;
            emit("portbay://status", { id: p.id, status: "stopped", ts: Date.now() });
          }
        }
        return Promise.resolve(null);
      case "plugin:event|listen": {
        const event = args && (args.event as string);
        const handler = args && (args.handler as number);
        if (typeof event === "string" && typeof handler === "number") {
          listeners.push({ event, cbId: handler });
        }
        return Promise.resolve(nextListenerId++);
      }
      case "plugin:event|unlisten":
        return Promise.resolve(null);
      default:
        // Unknown list_* commands get an empty array (stores expecting arrays
        // don't throw on boot); everything else resolves null.
        return Promise.resolve(cmd.indexOf("list_") === 0 ? [] : null);
    }
  }

  w.__TAURI_INTERNALS__ = {
    // Window/webview metadata is read synchronously on mount by some
    // components (e.g. the wizard's drag-drop listener); without it the app
    // throws during boot.
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
    transformCallback(cb: (p: unknown) => void, once: boolean) {
      const cid = nextCb++;
      w["_" + cid] = (p: unknown) => {
        if (once) delete w["_" + cid];
        return cb ? cb(p) : undefined;
      };
      return cid;
    },
    invoke,
  };
  /* eslint-enable @typescript-eslint/no-explicit-any */
}

/**
 * Convenience for the web-simulator build: install the mock with the bundled
 * canonical roster. The desktop build never imports this (tree-shaken behind
 * the `PUBLIC_SIMULATOR` flag).
 */
export function installSimulator(options?: SimulatorOptions): void {
  installSimulatorIpcBrowser({ fixtures: DEMO_FIXTURES, options });
}
