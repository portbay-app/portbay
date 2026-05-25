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
  /**
   * Commands that should reject with a `CommandError`-shaped payload instead of
   * resolving. Lets the e2e harness exercise the error-toast path (e.g.
   * `failCommands: ["stop_project"]`). Empty/unset in the screenshot + web
   * simulator builds, so their behaviour is unchanged.
   */
  failCommands?: string[];
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
  const failCommands = opts.failCommands ?? [];

  const w = window as any;
  // Mutable working copy so lifecycle transitions persist across calls within
  // a session (and never mutate the shared fixture object).
  const projects: any[] = JSON.parse(JSON.stringify(fixtures.projects));
  let metricsSnapshot: any = JSON.parse(JSON.stringify(fixtures.metrics));
  let metricsTick = 0;

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

  function clamp(n: number, min: number, max: number): number {
    return Math.min(max, Math.max(min, n));
  }

  // Synthesize a `cert_info` payload for the SSL Certificates page. Every demo
  // HTTPS project gets a freshly-minted, long-lived mkcert cert so the table
  // renders a realistic validity window without touching the filesystem.
  function certInfoFor(p: any): unknown {
    const now = Date.now();
    const day = 86_400_000;
    const issued = new Date(now - 12 * day);
    const expires = new Date(now + 808 * day);
    return {
      projectId: p.id,
      certificatePath: `/Users/you/.portbay/certs/${p.id}/cert.pem`,
      keyPath: `/Users/you/.portbay/certs/${p.id}/key.pem`,
      issuedAt: issued.toISOString(),
      expiresAt: expires.toISOString(),
      daysUntilExpiry: Math.round((expires.getTime() - now) / day),
      sans: [p.hostname],
    };
  }

  function nextMetrics(): unknown {
    const base = fixtures.metrics as any;
    metricsTick += 1;
    const tick = metricsTick;
    const cpuTotal = clamp(
      base.cpu.total + Math.sin(tick / 2) * 7 + Math.sin(tick / 5) * 3,
      4,
      92,
    );
    const memoryDrift = Math.sin(tick / 7) * 0.35 * 1024 ** 3;
    const diskDrift = Math.sin(tick / 23) * 0.08 * 1024 ** 3;

    return {
      cpu: { total: Number(cpuTotal.toFixed(1)) },
      memory: {
        usedBytes: Math.round(
          clamp(
            base.memory.usedBytes + memoryDrift,
            0,
            base.memory.totalBytes,
          ),
        ),
        totalBytes: base.memory.totalBytes,
      },
      disk: {
        usedBytes: Math.round(
          clamp(base.disk.usedBytes + diskDrift, 0, base.disk.totalBytes),
        ),
        totalBytes: base.disk.totalBytes,
      },
    };
  }

  if (w.__PORTBAY_SIM_METRICS_TIMER__) {
    clearInterval(w.__PORTBAY_SIM_METRICS_TIMER__);
  }
  w.__PORTBAY_SIM_METRICS_TIMER__ = setInterval(() => {
    metricsSnapshot = nextMetrics();
    emit("portbay://metrics", metricsSnapshot);
  }, 1000);

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

  // Synthesize the sandbox's blocked-connection log for the Sandbox page. The
  // tighter the policy, the more a typical app trips against it — so a
  // loopback-only project shows real denials while a "full" one shows none.
  function sandboxViolationsFor(id: unknown): string[] {
    const p = project(id);
    if (!p || !p.sandboxed) return [];
    const net = (p.sandbox && p.sandbox.network) || "loopback_only";
    if (net === "full") return [];
    const t = (h: number, m: number, s: number) =>
      `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    const lines = [
      `[${t(14, 2, 11)}] DENY outbound tcp → 140.82.121.4:443 (api.github.com)`,
      `[${t(14, 2, 11)}] DENY outbound tcp → 104.16.85.20:443 (registry.npmjs.org)`,
      `[${t(14, 3, 38)}] DENY dns query → telemetry.example.com`,
    ];
    if (net === "loopback_only" || net === "blocked") {
      lines.push(`[${t(14, 5, 2)}] DENY connect → 192.168.1.24:5432 (LAN)`);
    }
    if (net === "blocked") {
      lines.push(`[${t(14, 5, 9)}] DENY connect → 127.0.0.1:6060 (loopback)`);
    }
    return lines;
  }

  function logsFor(id: unknown, limit?: unknown): string[] {
    const key = typeof id === "string" ? id : "";
    const lines = (fixtures.logs && fixtures.logs[key]) || [];
    const max = typeof limit === "number" && limit > 0 ? limit : 200;
    return lines.slice(Math.max(0, lines.length - max));
  }

  function pushChannelMessage(channel: unknown, index: number, message: string): void {
    const ch = channel as { id?: number } | undefined;
    if (!ch || typeof ch.id !== "number") return;
    const cb = w["_" + ch.id];
    if (typeof cb === "function") cb({ index, message });
  }

  function subscribeLogs(args?: Record<string, unknown>): Promise<unknown> {
    const id = args && args.id;
    const channel = args && args.onLine;
    const seed = logsFor(id, 8);
    let index = 0;
    const timer = setInterval(() => {
      const p = project(id);
      const projectName = p && typeof p.name === "string" ? p.name : "Project";
      const messages = [
        `GET ${p?.url ?? "https://project.test"} 200 ${24 + index * 3}ms`,
        `hmr update ${projectName.replaceAll(" ", "-").toLowerCase()}/src/App.svelte`,
        `PortBay health check passed for ${projectName}`,
        `TLS certificate valid for ${p?.hostname ?? "project.test"}`,
      ];
      const message = seed[index] ?? JSON.stringify({
        level: "info",
        process: "simulator",
        replica: 0,
        message: messages[index % messages.length],
      });
      pushChannelMessage(channel, index, message);
      index += 1;
      if (index >= seed.length + 4) clearInterval(timer);
    }, 900);
    return Promise.resolve(null);
  }

  function invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
    // Injected-failure hook for the e2e error-path test (no-op unless the
    // caller opted a command into `failCommands`).
    if (failCommands.indexOf(cmd) !== -1) {
      return Promise.reject({
        code: "SIMULATED_FAILURE",
        whatHappened: `The ${cmd} command failed (simulated).`,
        whyItMatters: "Injected failure for the e2e error-path test.",
        whoCausedIt: "system",
        actions: [],
      });
    }
    switch (cmd) {
      case "list_projects":
        return Promise.resolve(projects);
      case "list_groups":
        return Promise.resolve(fixtures.groups);
      case "sidecar_status":
        return Promise.resolve(fixtures.sidecars);
      case "webserver_overview":
        return Promise.resolve(fixtures.webServers);
      case "installed_dev_tools":
        return Promise.resolve(fixtures.devTools);
      case "cert_info": {
        const p = project(args && args.id);
        if (!p || !p.https) {
          return Promise.reject({
            code: "PROJECT_NOT_FOUND",
            whatHappened: "No certificate issued for this project yet.",
            whyItMatters: "The reconciler mints the cert on first reconcile.",
            whoCausedIt: "system",
            actions: [],
          });
        }
        return Promise.resolve(certInfoFor(p));
      }
      case "reissue_cert":
        // No-op in the demo — `cert_info` synthesizes fresh metadata on the
        // next read, so the row repaints as if reissued.
        return Promise.resolve(null);
      case "system_metrics":
        metricsSnapshot = nextMetrics();
        return Promise.resolve(metricsSnapshot);
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
      case "account_resync":
        return Promise.resolve(fixtures.entitlement);
      case "logout":
        return Promise.resolve(fixtures.entitlement);
      case "begin_login":
        return Promise.resolve({ authorize_url: null });
      case "poll_login":
        return Promise.resolve({
          status: "ready",
          entitlement: fixtures.entitlement,
        });
      case "cancel_login":
        return Promise.resolve(null);
      case "recent_requests":
        return Promise.resolve(fixtures.requests);
      case "tail_logs":
        return Promise.resolve(logsFor(args && args.id, args && args.limit));
      case "subscribe_logs":
        return subscribeLogs(args);
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
      case "get_domain_settings":
        return Promise.resolve({
          domainSuffix: "test",
          projectCount: projects.length,
        });
      case "update_domain_suffix":
        return Promise.resolve({
          oldSuffix: "test",
          newSuffix: args && (args.domainSuffix as string),
          changedProjects: projects.length,
        });
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
      case "telemetry_settings":
        return Promise.resolve({
          enabled: false,
          crashReportCount: 0,
          endpointConfigured: false,
        });
      case "list_crash_reports":
        return Promise.resolve([]);
      case "send_crash_report":
      case "discard_crash_report":
      case "reset_onboarding":
      case "dnsmasq_install_resolver":
      case "dnsmasq_uninstall_resolver":
        return Promise.resolve(null);
      case "onboarding_status":
        return Promise.resolve({ onboarded: true, seenCloseToast: true });
      case "start_project":
      case "force_start_project":
        return startProject(args && args.id);
      case "open_project":
      case "open_in_ide":
        return Promise.resolve(null);
      case "detect_project": {
        // Framework auto-detection for the Add-project wizard. Derives a
        // plausible id/name/host from the folder's last path segment so the
        // wizard's L2 fields fill in just like the real backend.
        const p = String((args && args.path) || "");
        const seg = p.split("/").filter(Boolean).pop() || "project";
        const slug =
          seg.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") ||
          "project";
        const title = seg
          .replace(/[-_]+/g, " ")
          .replace(/\b\w/g, (c) => c.toUpperCase());
        return Promise.resolve({
          kind: "node",
          suggestedId: slug,
          suggestedName: title,
          suggestedHostname: `${slug}.test`,
          suggestedPort: 3000,
          suggestedStartCommand: "npm run dev",
        });
      }
      case "add_project": {
        // Register a new project from the wizard's input and return it; the
        // wizard then calls list_projects (projects.refresh) and the new row
        // appears. Mirrors the backend's id-derivation + url-from-hostname.
        const input = (args && (args.input as Record<string, unknown>)) || {};
        const p = String(input.path || "");
        const seg = p.split("/").filter(Boolean).pop() || "new-project";
        const slug =
          seg.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") ||
          "new-project";
        const id = String(input.id || slug);
        const hostname = String(input.hostname || `${id}.test`);
        const https = input.https !== false;
        const created: any = {
          id,
          name: String(input.name || seg),
          path: p,
          type: input.kind || "custom",
          startCommand: input.startCommand || undefined,
          port: input.port ?? null,
          extraPorts: [],
          hostname,
          url: `${https ? "https" : "http"}://${hostname}`,
          https,
          services: [],
          env: {},
          autoStart: input.autoStart === true,
          tags: [],
          sandboxed: false,
          status: "stopped",
        };
        if (!project(id)) projects.push(created);
        return Promise.resolve(created);
      }
      case "update_project": {
        const p = project(args && args.id);
        const patch = (args && (args.patch as Record<string, unknown>)) || {};
        if (p) {
          if (patch.hostname !== undefined) p.hostname = patch.hostname;
          if (patch.port !== undefined) p.port = patch.port;
          if (patch.https !== undefined) p.https = patch.https;
          if (patch.autoStart !== undefined) p.autoStart = patch.autoStart;
          if (patch.name !== undefined) p.name = patch.name;
          if (patch.domain !== undefined) {
            const d = patch.domain as Record<string, unknown> | null;
            // Normalise an all-default config to null, mirroring the core.
            const isDefault =
              !!d &&
              !d.notes &&
              !d.pathPrefix &&
              d.resolverMode === "auto" &&
              d.autoManageCert === true &&
              !d.includeWildcardSubdomains &&
              !d.exposeWhenRunning;
            p.domain = d && !isDefault ? d : null;
          }
          p.url = `${p.https ? "https" : "http"}://${p.hostname}`;
        }
        return Promise.resolve(p ?? null);
      }
      case "remove_project": {
        const idx = projects.findIndex((p) => p.id === (args && args.id));
        if (idx >= 0) projects.splice(idx, 1);
        return Promise.resolve(null);
      }
      case "stop_project":
        return stopProject(args && args.id);
      case "restart_project":
        return startProject(args && args.id);
      case "start_project_sandboxed":
        return startProject(args && args.id);
      case "promote_project_to_local": {
        const p = project(args && args.id);
        if (p) {
          p.sandboxed = false;
          delete p.sandbox;
        }
        return Promise.resolve(null);
      }
      case "sandbox_violations":
        return Promise.resolve(sandboxViolationsFor(args && args.id));
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
