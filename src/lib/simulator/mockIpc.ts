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
    notifications: {
      schemaVersion: 1,
      channels: {
        lifecycle: { toast: false, bell: true, banner: false, sound: false },
        "project-error": { toast: true, bell: true, banner: false, sound: true },
        "agent-board": { toast: false, bell: true, banner: false, sound: true },
        updates: { toast: false, bell: true, banner: false, sound: false },
        crash: { toast: true, bell: true, banner: false, sound: true },
        infrastructure: { toast: false, bell: true, banner: false, sound: false },
        "account-sync": { toast: false, bell: true, banner: false, sound: false },
      },
      severityFloor: "everything",
      quietHours: { enabled: false, start: "22:00", end: "07:00", exemptErrors: true },
      snoozeUntil: null,
      sound: {
        volumeFollowsOs: true,
        cuePerCategory: {
          lifecycle: "done",
          "project-error": "error",
          "agent-board": "comment",
          updates: "done",
          crash: "error",
          infrastructure: "attention",
          "account-sync": "comment",
        },
      },
    },
    accessibility: {
      reduceMotion: false,
      reduceTransparency: false,
      highContrast: false,
      textScale: "normal",
      focusMode: "standard",
      underlineLinks: false,
      colorIndependentStatus: false,
    },
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

  // Mutable SSH state so save/delete/start/stop and file edits round-trip within
  // a session (deep-cloned so the shared fixture is never touched).
  const sshConnections: any[] = JSON.parse(
    JSON.stringify(fixtures.sshConnections ?? []),
  );
  const sshIdentities: any[] = JSON.parse(
    JSON.stringify(fixtures.sshIdentities ?? []),
  );
  const sshTunnels: any[] = JSON.parse(JSON.stringify(fixtures.sshTunnels ?? []));
  // In-session SFTP edits (sftp_write_text), keyed `${connectionId}::${path}`.
  const sshFileEdits: Record<string, string> = {};
  // Live demo PTY sessions, keyed by the id ssh_pty_open returns.
  const ptys: Record<string, any> = {};
  let ptySeq = 1;
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

  function demoDbSchema(id: unknown): any {
    const sqlite = id === "quill-sqlite";
    if (sqlite) {
      return {
        engine: "sqlite",
        schemas: [],
        tables: [
          {
            schema: null,
            name: "documents",
            columns: [
              { name: "id", dataType: "INTEGER", nullable: false, primaryKey: true },
              { name: "title", dataType: "TEXT", nullable: false, primaryKey: false },
              { name: "metadata", dataType: "TEXT", nullable: true, primaryKey: false },
            ],
            foreignKeys: [],
          },
          {
            schema: null,
            name: "comments",
            columns: [
              { name: "id", dataType: "INTEGER", nullable: false, primaryKey: true },
              { name: "document_id", dataType: "INTEGER", nullable: false, primaryKey: false },
              { name: "body", dataType: "TEXT", nullable: false, primaryKey: false },
            ],
            foreignKeys: [
              {
                table: "comments",
                column: "document_id",
                refTable: "documents",
                refColumn: "id",
              },
            ],
          },
        ],
      };
    }
    const schema = id === "hatchway-pg" ? "public" : "app";
    return {
      engine: id === "hatchway-pg" ? "postgres" : "mysql",
      schemas: [schema],
      tables: [
        {
          schema,
          name: "users",
          columns: [
            { name: "id", dataType: "integer", nullable: false, primaryKey: true },
            { name: "email", dataType: "varchar", nullable: false, primaryKey: false },
            { name: "profile", dataType: "json", nullable: true, primaryKey: false },
          ],
          foreignKeys: [],
        },
        {
          schema,
          name: "orders",
          columns: [
            { name: "id", dataType: "integer", nullable: false, primaryKey: true },
            { name: "user_id", dataType: "integer", nullable: false, primaryKey: false },
            { name: "total", dataType: "decimal", nullable: false, primaryKey: false },
          ],
          foreignKeys: [
            { table: "orders", column: "user_id", refTable: "users", refColumn: "id" },
          ],
        },
      ],
    };
  }

  function demoDbRows(table: unknown): any {
    const name = typeof table === "string" ? table : "users";
    if (name === "documents") {
      return {
        columns: [
          { name: "id", dataType: "INTEGER", nullable: false, primaryKey: true },
          { name: "title", dataType: "TEXT", nullable: false, primaryKey: false },
          { name: "metadata", dataType: "TEXT", nullable: true, primaryKey: false },
        ],
        rows: [
          [1, "Launch notes", { status: "draft", tags: ["release", "docs"] }],
          [2, "API guide", { status: "published", tags: ["api"] }],
        ],
        affectedRows: 0,
        truncated: false,
      };
    }
    if (name === "orders") {
      return {
        columns: [
          { name: "id", dataType: "integer", nullable: false, primaryKey: true },
          { name: "user_id", dataType: "integer", nullable: false, primaryKey: false },
          { name: "total", dataType: "decimal", nullable: false, primaryKey: false },
        ],
        rows: [
          [101, 1, "129.00"],
          [102, 2, "74.50"],
        ],
        affectedRows: 0,
        truncated: false,
      };
    }
    return {
      columns: [
        { name: "id", dataType: "integer", nullable: false, primaryKey: true },
        { name: "email", dataType: "varchar", nullable: false, primaryKey: false },
        { name: "profile", dataType: "json", nullable: true, primaryKey: false },
      ],
      rows: [
        [1, "nora@example.test", { role: "admin", flags: ["beta"] }],
        [2, "dev@example.test", { role: "member", flags: [] }],
      ],
      affectedRows: 0,
      truncated: false,
    };
  }

  function demoExplain(id: unknown): any {
    const driver =
      id === "hatchway-pg" ? "postgres" : id === "quill-sqlite" ? "sqlite" : "mysql";
    const sqlite = driver === "sqlite";
    const empty = {
      buffersHit: null,
      buffersRead: null,
      actualRows: null,
      actualTimeMs: null,
      actualLoops: null,
      hashCondition: null,
      extra: {},
    };
    const root = {
      id: "node-0",
      nodeType: sqlite ? "SEARCH" : "Hash Join",
      relation: sqlite ? "users" : null,
      startupCost: sqlite ? null : 18.5,
      totalCost: sqlite ? null : 412.7,
      planRows: sqlite ? null : 1280,
      filter: null,
      indexCondition: null,
      joinType: sqlite ? null : "Inner",
      ...empty,
      hashCondition: sqlite ? null : "users.id = orders.user_id",
      children: [
        {
          id: "node-1",
          nodeType: sqlite ? "SCAN" : "Seq Scan",
          relation: "users",
          startupCost: sqlite ? null : 0,
          totalCost: sqlite ? null : 254.0,
          planRows: sqlite ? null : 9800,
          filter: "users.active = true",
          indexCondition: null,
          joinType: null,
          ...empty,
          extra: sqlite ? {} : { accessType: "ALL" },
          children: [],
        },
        {
          id: "node-2",
          nodeType: sqlite ? "SEARCH" : "Index Scan",
          relation: "orders",
          startupCost: sqlite ? null : 0.42,
          totalCost: sqlite ? null : 96.3,
          planRows: sqlite ? null : 1280,
          filter: null,
          indexCondition: sqlite ? "orders_user_id_idx" : "orders.user_id = users.id",
          joinType: null,
          ...empty,
          children: [],
        },
      ],
    };
    return {
      root,
      planningTimeMs: sqlite ? null : 0.42,
      executionTimeMs: null,
      originalQuery: "SELECT * FROM users JOIN orders ON orders.user_id = users.id",
      driver,
      hasAnalyzeData: false,
      rawOutput: sqlite
        ? "QUERY PLAN\n`--SEARCH users\n   `--SCAN orders"
        : JSON.stringify({ "Node Type": "Hash Join", demo: true }, null, 2),
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
      sans: [p.hostname, "localhost", "127.0.0.1", "::1"],
      status: "ready",
      trustStoreVerified: true,
      errors: [],
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

  function pushChannelMessage(channel: unknown, index: number, message: unknown): void {
    const ch = channel as { id?: number } | undefined;
    if (!ch || typeof ch.id !== "number") return;
    const cb = w["_" + ch.id];
    if (typeof cb === "function") cb({ index, message });
  }

  // ── SSH simulator helpers ─────────────────────────────────────────────────
  // The Tauri `Channel<T>` arg is the only value carrying a numeric `.id`, so we
  // find it positionally — robust to the arg name (onEvent / onLine / …).
  function channelArg(args?: Record<string, unknown>): unknown {
    if (!args) return undefined;
    for (const v of Object.values(args)) {
      if (v && typeof v === "object" && typeof (v as { id?: unknown }).id === "number") {
        return v;
      }
    }
    return undefined;
  }
  function toBytes(s: string): number[] {
    return Array.from(new TextEncoder().encode(s));
  }
  function sshHostData(id: unknown): any {
    const hosts = fixtures.sshHosts as Record<string, any> | undefined;
    return hosts ? hosts[String(id)] : undefined;
  }
  function sftpListing(id: unknown, path: unknown): any[] {
    const h = sshHostData(id);
    if (!h || !h.sftp) return [];
    return h.sftp[String(path)] || [];
  }
  function sftpFileText(id: unknown, path: unknown): string | null {
    const editKey = `${String(id)}::${String(path)}`;
    if (editKey in sshFileEdits) return sshFileEdits[editKey];
    const h = sshHostData(id);
    if (h && h.files && String(path) in h.files) return h.files[String(path)];
    return null;
  }
  function parentPath(p: unknown): string {
    const s = String(p);
    const i = s.lastIndexOf("/");
    return i <= 0 ? "/" : s.slice(0, i);
  }
  function baseName(p: unknown): string {
    const s = String(p);
    return s.slice(s.lastIndexOf("/") + 1);
  }
  function modeStr(mode: number | null): string {
    const m = mode == null ? 0o644 : mode;
    const bit = (n: number, ch: string) => ((m & n) !== 0 ? ch : "-");
    return (
      bit(0o400, "r") + bit(0o200, "w") + bit(0o100, "x") +
      bit(0o40, "r") + bit(0o20, "w") + bit(0o10, "x") +
      bit(0o4, "r") + bit(0o2, "w") + bit(0o1, "x")
    );
  }
  function pushData(p: any, s: string): void {
    pushChannelMessage(p.channel, p.idx++, { type: "data", bytes: toBytes(s) });
  }
  function promptText(p: any): string {
    return `\x1b[1;32m${p.user}@${p.hostName}\x1b[0m:\x1b[1;34m~\x1b[0m$ `;
  }
  function ptyMotd(conn: any, h: any): string {
    const os = (h?.snapshotStdout ?? "").match(/###OS\n(.*)/)?.[1]?.trim() ?? "Linux";
    const df = (h?.snapshotStdout ?? "").match(/###DISK\n[^\n]*\n([^\n]*)/)?.[1] ?? "";
    const usage = df.match(/(\d+%)/)?.[1] ?? "46%";
    return [
      "",
      `Welcome to ${conn?.detectedOs ?? "Ubuntu 22.04.4 LTS"} (GNU/Linux ${os.replace(/^Linux /, "")} x86_64)`,
      "",
      " * Documentation:  https://help.ubuntu.example",
      " * Management:     https://landscape.acme.example",
      "",
      `  Usage of /:   ${usage}    Users logged in:  1`,
      "",
      "Last login: from 10.0.0.7",
      "",
    ].join("\r\n");
  }
  function runPtyCommand(p: any, line: string): string {
    const parts = line.split(/\s+/).filter(Boolean);
    const cmd = parts[0] ?? "";
    const h = sshHostData(p.connectionId);
    const home = h?.homeDir ?? "~";
    const eol = "\r\n";
    if (cmd === "") return "";
    if (cmd === "exit" || cmd === "logout") {
      p.exited = true;
      return "logout" + eol;
    }
    if (cmd === "clear") return "\x1b[2J\x1b[3J\x1b[H";
    if (cmd === "pwd") return home + eol;
    if (cmd === "whoami") return (p.user ?? "user") + eol;
    if (cmd === "hostname") return (p.hostName ?? "host") + eol;
    if (cmd === "id") return `uid=1000(${p.user}) gid=1000(${p.user}) groups=1000(${p.user})` + eol;
    if (cmd === "uptime") {
      const m = (h?.snapshotStdout ?? "").match(/###UP\n(.*)/);
      return (m ? m[1].trim() : "up") + eol;
    }
    if (cmd === "uname") {
      const os = (h?.snapshotStdout ?? "").match(/###OS\n(.*)/)?.[1]?.trim() ?? "Linux";
      return (parts.includes("-a") ? `${os} ${p.hostName} x86_64 GNU/Linux` : os) + eol;
    }
    if (cmd === "ls") {
      const entries = sftpListing(p.connectionId, home);
      if (!entries.length) return "";
      if (parts.some((x) => x.startsWith("-") && x.includes("l"))) {
        return (
          entries
            .map((e: any) => {
              const type = e.isDir ? "d" : "-";
              const sz = String(e.size).padStart(8, " ");
              return `${type}${modeStr(e.permissions)} 1 ${p.user} ${p.user} ${sz} ${e.name}`;
            })
            .join(eol) + eol
        );
      }
      return (
        entries
          .map((e: any) => (e.isDir ? `\x1b[1;34m${e.name}\x1b[0m` : e.name))
          .join("  ") + eol
      );
    }
    if (cmd === "cat") {
      const target = parts[1] ?? "";
      const abs = target.startsWith("/") ? target : `${home}/${target}`;
      const txt = sftpFileText(p.connectionId, abs);
      return txt != null
        ? txt.replace(/\n/g, eol)
        : `cat: ${target}: No such file or directory${eol}`;
    }
    if (cmd === "echo") return parts.slice(1).join(" ") + eol;
    if (cmd === "help") {
      return (
        [
          "Demo shell — try:",
          "  ls [-la]   pwd   whoami   hostname   id   uname -a",
          "  uptime   cat <file>   echo <text>   clear   exit",
        ].join(eol) + eol
      );
    }
    return `${cmd}: command not found${eol}`;
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
      case "get_notification_prefs":
        return Promise.resolve(prefs.notifications);
      case "set_notification_prefs": {
        const next = args && (args.prefs as Record<string, unknown>);
        if (next) prefs.notifications = next;
        return Promise.resolve(prefs.notifications);
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
      case "database_client_schema":
        return Promise.resolve(demoDbSchema(args && args.id));
      case "database_client_table_rows":
        return Promise.resolve(demoDbRows(args && args.table));
      case "database_client_query":
        return Promise.resolve(demoDbRows("users"));
      case "database_client_explain":
        return Promise.resolve(demoExplain(args && args.id));
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
        // Shape must match OnboardingStatus { onboarded, registryEmpty } — the
        // simulator ships seeded fixtures, so the board is onboarded + non-empty.
        return Promise.resolve({ onboarded: true, registryEmpty: false });
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

      // --- SSH connections ------------------------------------------------------
      case "ssh_connections_list":
        return Promise.resolve(sshConnections);
      case "ssh_connection_save": {
        const input = (args?.input ?? {}) as any;
        const id = input.id || `conn-${Date.now().toString(36)}`;
        const i = sshConnections.findIndex((c) => c.id === id);
        const prev = i >= 0 ? sshConnections[i] : null;
        const view = {
          id,
          name: input.name,
          sshHost: input.sshHost,
          sshPort: input.sshPort,
          sshUser: input.sshUser,
          authKind: input.authKind,
          keyPath: input.keyPath ?? null,
          proxyJump: input.proxyJump ?? null,
          identityId: input.identityId ?? null,
          proxy: input.proxy ?? null,
          tags: input.tags ?? [],
          color: input.color ?? null,
          notes: input.notes ?? null,
          detectedOs: prev?.detectedOs ?? null,
          environment: input.environment ?? null,
          stage: input.stage ?? null,
          region: input.region ?? null,
          provider: prev?.provider ?? null,
          createdAt: prev?.createdAt ?? Math.floor(Date.now() / 1000),
          lastUsed: prev?.lastUsed ?? null,
          tunnelCount: prev?.tunnelCount ?? 0,
          inUse: prev?.inUse ?? false,
        };
        if (i >= 0) sshConnections[i] = view;
        else sshConnections.push(view);
        return Promise.resolve(view);
      }
      case "ssh_connection_delete": {
        const i = sshConnections.findIndex((c) => c.id === args?.id);
        if (i >= 0) sshConnections.splice(i, 1);
        return Promise.resolve(null);
      }
      case "ssh_connection_touch": {
        const c = sshConnections.find((x) => x.id === args?.id);
        if (c) c.lastUsed = Math.floor(Date.now() / 1000);
        return Promise.resolve(null);
      }
      case "ssh_connection_probe":
        return Promise.resolve(
          (fixtures.sshProbes as any)?.[String(args?.id)] ?? {
            reachable: true,
            latencyMs: 30,
            health: "healthy",
            fingerprint: "SHA256:demoFingerprintValueForNewlyAddedHost000000",
            trust: "trusted",
          },
        );
      case "ssh_connection_detect_os": {
        const c = sshConnections.find((x) => x.id === args?.id);
        return Promise.resolve(c?.detectedOs ?? "Linux (unknown)");
      }
      case "ssh_config_import":
        return Promise.resolve([]);
      case "ssh_has_stored_credential": {
        const c = sshConnections.find((x) => x.id === args?.id);
        return Promise.resolve(c?.authKind === "password");
      }
      case "ssh_forget_credentials":
      case "ssh_set_credential":
      case "ssh_interaction_respond":
      case "ssh_interaction_cancel":
        return Promise.resolve(null);
      case "ssh_known_host_remove":
        return Promise.resolve(1);

      // --- SSH identities -------------------------------------------------------
      case "ssh_identities_list":
        return Promise.resolve(sshIdentities);
      case "ssh_identity_save": {
        const input = (args?.input ?? {}) as any;
        const id = input.id || `id-${Date.now().toString(36)}`;
        const i = sshIdentities.findIndex((x) => x.id === id);
        const view = {
          id,
          name: input.name,
          sshUser: input.sshUser,
          authKind: input.authKind,
          keyPath: input.keyPath ?? null,
          connectionCount: i >= 0 ? sshIdentities[i].connectionCount : 0,
          inUse: i >= 0 ? sshIdentities[i].inUse : false,
        };
        if (i >= 0) sshIdentities[i] = view;
        else sshIdentities.push(view);
        return Promise.resolve(view);
      }
      case "ssh_identity_delete": {
        const i = sshIdentities.findIndex((x) => x.id === args?.id);
        if (i >= 0) sshIdentities.splice(i, 1);
        return Promise.resolve(null);
      }

      // --- SSH tunnels ----------------------------------------------------------
      case "ssh_tunnel_list":
        return Promise.resolve(sshTunnels);
      case "ssh_tunnel_save": {
        const input = (args?.input ?? {}) as any;
        const id = input.id || `tun-${Date.now().toString(36)}`;
        const conn = sshConnections.find((c) => c.id === input.connectionId);
        const i = sshTunnels.findIndex((t) => t.id === id);
        const view = {
          id,
          connectionId: input.connectionId ?? conn?.id ?? "",
          name: input.name,
          sshHost: input.sshHost ?? conn?.sshHost ?? "",
          sshPort: input.sshPort ?? conn?.sshPort ?? 22,
          sshUser: input.sshUser ?? conn?.sshUser ?? "",
          authKind: input.authKind ?? conn?.authKind ?? "key",
          keyPath: input.keyPath ?? conn?.keyPath ?? null,
          localHost: input.localHost ?? "127.0.0.1",
          localPort: input.localPort ?? input.remotePort ?? 0,
          remoteHost: input.remoteHost ?? "127.0.0.1",
          remotePort: input.remotePort ?? 0,
          forwardKind: input.forwardKind ?? "local",
          proxyJump: input.proxyJump ?? null,
          keepAlive: !!input.keepAlive,
          autoReconnect: !!input.autoReconnect,
          state: "down",
          running: false,
          startedAtMs: null,
          command: `ssh -N -L ${input.localPort ?? ""}:${input.remoteHost ?? "127.0.0.1"}:${input.remotePort ?? ""} ${input.sshUser ?? ""}@${input.sshHost ?? ""}`,
        };
        if (i >= 0) sshTunnels[i] = view;
        else sshTunnels.push(view);
        return Promise.resolve(view);
      }
      case "ssh_tunnel_start": {
        const t = sshTunnels.find((x) => x.id === args?.id);
        if (t) {
          t.running = true;
          t.state = "live";
          t.startedAtMs = Date.now();
        }
        return Promise.resolve(t ?? null);
      }
      case "ssh_tunnel_stop": {
        const t = sshTunnels.find((x) => x.id === args?.id);
        if (t) {
          t.running = false;
          t.state = "down";
          t.startedAtMs = null;
        }
        return Promise.resolve(null);
      }
      case "ssh_tunnel_delete": {
        const i = sshTunnels.findIndex((x) => x.id === args?.id);
        if (i >= 0) sshTunnels.splice(i, 1);
        return Promise.resolve(null);
      }
      case "ssh_tunnel_test":
        return Promise.resolve(null);
      case "ssh_tunnel_open_database":
        return Promise.resolve("demo-db-from-tunnel");

      // --- Remote exec / deploy -------------------------------------------------
      case "ssh_exec_run": {
        const input = (args?.input ?? {}) as any;
        const command = String(input.command ?? "");
        const h = sshHostData(input.connectionId);
        let stdout = "";
        if (command.includes("###USER")) stdout = h?.snapshotStdout ?? "";
        else if (command.includes("ps aux")) stdout = h?.psStdout ?? "";
        else if (command.includes("ss -tlnp") || command.includes("netstat"))
          stdout = h?.portsStdout ?? "";
        return Promise.resolve({ stdout, stderr: "", exitCode: 0 });
      }
      case "ssh_deploy_run": {
        const input = (args?.input ?? {}) as any;
        const steps: string[] = Array.isArray(input.steps) ? input.steps : [];
        return Promise.resolve(
          steps.map((command) => ({
            command,
            stdout: `$ ${command}\n✓ ok`,
            stderr: "",
            exitCode: 0,
          })),
        );
      }

      // --- SFTP file manager ----------------------------------------------------
      case "sftp_connect":
      case "sftp_home_dir":
        return Promise.resolve(sshHostData(args?.connectionId)?.homeDir ?? "/home/deploy");
      case "sftp_list_dir":
        return Promise.resolve(
          sftpListing((args?.input as any)?.connectionId, (args?.input as any)?.path),
        );
      case "sftp_stat": {
        const inp = (args?.input ?? {}) as any;
        const found = sftpListing(inp.connectionId, parentPath(inp.path)).find(
          (e: any) => e.path === inp.path,
        );
        return Promise.resolve(
          found ?? {
            name: baseName(inp.path),
            path: inp.path,
            isDir: false,
            isSymlink: false,
            size: 0,
            permissions: 0o644,
            mtimeSecs: Math.floor(Date.now() / 1000),
          },
        );
      }
      case "sftp_read_text": {
        const inp = (args?.input ?? {}) as any;
        return Promise.resolve(sftpFileText(inp.connectionId, inp.path) ?? "");
      }
      case "sftp_read_preview": {
        const inp = (args?.input ?? {}) as any;
        const txt = sftpFileText(inp.connectionId, inp.path);
        if (txt != null) {
          return Promise.resolve({
            kind: "text",
            mime: "text/plain",
            base64: null,
            text: txt,
            size: txt.length,
          });
        }
        return Promise.resolve({
          kind: "binary",
          mime: "application/octet-stream",
          base64: null,
          text: null,
          size: 0,
        });
      }
      case "sftp_write_text": {
        const inp = (args?.input ?? {}) as any;
        sshFileEdits[`${String(inp.connectionId)}::${String(inp.path)}`] = String(
          inp.contents ?? "",
        );
        return Promise.resolve(null);
      }
      case "sftp_mkdir":
      case "sftp_rename":
      case "sftp_remove_file":
      case "sftp_remove_dir":
      case "sftp_chmod":
      case "sftp_disconnect":
        return Promise.resolve(null);
      case "sftp_upload":
      case "sftp_download":
        return Promise.resolve(1024);
      case "sftp_transfer": {
        const inp = (args?.input ?? {}) as any;
        const tid = String(inp.id ?? "t");
        const total = 1_048_576;
        let sent = 0;
        const tick = () => {
          sent = Math.min(total, sent + 262_144);
          emit("portbay://sftp-progress", {
            id: tid,
            transferred: sent,
            total,
            done: sent >= total,
            error: null,
          });
          if (sent < total) setTimeout(tick, 120);
        };
        setTimeout(tick, 60);
        return Promise.resolve(total);
      }

      // --- Interactive PTY shell ------------------------------------------------
      case "ssh_pty_open": {
        const input = (args?.input ?? {}) as any;
        const connectionId = String(input.connectionId ?? "");
        const conn = sshConnections.find((c) => c.id === connectionId);
        const h = sshHostData(connectionId);
        const id = `pty-${ptySeq++}`;
        const p: any = {
          channel: channelArg(args),
          idx: 0,
          line: "",
          connectionId,
          user: conn?.sshUser ?? "user",
          hostName: conn?.name ?? "host",
          exited: false,
        };
        ptys[id] = p;
        const command = input.command as string | undefined;
        setTimeout(() => {
          if (command) {
            // Logs-follow style (tail -F / journalctl -f): canned stream, no prompt.
            pushData(p, `\x1b[2m$ ${command}\x1b[0m\r\n`);
            [
              "[deploy] pulling origin/main … done",
              "[deploy] pnpm install … up to date",
              "[deploy] next build … compiled successfully",
              "[deploy] pm2 reload acme-storefront … ✓ online",
            ].forEach((l) => pushData(p, l + "\r\n"));
            return;
          }
          pushData(p, ptyMotd(conn, h));
          pushData(p, promptText(p));
        }, 24);
        return Promise.resolve(id);
      }
      case "ssh_pty_input": {
        const id = String(args?.id ?? "");
        const data = String(args?.data ?? "");
        const p = ptys[id];
        if (!p) return Promise.resolve(null);
        for (const chr of data) {
          if (chr === "\r" || chr === "\n") {
            pushData(p, "\r\n");
            const out = runPtyCommand(p, p.line.trim());
            p.line = "";
            if (out) pushData(p, out);
            if (p.exited) {
              pushChannelMessage(p.channel, p.idx++, { type: "exit", code: 0 });
              delete ptys[id];
              return Promise.resolve(null);
            }
            pushData(p, promptText(p));
          } else if (chr === "\u007f" || chr === "\b") {
            if (p.line.length) {
              p.line = p.line.slice(0, -1);
              pushData(p, "\b \b");
            }
          } else if (chr === "\u0003") {
            pushData(p, "^C");
            p.line = "";
            pushData(p, promptText(p));
          } else if (chr >= " ") {
            p.line += chr;
            pushData(p, chr);
          }
        }
        return Promise.resolve(null);
      }
      case "ssh_pty_resize":
        return Promise.resolve(null);
      case "ssh_pty_close": {
        delete ptys[String(args?.id ?? "")];
        return Promise.resolve(null);
      }

      // --- On-host AI agent -----------------------------------------------------
      case "ssh_agent_open":
        return Promise.resolve(
          sshHostData(args?.connectionId)?.agent ?? {
            hasCurl: true,
            hasWget: true,
            hasOllama: false,
            hasLlm: false,
            ollamaModels: [],
            port: 11434,
          },
        );
      case "ssh_agent_chat": {
        const channel = channelArg(args);
        const reply =
          "This host has Ollama with qwen2.5-coder:7b and llama3.1:8b pulled, on an A100 40GB (CUDA 12.4). For a quick generation run `ollama run qwen2.5-coder:7b`. Want me to check `nvidia-smi` for current GPU utilisation?";
        const tokens = reply.match(/\S+\s*/g) ?? [reply];
        let i = 0;
        let idx = 0;
        const step = () => {
          if (i < tokens.length) {
            pushChannelMessage(channel, idx++, { type: "token", text: tokens[i++] });
            setTimeout(step, 32);
          } else {
            pushChannelMessage(channel, idx++, { type: "done", content: reply });
          }
        };
        setTimeout(step, 40);
        return Promise.resolve(null);
      }
      case "ssh_agent_run": {
        const command = String(args?.command ?? "");
        return Promise.resolve({
          stdout: `$ ${command}\n(demo) command approved and run on the host`,
          stderr: "",
          exitCode: 0,
        });
      }
      case "ssh_agent_close":
        return Promise.resolve(null);

      // --- Cloudflare tunnels (public sharing) ----------------------------------
      case "list_tunnels":
        return Promise.resolve(fixtures.cfTunnels ?? []);
      case "list_named_tunnels":
        return Promise.resolve(fixtures.cfNamedTunnels ?? []);

      // --- Python project: venv provisioning stream -----------------------------
      case "provision_python_env": {
        const channel = channelArg(args);
        const lines = [
          "Creating virtualenv with uv …",
          "Resolving dependencies …",
          "Installed 24 packages in 1.2s",
          "Activated .venv (Python 3.12.3)",
        ];
        let k = 0;
        let idx = 0;
        const step = () => {
          if (k < lines.length) {
            pushChannelMessage(channel, idx++, { kind: "log", line: lines[k++] });
            setTimeout(step, 140);
          } else {
            pushChannelMessage(channel, idx++, { kind: "done" });
          }
        };
        setTimeout(step, 60);
        return Promise.resolve(null);
      }

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
