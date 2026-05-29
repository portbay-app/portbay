<!--
  AdvancedPanel — the catch-all tab.

  Folds the old Artifacts + Advanced cards and the previously-collapsed
  "Advanced toggles & diagnostics" region into one tab. The tab itself is the
  disclosure, so the collapse toggle is gone; the in-panel <h3> subheads group
  artifacts / logs / menu-bar / DNS / crash reporting / migration / about /
  updates, with the destructive reset actions in a footer.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion, getTauriVersion } from "@tauri-apps/api/app";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import ImportSection from "$lib/components/imports/ImportSection.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import {
    preferences,
    type AutoCleanSchedule,
  } from "$lib/stores/preferences.svelte";
  import { updater } from "$lib/stores/updater.svelte";
  import { safeInvoke } from "$lib/ipc";
  import type { ResolverStatus } from "$lib/types/dns";
  import type { CrashReportSummary, TelemetrySettings } from "$lib/types/telemetry";
  import SettingsPanel from "./SettingsPanel.svelte";

  // ---- DNS routing (kept rich because it's a system-level install) ----
  let dnsStatus = $state<ResolverStatus | null>(null);
  let dnsBusy = $state<boolean>(false);

  // ---- Crash reporting ----
  let telemetryInfo = $state<TelemetrySettings>({
    enabled: false,
    crashReportCount: 0,
    endpointConfigured: false,
  });
  let crashReports = $state<CrashReportSummary[]>([]);
  let telemetryBusy = $state<boolean>(false);

  let appVersion = $state<string>("…");
  let tauriVersion = $state<string>("…");

  let restoreArmed = $state<boolean>(false);
  let resetArmed = $state<boolean>(false);

  async function refreshTelemetry() {
    try {
      telemetryInfo = await safeInvoke<TelemetrySettings>("telemetry_settings");
      crashReports = await safeInvoke<CrashReportSummary[]>("list_crash_reports");
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
      /* toast */
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
      /* toast */
    } finally {
      telemetryBusy = false;
    }
  }

  async function refreshDnsStatus() {
    try {
      dnsStatus = await safeInvoke<ResolverStatus>("dnsmasq_resolver_status");
    } catch {
      dnsStatus = null;
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
          "Subdomains of this suffix now resolve to 127.0.0.1 via dnsmasq.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refreshDnsStatus();
    } catch {
      /* toast */
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
        whyItMatters: "Falls back to /etc/hosts on the next reconcile.",
        whoCausedIt: "system",
        severity: "info",
        actions: [],
      });
      await refreshDnsStatus();
    } catch {
      /* toast */
    } finally {
      dnsBusy = false;
    }
  }

  async function restoreDefaults() {
    if (!restoreArmed) {
      restoreArmed = true;
      setTimeout(() => (restoreArmed = false), 2_500);
      return;
    }
    restoreArmed = false;
    // "Restore defaults" is non-destructive — it just resets the user-facing
    // toggles, not the workspace/registry. Send a full patch using the same
    // defaults the Rust side ships with.
    await preferences.update({
      launchAtLogin: false,
      reopenPreviousProjects: false,
      confirmBeforeStopAll: true,
      desktopNotifications: false,
      accentColor: "blue",
      autoDetectProjects: false,
      defaultSort: "name-asc",
      defaultStartBehavior: "manual",
      manageHostsAutomatically: true,
      autoRenewCertificates: true,
      storeLogsLocally: true,
      logRetentionDays: 7,
    });
    theme.set("system");
    density.set("comfortable");
    errorBus.push({
      code: "SETTINGS_RESTORED",
      whatHappened: "Settings restored to defaults.",
      whyItMatters: "Your workspace and registered projects are unchanged.",
      whoCausedIt: "system",
      severity: "info",
      actions: [],
    });
  }

  async function resetAll() {
    if (!resetArmed) {
      resetArmed = true;
      setTimeout(() => (resetArmed = false), 2_500);
      return;
    }
    resetArmed = false;
    // "Reset all settings" matches "Restore defaults" today — there is no
    // separate destructive scope yet. The button still exists so the design's
    // "danger zone" affordance is visible and a future wipe-onboarding/registry
    // flow has a home.
    await preferences.update({
      launchAtLogin: false,
      reopenPreviousProjects: false,
      confirmBeforeStopAll: true,
      desktopNotifications: false,
      accentColor: "blue",
      autoDetectProjects: false,
      defaultSort: "name-asc",
      defaultStartBehavior: "manual",
      manageHostsAutomatically: true,
      autoRenewCertificates: true,
      storeLogsLocally: true,
      logRetentionDays: 7,
      showTrayIcon: true,
      closeToMenuBar: true,
      telemetryEnabled: false,
    });
    theme.set("system");
    density.set("comfortable");
    try {
      await safeInvoke("reset_onboarding");
    } catch {
      /* onboarding reset failure is non-fatal here */
    }
    errorBus.push({
      code: "SETTINGS_RESET",
      whatHappened: "All settings reset.",
      whyItMatters: "Onboarding will replay on the next launch.",
      whoCausedIt: "system",
      severity: "warning",
      actions: [],
    });
  }

  async function copyCliPath() {
    try {
      await navigator.clipboard.writeText(preferences.value.cliPath);
      errorBus.push({
        code: "COPIED",
        whatHappened: "CLI path copied.",
        whyItMatters: "Paste into your terminal config.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* private mode */
    }
  }

  /** Human "last cleaned" label from a Unix-seconds stamp (0 → never run). */
  function formatLastClean(secs: number): string {
    if (!secs) return "Never";
    return new Date(secs * 1000).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  }

  /** Parse the comma/whitespace-separated extra-dirs input into a clean list
      and persist it. Drops blanks; the backend re-sanitises for safety. */
  function saveExtraDirs(raw: string): void {
    const dirs = raw
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    void preferences.update({ autoCleanExtraDirs: dirs });
  }

  const cleanScheduleOptions: { value: AutoCleanSchedule; label: string }[] = [
    { value: "off", label: "Off" },
    { value: "weekly", label: "Weekly" },
    { value: "monthly", label: "Monthly" },
  ];

  const retentionOptions = [
    { value: 1, label: "1 day" },
    { value: 7, label: "7 days" },
    { value: 30, label: "30 days" },
    { value: 90, label: "90 days" },
    { value: 0, label: "Forever" },
  ];

  onMount(() => {
    void refreshTelemetry();
    void refreshDnsStatus();
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
</script>

<SettingsPanel
  title="Advanced"
  description="Storage, diagnostics, system integration, and reset."
>
  <div class="space-y-6">
    <!-- Artifacts -->
    <div>
      <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
        Artifacts
      </h3>
      <div class="divide-y divide-border/60">
        <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
          <div class="flex flex-col">
            <span class="text-[13px] text-fg">Auto-clean schedule</span>
            <span class="text-[11px] text-fg-subtle"
              >Periodically delete build output (node_modules, .next, vendor…)
              across all projects.</span
            >
          </div>
          <select
            value={preferences.value.autoCleanSchedule}
            onchange={(e) =>
              preferences.update({
                autoCleanSchedule: (e.currentTarget as HTMLSelectElement)
                  .value as AutoCleanSchedule,
              })}
            class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
          >
            {#each cleanScheduleOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <div class="flex items-center justify-between gap-3 py-2.5">
          <div class="flex flex-col">
            <span class="text-[13px] text-fg">Extra directories</span>
            <span class="text-[11px] text-fg-subtle"
              >Comma-separated, added to every project (e.g. .turbo, .cache).</span
            >
          </div>
          <input
            type="text"
            value={preferences.value.autoCleanExtraDirs.join(", ")}
            onchange={(e) =>
              saveExtraDirs((e.currentTarget as HTMLInputElement).value)}
            placeholder=".turbo, .cache"
            class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg font-mono focus:outline-none focus:border-accent/60"
          />
        </div>

        <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
          <span class="text-[13px] text-fg">Last cleaned</span>
          <span class="text-[12px] text-fg-muted"
            >{formatLastClean(preferences.value.lastAutoClean)}</span
          >
        </div>
      </div>
    </div>

    <!-- Logs & CLI -->
    <div>
      <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
        Logs & CLI
      </h3>
      <div class="divide-y divide-border/60">
        <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
          <span class="text-[13px] text-fg">Store logs locally</span>
          <Toggle
            checked={preferences.value.storeLogsLocally}
            label="Store logs locally"
            onchange={(v) => preferences.update({ storeLogsLocally: v })}
          />
        </div>

        <div class="flex items-center justify-between gap-3 py-2.5">
          <span class="text-[13px] text-fg">Keep logs for</span>
          <select
            value={preferences.value.logRetentionDays}
            onchange={(e) =>
              preferences.update({
                logRetentionDays: Number(
                  (e.currentTarget as HTMLSelectElement).value,
                ),
              })}
            class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
          >
            {#each retentionOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
          <span class="text-[13px] text-fg">PortBay CLI path</span>
          <div class="flex items-center gap-2">
            <input
              type="text"
              value={preferences.value.cliPath}
              oninput={(e) =>
                preferences.update({
                  cliPath: (e.currentTarget as HTMLInputElement).value,
                })}
              class="h-8 w-72 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg font-mono focus:outline-none focus:border-accent/60"
            />
            <button
              type="button"
              onclick={copyCliPath}
              title="Copy CLI path"
              aria-label="Copy CLI path"
              class="inline-flex items-center justify-center w-8 h-8 rounded-md border border-border text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
            >
              <Icon name="link" size={13} />
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Menu bar -->
    <div>
      <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
        Menu bar
      </h3>
      <div class="divide-y divide-border/60">
        <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
          <div>
            <p class="text-[13px] text-fg">Show menu bar icon</p>
            <p class="text-[11.5px] text-fg-muted max-w-md">
              Adds a status-coloured PortBay icon to the macOS menu bar. Gray
              when idle, blue while starting, green when healthy, red when
              something needs attention.
            </p>
          </div>
          <Toggle
            checked={preferences.value.showTrayIcon}
            label="Show menu bar icon"
            onchange={(v) => preferences.update({ showTrayIcon: v })}
          />
        </div>
        <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
          <div>
            <p class="text-[13px] text-fg">Close to menu bar</p>
            <p class="text-[11.5px] text-fg-muted max-w-md">
              Closing the window keeps the app and your projects running. Quit
              from the tray (or ⌘Q) to stop everything.
            </p>
          </div>
          <Toggle
            checked={preferences.value.closeToMenuBar}
            label="Close to menu bar"
            disabled={!preferences.value.showTrayIcon}
            onchange={(v) => preferences.update({ closeToMenuBar: v })}
          />
        </div>
      </div>
    </div>

    <!-- DNS routing -->
    <div>
      <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
        DNS routing
      </h3>
      {#if dnsStatus}
        <p class="text-[11.5px] text-fg-muted mb-2">
          PortBay can route every <span class="font-mono">*.{dnsStatus.suffix}</span>
          query to its local dnsmasq daemon on port
          <span class="font-mono">{dnsStatus.currentPort}</span>. One macOS
          authorisation prompt; no <span class="font-mono">/etc/hosts</span> edits
          after.
        </p>
        <div class="flex items-center gap-2 text-[12px] mb-2">
          <span class="text-fg-muted">Status:</span>
          {#if dnsStatus.installed}
            <span class="text-status-running font-medium">Installed</span>
          {:else}
            <span class="text-fg-subtle">Not installed</span>
          {/if}
          <span class="ml-3 text-fg-subtle">File:</span>
          <span class="font-mono text-[11px] text-fg-muted truncate">
            {dnsStatus.path}
          </span>
        </div>
        <div class="flex items-center gap-2">
          {#if dnsStatus.installed}
            <button
              type="button"
              onclick={uninstallDns}
              disabled={dnsBusy}
              class="h-7 px-3 text-[11.5px] rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors disabled:opacity-50"
            >
              {dnsBusy ? "Working…" : "Remove DNS routing"}
            </button>
            <button
              type="button"
              onclick={installDns}
              disabled={dnsBusy}
              class="h-7 px-3 text-[11.5px] rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors disabled:opacity-50"
            >
              Reinstall
            </button>
          {:else}
            <button
              type="button"
              onclick={installDns}
              disabled={dnsBusy}
              class="h-7 px-3 text-[11.5px] rounded-md text-accent border border-accent/40 hover:bg-accent/10 transition-colors disabled:opacity-50"
            >
              {dnsBusy ? "Installing…" : "Install DNS routing"}
            </button>
          {/if}
        </div>
      {:else}
        <p class="text-[12px] text-fg-subtle">Loading DNS status…</p>
      {/if}
    </div>

    <!-- Crash reporting -->
    <div>
      <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
        Crash reporting
      </h3>
      <div
        class="flex items-center justify-between gap-3 py-2.5 border-b border-border/60"
      >
        <div>
          <p class="text-[13px] text-fg">Send anonymous diagnostics</p>
          <p class="text-[11.5px] text-fg-muted max-w-md">
            Off by default. When on, PortBay may send OS, app version, command
            name, and success/failure. Crashes include panic message and
            sanitised backtrace only.
          </p>
        </div>
        <Toggle
          checked={preferences.value.telemetryEnabled}
          label="Send anonymous diagnostics"
          onchange={async (v) => {
            await preferences.update({ telemetryEnabled: v });
            await refreshTelemetry();
          }}
        />
      </div>
      {#if crashReports.length > 0}
        <ul class="mt-3 space-y-2">
          {#each crashReports as r (r.id)}
            <li
              class="flex items-start justify-between gap-3 p-3
                     rounded-md border border-border bg-bg/60"
            >
              <div class="min-w-0">
                <p class="text-[12px] text-fg font-medium">
                  {r.kind.replaceAll("_", " ")}
                </p>
                <p class="text-[11px] text-fg-muted break-words">
                  {r.message}
                </p>
              </div>
              <div class="flex shrink-0 gap-1.5">
                <button
                  type="button"
                  onclick={() => sendCrash(r.id)}
                  disabled={telemetryBusy}
                  class="h-7 px-2 text-[11px] rounded border border-accent/40 text-accent disabled:opacity-50"
                >
                  Send
                </button>
                <button
                  type="button"
                  onclick={() => discardCrash(r.id)}
                  disabled={telemetryBusy}
                  class="h-7 px-2 text-[11px] rounded border border-border text-fg-muted disabled:opacity-50"
                >
                  Discard
                </button>
              </div>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="mt-3 text-[12px] text-fg-subtle">No pending crash reports.</p>
      {/if}
    </div>

    <!-- Migration -->
    <div>
      <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
        Migration
      </h3>
      <ImportSection />
    </div>

    <!-- Onboarding / about -->
    <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
      <div>
        <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
          Onboarding
        </h3>
        <p class="text-[11.5px] text-fg-muted mb-2">
          Re-run the welcome flow to scaffold a new project or replay the system
          health check.
        </p>
        <button
          type="button"
          onclick={async () => {
            await safeInvoke("reset_onboarding");
            window.location.assign("/onboarding");
          }}
          class="h-7 px-3 text-[11.5px] rounded-md border border-border
                 text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          Re-run setup
        </button>
      </div>

      <div>
        <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
          About
        </h3>
        <dl class="grid grid-cols-[auto,1fr] gap-x-4 gap-y-1 text-[11.5px]">
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
      </div>
    </div>

    <!-- Updates -->
    <div>
      <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
        Updates
      </h3>
      <p class="text-[11.5px] text-fg-muted mb-2">
        PortBay checks for new signed releases on launch. Updates are verified
        against PortBay's public key before they install.
      </p>
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={updater.status === "checking" ||
            updater.status === "installing"}
          onclick={() => updater.check()}
          class="h-7 px-3 text-[11.5px] rounded-md border border-border
                 text-fg-muted hover:text-fg hover:bg-surface-2
                 transition-colors disabled:opacity-50"
        >
          {updater.status === "checking" ? "Checking…" : "Check for updates"}
        </button>
        {#if updater.available}
          <button
            type="button"
            disabled={updater.status === "installing"}
            onclick={() => updater.install()}
            class="h-7 px-3 text-[11.5px] rounded-md bg-accent text-on-accent
                   hover:bg-accent-hover transition-colors disabled:opacity-50"
          >
            {updater.status === "installing"
              ? "Installing…"
              : `Update to ${updater.available.version}`}
          </button>
        {/if}
      </div>
      <p class="text-[11px] text-fg-subtle mt-2">
        {#if updater.status === "uptodate"}
          You're on the latest version.
        {:else if updater.status === "available" && updater.available}
          Version {updater.available.version} is available.
        {:else if updater.status === "error"}
          Couldn't check for updates — try again later.
        {/if}
        {#if updater.lastChecked}
          · Last checked {new Date(updater.lastChecked).toLocaleString()}
        {/if}
      </p>
    </div>

    <!-- Danger zone — settings persist on change, so there's no Save. -->
    <div
      class="flex items-center justify-between gap-3 pt-4 border-t border-border/60"
    >
      <button
        type="button"
        onclick={restoreDefaults}
        class="text-[13px] text-fg-muted hover:text-fg transition-colors"
      >
        {restoreArmed ? "Click again to confirm" : "Restore defaults"}
      </button>

      <button
        type="button"
        onclick={resetAll}
        class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg
               border border-status-crashed/40
               text-[13px] text-status-crashed
               hover:bg-status-crashed/10 transition"
      >
        <Icon name="x" size={13} />
        {resetArmed ? "Confirm reset" : "Reset all settings…"}
      </button>
    </div>
  </div>
</SettingsPanel>
