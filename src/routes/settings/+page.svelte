<!--
  Settings — operational toggles + diagnostics. Density + theme,
  DNS resolver install/uninstall, Mail catcher status, migration
  import, onboarding reset, and version metadata.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion, getTauriVersion } from "@tauri-apps/api/app";

  import { DashboardCard } from "$lib/components/atoms";
  import ImportSection from "$lib/components/imports/ImportSection.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { density, type Density } from "$lib/stores/density.svelte";
  import { theme, type Theme } from "$lib/stores/theme.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { safeInvoke } from "$lib/ipc";

  interface ResolverStatus {
    suffix: string;
    installed: boolean;
    path: string;
    currentContents: string | null;
    currentPort: number;
  }

  let dnsStatus = $state<ResolverStatus | null>(null);
  let dnsBusy = $state<boolean>(false);

  interface DomainSettings {
    domainSuffix: string;
    projectCount: number;
  }

  interface DomainMigration {
    oldSuffix: string;
    newSuffix: string;
    changedProjects: number;
    certDirsRemoved: number;
  }

  let domainSettings = $state<DomainSettings | null>(null);
  let domainDraft = $state<string>("test");
  let domainBusy = $state<boolean>(false);

  interface MailStatusInfo {
    running: boolean;
    smtpPort: number | null;
    uiPort: number | null;
  }
  let mailInfo = $state<MailStatusInfo>({ running: false, smtpPort: null, uiPort: null });

  interface CrashSummary {
    id: string;
    kind: "rust_panic" | "js_error" | "js_unhandled_rejection";
    message: string;
    createdAt: number;
  }

  interface TelemetrySettings {
    enabled: boolean;
    crashReportCount: number;
    endpointConfigured: boolean;
  }

  let telemetryInfo = $state<TelemetrySettings>({
    enabled: false,
    crashReportCount: 0,
    endpointConfigured: false,
  });
  let crashReports = $state<CrashSummary[]>([]);
  let telemetryBusy = $state<boolean>(false);

  async function refreshTelemetry() {
    try {
      telemetryInfo = await safeInvoke<TelemetrySettings>("telemetry_settings");
      crashReports = await safeInvoke<CrashSummary[]>("list_crash_reports");
    } catch {
      crashReports = [];
    }
  }

  async function sendCrash(id: string) {
    telemetryBusy = true;
    try {
      await safeInvoke("send_crash_report", { id });
      await refreshTelemetry();
    } catch {
      /* toast already pushed */
    } finally {
      telemetryBusy = false;
    }
  }

  async function discardCrash(id: string) {
    telemetryBusy = true;
    try {
      await safeInvoke("discard_crash_report", { id });
      await refreshTelemetry();
    } catch {
      /* toast already pushed */
    } finally {
      telemetryBusy = false;
    }
  }

  async function refreshMailStatus() {
    try {
      const health = await safeInvoke<{ mailpit: { status: string; detail?: string } }>(
        "sidecar_status",
      );
      const m = health.mailpit;
      const running = m.status === "running";
      let smtp: number | null = null;
      let ui: number | null = null;
      // Detail format: "smtp :1025 · ui :8025"
      if (m.detail) {
        const sm = m.detail.match(/smtp\s*:(\d+)/);
        const um = m.detail.match(/ui\s*:(\d+)/);
        if (sm) smtp = Number(sm[1]);
        if (um) ui = Number(um[1]);
      }
      mailInfo = { running, smtpPort: smtp, uiPort: ui };
    } catch {
      mailInfo = { running: false, smtpPort: null, uiPort: null };
    }
  }

  async function refreshDnsStatus() {
    try {
      dnsStatus = await safeInvoke<ResolverStatus>("dnsmasq_resolver_status");
    } catch {
      dnsStatus = null;
    }
  }

  async function refreshDomainSettings() {
    try {
      domainSettings = await safeInvoke<DomainSettings>("get_domain_settings");
      domainDraft = domainSettings.domainSuffix;
    } catch {
      domainSettings = null;
    }
  }

  async function saveDomainSuffix() {
    const next = domainDraft.trim().replace(/^\./, "");
    if (!next || next === domainSettings?.domainSuffix) return;
    domainBusy = true;
    try {
      const migration = await safeInvoke<DomainMigration>("update_domain_suffix", {
        domainSuffix: next,
      });
      errorBus.push({
        code: "DOMAIN_SUFFIX_UPDATED",
        whatHappened: `Domain suffix changed from .${migration.oldSuffix} to .${migration.newSuffix}.`,
        whyItMatters: `${migration.changedProjects} project hostname(s) were migrated. PortBay will reconcile DNS, Caddy, and certificates in the background.`,
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refreshDomainSettings();
      await refreshDnsStatus();
    } catch {
      /* toast already pushed */
    } finally {
      domainBusy = false;
    }
  }

  async function installDns() {
    dnsBusy = true;
    try {
      await safeInvoke("dnsmasq_install_resolver");
      errorBus.push({
        code: "DNS_INSTALLED",
        whatHappened: `DNS routing for .${dnsStatus?.suffix ?? "test"} installed.`,
        whyItMatters:
          "Subdomains of this suffix now resolve to 127.0.0.1 via dnsmasq. /etc/hosts edits are no longer needed.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refreshDnsStatus();
    } catch {
      /* toast already pushed */
    } finally {
      dnsBusy = false;
    }
  }

  async function uninstallDns() {
    dnsBusy = true;
    try {
      await safeInvoke("dnsmasq_uninstall_resolver");
      errorBus.push({
        code: "DNS_REMOVED",
        whatHappened: `DNS routing for .${dnsStatus?.suffix ?? "test"} removed.`,
        whyItMatters:
          "PortBay will fall back to writing /etc/hosts on the next reconcile tick.",
        whoCausedIt: "system",
        severity: "info",
        actions: [],
      });
      await refreshDnsStatus();
    } catch {
      /* toast already pushed */
    } finally {
      dnsBusy = false;
    }
  }

  async function openMailInbox() {
    if (!mailInfo.uiPort) return;
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(`http://127.0.0.1:${mailInfo.uiPort}`);
  }

  let appVersion = $state<string>("…");
  let tauriVersion = $state<string>("…");

  onMount(() => {
    void preferences.load();
    void refreshDomainSettings();
    void refreshDnsStatus();
    void refreshMailStatus();
    void refreshTelemetry();
    void (async () => {
      try {
        appVersion = await getVersion();
        tauriVersion = await getTauriVersion();
      } catch {
        appVersion = "unknown";
        tauriVersion = "unknown";
      }
    })();
  });

  const densityOptions: { value: Density; label: string; detail: string }[] = [
    {
      value: "comfortable",
      label: "Comfortable",
      detail: "Spacious rows, friendly empty states. Recommended for new users.",
    },
    {
      value: "compact",
      label: "Compact",
      detail:
        "Tighter rows, icon-only status, no right-rail. Optimized for power users.",
    },
  ];

  const densityPreviewRows = ["PortBay smoke", "CMS", "API"];

  const themeOptions: { value: Theme; label: string; detail: string }[] = [
    {
      value: "dark",
      label: "Dark",
      detail: "Default PortBay theme for local-dev work sessions.",
    },
    {
      value: "light",
      label: "Light",
      detail: "Higher ambient-light theme with the same status taxonomy.",
    },
  ];

  async function triggerDemoError() {
    // Calls a real command with input that's guaranteed to fail. The Rust
    // side returns AppError::NotFound, which round-trips through the
    // CommandError envelope and lands as a toast in the bottom-right.
    try {
      await safeInvoke("start_project", { id: "this-project-does-not-exist" });
    } catch {
      // safeInvoke already pushed the toast.
    }
  }
</script>

<div class="p-6 max-w-2xl space-y-4">
  <DashboardCard title="Theme" flush>
    <div class="space-y-3">
      {#each themeOptions as opt (opt.value)}
        <label
          class="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors
                 {theme.value === opt.value
            ? 'border-accent/60 bg-accent/8'
            : 'border-border hover:border-border-strong'}"
        >
          <input
            type="radio"
            name="theme"
            value={opt.value}
            checked={theme.value === opt.value}
            onchange={() => theme.set(opt.value)}
            class="mt-1 accent-accent"
          />
          <div>
            <div class="text-sm font-medium text-fg">{opt.label}</div>
            <div class="text-xs text-fg-muted">{opt.detail}</div>
          </div>
        </label>
      {/each}
    </div>
  </DashboardCard>

  <DashboardCard title="Density" flush>
    <div class="space-y-3">
      {#each densityOptions as opt (opt.value)}
        <label
          class="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors
                 {density.value === opt.value
            ? 'border-accent/60 bg-accent/8'
            : 'border-border hover:border-border-strong'}"
        >
          <input
            type="radio"
            name="density"
            value={opt.value}
            checked={density.value === opt.value}
            onchange={() => density.set(opt.value)}
            class="mt-1 accent-accent"
          />
          <div>
            <div class="text-sm font-medium text-fg">{opt.label}</div>
            <div class="text-xs text-fg-muted">{opt.detail}</div>
            <div
              class="mt-3 w-52 rounded-md border border-border bg-bg/70 overflow-hidden"
              aria-hidden="true"
            >
              {#each densityPreviewRows as row, i (row)}
                <div
                  class="flex items-center gap-2 px-2 border-b border-border/60 last:border-b-0
                         {opt.value === 'compact' ? 'h-7' : 'h-9'}"
                >
                  <span class="h-1.5 w-1.5 rounded-full bg-status-running"></span>
                  <span class="min-w-0 flex-1 truncate text-[10px] text-fg-muted">
                    {row}
                  </span>
                  {#if opt.value === "comfortable"}
                    <span class="rounded border border-border px-1 text-[9px] text-fg-subtle">
                      {i === 0 ? "Vite" : "PHP"}
                    </span>
                  {/if}
                  <span class="font-mono text-[9px] text-fg-subtle">
                    {i === 0 ? "5173" : "—"}
                  </span>
                </div>
              {/each}
            </div>
          </div>
        </label>
      {/each}
    </div>
  </DashboardCard>

  <DashboardCard title="Behavior" flush>
    <div class="space-y-3">
      <p class="text-xs text-fg-muted leading-relaxed">
        PortBay can live in your menu bar so quick actions don't require
        focusing the window. The tray icon shows aggregate project
        health and exposes Start, Stop, Restart, and Open for every
        project — same as the dashboard, one click away.
      </p>

      <label
        class="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors
               border-border hover:border-border-strong"
      >
        <input
          type="checkbox"
          checked={preferences.value.showTrayIcon}
          onchange={(e) =>
            preferences.update({
              showTrayIcon: (e.currentTarget as HTMLInputElement).checked,
            })}
          class="mt-1 accent-accent"
          disabled={!preferences.loaded}
        />
        <div>
          <div class="text-sm font-medium text-fg">Show menu bar icon</div>
          <div class="text-xs text-fg-muted">
            Adds a status-coloured PortBay icon to the macOS menu bar.
            Gray when idle, blue while starting, green when healthy,
            red when something needs attention.
          </div>
        </div>
      </label>

      <label
        class="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors
               {preferences.value.showTrayIcon
          ? 'border-border hover:border-border-strong'
          : 'border-border opacity-60 cursor-not-allowed'}"
      >
        <input
          type="checkbox"
          checked={preferences.value.closeToMenuBar}
          onchange={(e) =>
            preferences.update({
              closeToMenuBar: (e.currentTarget as HTMLInputElement).checked,
            })}
          class="mt-1 accent-accent"
          disabled={!preferences.loaded || !preferences.value.showTrayIcon}
        />
        <div>
          <div class="text-sm font-medium text-fg">Close to menu bar</div>
          <div class="text-xs text-fg-muted">
            Closing the window keeps the app and your projects running.
            Quit explicitly from the tray menu (or press ⌘Q) to stop
            everything. Requires the menu bar icon.
          </div>
        </div>
      </label>
    </div>
  </DashboardCard>

  <DashboardCard title="Domain suffix" flush>
    <div class="space-y-3">
      <p class="text-xs text-fg-muted leading-relaxed">
        New projects default to <span class="font-mono">project.{domainSettings?.domainSuffix ?? "test"}</span>.
        Changing this rewrites existing project hostnames, clears affected
        HTTPS cert directories, and asks the reconciler to update DNS and Caddy.
      </p>

      <div class="grid grid-cols-[1fr_auto] gap-2">
        <label class="sr-only" for="domain-suffix">Domain suffix</label>
        <div class="relative">
          <span class="absolute left-3 top-1/2 -translate-y-1/2 text-fg-subtle text-sm">.</span>
          <input
            id="domain-suffix"
            value={domainDraft}
            oninput={(e) => (domainDraft = (e.currentTarget as HTMLInputElement).value)}
            onkeydown={(e) => {
              if (e.key === "Enter") void saveDomainSuffix();
            }}
            class="w-full rounded-md bg-bg border border-border pl-6 pr-3 py-2 text-sm text-fg outline-none focus:border-accent/60 font-mono"
            placeholder="test"
            disabled={domainBusy}
          />
        </div>
        <button
          type="button"
          onclick={saveDomainSuffix}
          disabled={domainBusy || !domainDraft.trim() || domainDraft.trim().replace(/^\./, "") === domainSettings?.domainSuffix}
          class="px-3 py-2 text-xs rounded-md text-accent border border-accent/40 hover:bg-accent/10 transition-colors disabled:opacity-50 disabled:hover:bg-transparent"
        >
          {domainBusy ? "Saving…" : "Apply"}
        </button>
      </div>

      <div class="space-y-1 text-[11px] text-fg-muted leading-relaxed">
        <p><span class="text-fg">.test</span> is recommended for local development and stays the default.</p>
        <p><span class="text-fg">.local</span> can clash with mDNS on some networks.</p>
        <p><span class="text-fg">.dev</span> is browser-HSTS enforced; keep HTTPS enabled.</p>
        <p>Public suffixes such as <span class="font-mono">.com</span>, <span class="font-mono">.net</span>, and <span class="font-mono">.io</span> are rejected.</p>
      </div>

      {#if domainSettings}
        <div class="text-[11px] text-fg-subtle">
          {domainSettings.projectCount} project{domainSettings.projectCount === 1 ? "" : "s"} will be checked during migration.
        </div>
      {/if}
    </div>
  </DashboardCard>

  <DashboardCard title="DNS routing" flush>
    {#if dnsStatus}
      <div class="space-y-3">
        <p class="text-xs text-fg-muted">
          PortBay can route every <span class="font-mono">*.{dnsStatus.suffix}</span>
          query to its local dnsmasq daemon on port
          <span class="font-mono">{dnsStatus.currentPort}</span>. One macOS
          authorisation prompt; no <span class="font-mono">/etc/hosts</span>
          edits afterwards.
        </p>

        <div class="flex items-center gap-2 text-xs">
          <span class="text-fg-muted">Status:</span>
          {#if dnsStatus.installed}
            <span class="text-status-running font-medium">Installed</span>
          {:else}
            <span class="text-fg-subtle">Not installed</span>
          {/if}
          <span class="ml-3 text-fg-subtle">File:</span>
          <span class="font-mono text-[11px] text-fg-muted truncate">{dnsStatus.path}</span>
        </div>

        {#if dnsStatus.currentContents}
          <pre class="text-[11px] font-mono bg-bg/60 border border-border rounded-md p-2 leading-relaxed text-fg-muted">{dnsStatus.currentContents}</pre>
        {/if}

        <div class="flex items-center gap-2 pt-1">
          {#if dnsStatus.installed}
            <button
              type="button"
              onclick={uninstallDns}
              disabled={dnsBusy}
              class="px-3 py-1.5 text-xs rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors disabled:opacity-50"
            >
              {dnsBusy ? "Working…" : "Remove DNS routing"}
            </button>
            <button
              type="button"
              onclick={installDns}
              disabled={dnsBusy}
              title="Re-install with current port (if it changed since first install)"
              class="px-3 py-1.5 text-xs rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors disabled:opacity-50"
            >
              Reinstall
            </button>
          {:else}
            <button
              type="button"
              onclick={installDns}
              disabled={dnsBusy}
              class="px-3 py-1.5 text-xs rounded-md text-accent border border-accent/40 hover:bg-accent/10 transition-colors disabled:opacity-50"
            >
              {dnsBusy ? "Installing…" : "Install DNS routing"}
            </button>
          {/if}
        </div>
      </div>
    {:else}
      <p class="text-xs text-fg-subtle">Loading DNS status…</p>
    {/if}
  </DashboardCard>

  <DashboardCard title="Mail catcher" flush>
    {#if mailInfo.running}
      <div class="space-y-3">
        <p class="text-xs text-fg-muted">
          Mailpit catches every outgoing SMTP message from your local
          projects. Frameworks reading <span class="font-mono">MAIL_HOST</span>
          /
          <span class="font-mono">MAIL_PORT</span> see these defaults
          automatically — the reconciler injects them into every project's
          environment unless the project overrides them.
        </p>
        <dl class="grid grid-cols-[140px,1fr] gap-x-4 gap-y-1.5 text-xs">
          <dt class="text-fg-muted">SMTP host</dt>
          <dd class="text-fg font-mono">127.0.0.1</dd>
          <dt class="text-fg-muted">SMTP port</dt>
          <dd class="text-fg font-mono">{mailInfo.smtpPort ?? "—"}</dd>
          <dt class="text-fg-muted">Web UI</dt>
          <dd class="text-fg font-mono">http://127.0.0.1:{mailInfo.uiPort ?? "—"}</dd>
          <dt class="text-fg-muted">Retention</dt>
          <dd class="text-fg">1 000 most-recent messages (auto-rotates)</dd>
        </dl>
        <button
          type="button"
          onclick={openMailInbox}
          class="px-3 py-1.5 text-xs rounded-md text-accent border border-accent/40 hover:bg-accent/10 transition-colors"
        >
          Open inbox
        </button>
      </div>
    {:else}
      <p class="text-xs text-fg-subtle">
        Mailpit isn't running. Install via
        <span class="font-mono">brew install mailpit</span> or place the
        bundled binary, then restart from the dashboard's Mailpit card.
      </p>
    {/if}
  </DashboardCard>

  <DashboardCard title="Crash reporting" flush>
    <div class="space-y-3">
      <label
        class="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors border-border hover:border-border-strong"
      >
        <input
          type="checkbox"
          checked={preferences.value.telemetryEnabled}
          onchange={async (e) => {
            await preferences.update({
              telemetryEnabled: (e.currentTarget as HTMLInputElement).checked,
            });
            await refreshTelemetry();
          }}
          class="mt-1 accent-accent"
          disabled={!preferences.loaded}
        />
        <div>
          <div class="text-sm font-medium text-fg">
            Send anonymous diagnostics
          </div>
          <div class="text-xs text-fg-muted">
            Off by default. When enabled, PortBay may send OS, app
            version, command name, and success/failure. Crash reports
            include panic message and sanitized backtrace only.
          </div>
        </div>
      </label>

      <details class="rounded-md border border-border bg-bg/50 p-3">
        <summary class="text-xs text-fg cursor-pointer">What we collect</summary>
        <ul class="mt-2 space-y-1 text-[11px] text-fg-muted leading-relaxed">
          <li>Telemetry: OS, architecture, app version, command name, success/failure.</li>
          <li>Crashes: panic or JS error message, sanitized backtrace, OS, architecture, app version.</li>
          <li>Never collected: project paths, hostnames, environment variables, registry contents, or log contents.</li>
        </ul>
      </details>

      <div class="text-[11px] text-fg-muted">
        Endpoint:
        {#if telemetryInfo.endpointConfigured}
          <span class="text-status-running">configured</span>
        {:else}
          <span class="text-fg-subtle">not configured for this build</span>
        {/if}
      </div>

      {#if crashReports.length > 0}
        <div class="space-y-2">
          {#each crashReports as report (report.id)}
            <div class="rounded-md border border-border bg-bg/60 p-3">
              <div class="flex items-start justify-between gap-3">
                <div class="min-w-0">
                  <div class="text-xs text-fg font-medium">
                    {report.kind.replaceAll("_", " ")}
                  </div>
                  <div class="mt-1 text-[11px] text-fg-muted break-words">
                    {report.message}
                  </div>
                </div>
                <div class="flex shrink-0 gap-1.5">
                  <button
                    type="button"
                    onclick={() => sendCrash(report.id)}
                    disabled={telemetryBusy || !preferences.value.telemetryEnabled}
                    class="px-2 py-1 text-[11px] rounded border border-accent/40 text-accent disabled:opacity-50"
                  >
                    Send
                  </button>
                  <button
                    type="button"
                    onclick={() => discardCrash(report.id)}
                    disabled={telemetryBusy}
                    class="px-2 py-1 text-[11px] rounded border border-border text-fg-muted disabled:opacity-50"
                  >
                    Discard
                  </button>
                </div>
              </div>
            </div>
          {/each}
        </div>
      {:else}
        <p class="text-xs text-fg-subtle">No pending crash reports.</p>
      {/if}
    </div>
  </DashboardCard>

  <ImportSection />

  <DashboardCard title="Diagnostics" flush>
    <p class="text-xs text-fg-muted mb-3">
      Smoke-test the error envelope round-trip — calls a command with a
      bogus id; the toast should appear in the bottom-right with a
      "system" error envelope.
    </p>
    <button
      type="button"
      onclick={triggerDemoError}
      class="text-xs px-3 py-1.5 rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors"
    >
      Trigger demo error
    </button>
  </DashboardCard>

  <DashboardCard title="Onboarding" flush>
    <div class="space-y-3">
      <p class="text-xs text-fg-muted leading-relaxed">
        Re-run the welcome flow to scaffold a new project from a template
        or replay the system health check.
      </p>
      <button
        type="button"
        onclick={async () => {
          await safeInvoke("reset_onboarding");
          window.location.assign("/onboarding");
        }}
        class="px-3 py-1.5 text-xs rounded-md border border-border
               text-fg-muted hover:text-fg hover:bg-surface-2
               transition-colors inline-flex items-center gap-1.5"
      >
        Re-run setup
      </button>
    </div>
  </DashboardCard>

  <DashboardCard title="About" flush>
    <dl class="grid grid-cols-[auto,1fr] gap-x-6 gap-y-2 text-xs">
      <dt class="text-fg-muted">App version</dt>
      <dd class="text-fg font-mono">{appVersion}</dd>
      <dt class="text-fg-muted">Tauri runtime</dt>
      <dd class="text-fg font-mono">{tauriVersion}</dd>
      <dt class="text-fg-muted">Source</dt>
      <dd>
        <a
          href="https://github.com/portbay-app/portbay"
          class="text-accent hover:text-accent-hover"
          target="_blank"
          rel="noopener noreferrer"
        >
          github.com/portbay-app/portbay
        </a>
      </dd>
    </dl>
  </DashboardCard>
</div>
