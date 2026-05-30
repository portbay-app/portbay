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
  let metricsSnapshot: any = JSON.parse(JSON.stringify(fixtures.metrics));
  let metricsTick = 0;

  // In-memory preferences for the demo so get/set round-trips — the Web Server
  // page's "Set as default" and Settings toggles reflect within the session.
  const prefs: Record<string, unknown> = {
    showTrayIcon: true,
    showDockIcon: true,
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

  // Mutable task-board state so create / move / edit / check persist across
  // calls within a session (deep-cloned so the shared fixture is never touched).
  const boardCards: Record<string, any[]> = JSON.parse(
    JSON.stringify(fixtures.tasks ?? {}),
  );
  const boardConfigs: Record<string, any> = JSON.parse(
    JSON.stringify(fixtures.boardConfigs ?? {}),
  );
  const handoffs: Record<string, any> = JSON.parse(
    JSON.stringify(fixtures.handoffs ?? {}),
  );
  const scratchpads: Record<string, string> = JSON.parse(
    JSON.stringify(fixtures.scratchpads ?? {}),
  );
  const cardActivity: Record<string, any[]> = JSON.parse(
    JSON.stringify(fixtures.cardActivity ?? {}),
  );
  const runLogs: Record<string, any> = JSON.parse(
    JSON.stringify(fixtures.runLogs ?? {}),
  );
  // Mutable agent roster so Settings → AI Integrations toggles (launch mode,
  // path override) round-trip within the session.
  const agents: any[] = JSON.parse(JSON.stringify(fixtures.agentOptions ?? []));
  let taskIdSeq = 1;

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

  /* ----------------------------------------------------------------- tasks */

  function nowIso(): string {
    return new Date(Date.now()).toISOString();
  }
  function newCardId(): string {
    return "demo-card-" + taskIdSeq++;
  }
  function boardOf(pid: unknown): any[] {
    const key = typeof pid === "string" ? pid : "";
    if (!boardCards[key]) boardCards[key] = [];
    return boardCards[key];
  }
  function findCard(pid: unknown, id: unknown): any {
    return boardOf(pid).find((c) => c.id === id);
  }
  function maxOrder(list: any[]): number {
    return list.reduce((m, c) => Math.max(m, c.order ?? 0), 0);
  }
  function emitTasksChanged(pid: unknown): void {
    if (typeof pid === "string") emit("tasks://changed", { projectId: pid });
  }
  // A template's string checklist → the wire Checklist object.
  function templateChecklist(t: any): any {
    if (!t || !Array.isArray(t.checklist) || t.checklist.length === 0) return null;
    return {
      label: "Steps",
      items: t.checklist.map((desc: string, idx: number) => ({ idx, desc, done: false })),
    };
  }
  // Begin a (simulated) agent run on a card: claim it, move to In Progress, and
  // seed a short transcript. Used by manual "Start with agent" and the
  // auto-dispatch a To-Do move triggers on an `autoOnTodo` board.
  function beginRun(pid: unknown, card: any, agent: string): void {
    card.status = "InProgress";
    card.claim = { host: "studio.local", runId: "run_" + agent + "_" + taskIdSeq++, at: nowIso() };
    card.updated = nowIso();
    runLogs[card.id] = {
      runId: card.claim.runId,
      agent,
      running: true,
      text:
        "● Claimed card · run " + card.claim.runId + "\n" +
        "✓ Loaded project context (CLAUDE.md, HANDOFF.md)\n" +
        "▸ Working…",
    };
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
      // Avatar fetch is backend-only (network + disk cache); the hosted demo
      // has neither, so the chip falls back to the account's initials.
      case "get_account_avatar":
        return Promise.resolve(null);
      // Profile edits are no-ops in the hosted demo — echo the fixture entitlement.
      case "update_display_name":
      case "upload_avatar":
      case "remove_avatar":
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
      // Telemetry is opt-out by default and the hosted demo has no backend
      // sink, so usage events / JS-error capture are accepted and dropped.
      case "record_js_error":
        return Promise.resolve("demo-crash-0");
      case "send_crash_report":
      case "discard_crash_report":
      case "record_telemetry_event":
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
      // Icon detection is filesystem-only; the hosted demo has no project
      // tree, so avatars fall back to their stack glyph.
      case "project_icon":
        return Promise.resolve(null);
      case "update_project": {
        const p = project(args && args.id);
        const patch = (args && (args.patch as Record<string, unknown>)) || {};
        if (p) {
          if (patch.hostname !== undefined) p.hostname = patch.hostname;
          if (patch.port !== undefined) p.port = patch.port;
          if (patch.https !== undefined) p.https = patch.https;
          if (patch.autoStart !== undefined) p.autoStart = patch.autoStart;
          if (patch.name !== undefined) p.name = patch.name;
          if (patch.kind !== undefined) p.type = patch.kind;
          if (patch.startCommand !== undefined)
            p.startCommand = patch.startCommand ?? undefined;
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
      // Settings → Sync: the demo entitlement is Pro, so SyncSection calls
      // `sync_state` on mount. The Rust command always returns a valid DTO;
      // without this the default (null) crashes the Settings render.
      case "sync_state":
        return Promise.resolve({
          signed_in: false,
          is_pro: true,
          enabled: false,
          last_version: 0,
        });
      // Settings → Migration: ImportSection iterates the result, so it must be
      // an array rather than the default null.
      case "detect_sources":
        return Promise.resolve([]);
      // Agent-activity notifications: the hosted demo has no audit logs, so the
      // bell starts empty. List must be an array (the store reduces over it).
      case "notifications_list":
        return Promise.resolve([]);
      case "notifications_mark_read":
      case "notifications_mark_all_read":
      case "notifications_clear":
        return Promise.resolve(null);

      // --- Task board ("Project Context & Task Authority") ------------------
      case "tasks_list":
        return Promise.resolve(boardOf(args && args.projectId));
      case "board_config_get": {
        const pid = typeof (args && args.projectId) === "string" ? (args!.projectId as string) : "";
        return Promise.resolve(
          boardConfigs[pid] ?? JSON.parse(JSON.stringify(fixtures.defaultBoardConfig)),
        );
      }
      case "board_config_set": {
        const pid = args && (args.projectId as string);
        if (pid && args && args.config) boardConfigs[pid] = args.config;
        return Promise.resolve((pid && boardConfigs[pid]) || (args && args.config) || null);
      }
      case "handoff_show": {
        const pid = args && (args.projectId as string);
        return Promise.resolve(
          (pid && handoffs[pid]) || {
            exists: false,
            updated: null,
            maxChars: 1200,
            chars: 0,
            autoGenerated: false,
            body: "",
          },
        );
      }
      case "handoff_update":
      case "handoff_replace": {
        const pid = args && (args.projectId as string);
        const incoming = (args && ((args.body as string) ?? (args.narrative as string))) || "";
        const prev = (pid && handoffs[pid] && handoffs[pid].body) || "";
        const nextBody =
          cmd === "handoff_replace"
            ? incoming
            : incoming
              ? incoming + "\n\n" + prev
              : prev;
        const view = {
          exists: nextBody.length > 0,
          updated: nowIso(),
          maxChars: 1200,
          chars: nextBody.length,
          autoGenerated: false,
          body: nextBody,
        };
        if (pid) handoffs[pid] = view;
        return Promise.resolve(view);
      }
      case "scratchpad_get":
        return Promise.resolve(
          (args && scratchpads[args.projectId as string]) || "",
        );
      case "scratchpad_set": {
        const pid = args && (args.projectId as string);
        if (pid) scratchpads[pid] = (args && (args.body as string)) || "";
        return Promise.resolve(null);
      }
      case "agents_installed":
        return Promise.resolve(agents);
      // Settings → AI Integrations: flip an agent's CLI/Desktop launch form.
      case "set_agent_launch_mode": {
        const a = agents.find((x) => x.id === (args && args.agent));
        if (a && (args!.mode === "cli" || args!.mode === "app")) a.mode = args!.mode;
        return Promise.resolve(agents);
      }
      // Settings → AI Integrations: set a manual binary path override.
      case "set_agent_path": {
        const a = agents.find((x) => x.id === (args && args.agent));
        if (a) {
          a.path = (args && (args.path as string)) || a.path;
          a.overridden = true;
          a.cliInstalled = true;
          a.installed = true;
        }
        return Promise.resolve(agents);
      }
      // Settings → AI Integrations: clear a path override (fall back to PATH).
      case "clear_agent_path": {
        const a = agents.find((x) => x.id === (args && args.agent));
        if (a) a.overridden = false;
        return Promise.resolve(agents);
      }
      // Settings → AI Integrations (MCP): the resolved PortBay MCP binary.
      case "resolve_mcp_binary_path":
        return Promise.resolve("/usr/local/bin/portbay");
      case "board_templates":
        return Promise.resolve(fixtures.boardTemplates ?? []);
      case "tasks_watch":
      case "tasks_unwatch":
      case "board_reconcile":
        return Promise.resolve(null);
      case "card_activity":
        return Promise.resolve((args && cardActivity[args.id as string]) || []);
      case "task_run_log":
        return Promise.resolve(
          (args && runLogs[args.id as string]) || {
            runId: null,
            agent: null,
            running: false,
            text: "",
          },
        );
      case "task_create": {
        const pid = args && args.projectId;
        const input: any = (args && args.input) || {};
        const tmpl = input.template
          ? (fixtures.boardTemplates ?? []).find(
              (t) => t.name.toLowerCase() === String(input.template).toLowerCase(),
            )
          : null;
        const list = boardOf(pid);
        const c = {
          id: newCardId(),
          title: input.title || "Untitled task",
          status: input.status || "Backlog",
          priority: input.priority ?? (tmpl ? tmpl.priority : null) ?? null,
          labels: input.labels ?? (tmpl ? tmpl.labels : []) ?? [],
          estimate: input.estimate ?? null,
          color: input.color ?? null,
          url: input.url ?? null,
          checklist: input.checklist ?? (tmpl ? templateChecklist(tmpl) : null) ?? null,
          acceptance: input.acceptance ?? (tmpl ? tmpl.acceptance : null) ?? null,
          touchpoints: input.touchpoints ?? [],
          automation: input.automation || "inherit",
          agent: input.agent ?? null,
          order: maxOrder(list) + 1000,
          created: nowIso(),
          updated: nowIso(),
          schemaVersion: 1,
          body: input.body ?? (tmpl ? tmpl.body : "") ?? "",
        };
        list.push(c);
        emitTasksChanged(pid);
        return Promise.resolve(c);
      }
      case "task_update": {
        const pid = args && args.projectId;
        const input: any = (args && args.input) || {};
        const c = findCard(pid, input.id);
        if (!c) return Promise.resolve(null);
        const fields = [
          "title", "body", "priority", "due", "acceptance", "touchpoints",
          "blockedBy", "automation", "order", "labels", "estimate", "color",
          "url", "checklist", "custom", "subscribed", "archived", "autoBranch",
          "autoArchive",
        ];
        for (const f of fields) if (input[f] !== undefined) c[f] = input[f];
        // agent / assignee / icon: "" clears (inherit/none), a value sets.
        for (const f of ["agent", "assignee", "icon"]) {
          if (input[f] !== undefined) c[f] = input[f] === "" ? null : input[f];
        }
        c.updated = nowIso();
        emitTasksChanged(pid);
        return Promise.resolve(c);
      }
      case "task_move": {
        const pid = args && args.projectId;
        const c = findCard(pid, args && args.id);
        if (!c) return Promise.resolve(null);
        c.status = args && args.to;
        if (args && args.order != null) c.order = args.order;
        c.updated = nowIso();
        // autoOnTodo: entering To Do auto-dispatches the card's agent. Simulate
        // the run after a short beat so the move reads, then the card "starts".
        const cfg = pid ? boardConfigs[pid as string] : null;
        const eligible =
          c.automation !== "off" && (c.automation === "on" || c.agent != null);
        if (
          args && args.to === "Todo" &&
          cfg && cfg.automation && cfg.automation.mode === "autoOnTodo" &&
          eligible
        ) {
          const agent = c.agent || cfg.automation.agent || "claude";
          const cardId = c.id;
          setTimeout(() => {
            const card = findCard(pid, cardId);
            if (!card || card.status !== "Todo") return; // moved on meanwhile
            beginRun(pid, card, agent);
            emitTasksChanged(pid);
          }, 1200);
        }
        emitTasksChanged(pid);
        return Promise.resolve(null);
      }
      case "task_reorder": {
        const c = findCard(args && args.projectId, args && args.id);
        if (c && args && args.order != null) {
          c.order = args.order;
          c.updated = nowIso();
        }
        return Promise.resolve(null);
      }
      case "task_delete": {
        const list = boardOf(args && args.projectId);
        const idx = list.findIndex((c) => c.id === (args && args.id));
        if (idx >= 0) list.splice(idx, 1);
        emitTasksChanged(args && args.projectId);
        return Promise.resolve(null);
      }
      case "task_check_item": {
        const c = findCard(args && args.projectId, args && args.id);
        if (c && c.checklist && Array.isArray(c.checklist.items)) {
          const it = c.checklist.items.find((i: any) => i.idx === (args && args.idx));
          if (it) it.done = !!(args && args.done);
          c.updated = nowIso();
        }
        return Promise.resolve(null);
      }
      case "task_checklist_add": {
        const c = findCard(args && args.projectId, args && args.id);
        if (c) {
          const items =
            c.checklist && Array.isArray(c.checklist.items)
              ? c.checklist.items.slice()
              : [];
          let nextIdx = items.reduce((m: number, i: any) => Math.max(m, i.idx + 1), 0);
          for (const desc of (args && (args.items as string[])) || [])
            items.push({ idx: nextIdx++, desc, done: false });
          c.checklist = {
            label: (c.checklist && c.checklist.label) || (args && args.label) || "Steps",
            items,
          };
          c.updated = nowIso();
        }
        emitTasksChanged(args && args.projectId);
        return Promise.resolve(null);
      }
      case "task_archive": {
        const c = findCard(args && args.projectId, args && args.id);
        if (c) {
          c.archived = !!(args && args.archived);
          c.updated = nowIso();
        }
        emitTasksChanged(args && args.projectId);
        return Promise.resolve(null);
      }
      case "task_subscribe": {
        const c = findCard(args && args.projectId, args && args.id);
        if (c) c.subscribed = !!(args && args.subscribed);
        emitTasksChanged(args && args.projectId);
        return Promise.resolve(null);
      }
      case "task_capture": {
        const pid = args && args.projectId;
        const list = boardOf(pid);
        const c = {
          id: newCardId(),
          title: (args && (args.title as string)) || "Untitled",
          status: "Backlog",
          draft: true,
          automation: "inherit",
          order: maxOrder(list) + 1000,
          created: nowIso(),
          updated: nowIso(),
          schemaVersion: 1,
          body: "",
        };
        list.push(c);
        emitTasksChanged(pid);
        return Promise.resolve(c);
      }
      case "task_promote": {
        const c = findCard(args && args.projectId, args && args.id);
        if (c) {
          c.draft = false;
          c.status = "Backlog";
          c.updated = nowIso();
        }
        emitTasksChanged(args && args.projectId);
        return Promise.resolve(null);
      }
      case "task_duplicate": {
        const pid = args && args.projectId;
        const src = findCard(pid, args && args.id);
        if (!src) return Promise.resolve(null);
        const list = boardOf(pid);
        const copy = JSON.parse(JSON.stringify(src));
        copy.id = newCardId();
        copy.title = src.title + " (copy)";
        copy.status = "Backlog";
        copy.claim = null;
        copy.order = maxOrder(list) + 1000;
        copy.created = nowIso();
        copy.updated = nowIso();
        list.push(copy);
        emitTasksChanged(pid);
        return Promise.resolve(copy);
      }
      case "task_start_with_agent": {
        const pid = args && args.projectId;
        const c = findCard(pid, args && args.id);
        if (c) {
          const cfg = pid ? boardConfigs[pid as string] : null;
          const agent =
            c.agent || (cfg && cfg.automation && cfg.automation.agent) || "claude";
          beginRun(pid, c, agent);
        }
        emitTasksChanged(pid);
        return Promise.resolve(null);
      }
      case "task_stop_agent": {
        const c = findCard(args && args.projectId, args && args.id);
        if (c) {
          c.claim = null;
          c.updated = nowIso();
          const rl = runLogs[c.id];
          if (rl) rl.running = false;
        }
        emitTasksChanged(args && args.projectId);
        return Promise.resolve(null);
      }
      case "task_branch":
        return Promise.resolve({
          branch: "task/" + ((args && (args.id as string)) || "card"),
          created: true,
        });
      case "task_comment": {
        const id = args && (args.id as string);
        if (id) {
          const list = cardActivity[id] || (cardActivity[id] = []);
          list.unshift({
            at: nowIso(),
            cardId: id,
            action: "comment",
            actor: { kind: "human" },
            note: (args && (args.text as string)) || "",
          });
        }
        return Promise.resolve(null);
      }
      case "task_attach":
      case "task_detach":
        return Promise.resolve(null);
      case "task_attachment_path":
        return Promise.resolve("/Users/dev/Sites/demo/.portbay/attachments/file");
      case "context_sync":
        return Promise.resolve({
          projectId: args && args.projectId,
          dryRun: !!(args && args.dryRun),
          results: [],
        });

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
