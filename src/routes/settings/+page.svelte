<!--
  Settings (redesigned) — five primary cards (General, Appearance,
  Workspace & Projects, Domains & HTTPS, Advanced) plus a collapsed
  "Advanced toggles & diagnostics" region that preserves the existing
  rich surfaces (DNS install/uninstall, crash reports, mail catcher,
  onboarding re-run, migration import, about) so nothing regresses.

  Save model: every control writes through `preferences.update(...)`
  on change — the changes persist immediately. There is no Save button,
  because there's nothing to buffer-then-commit; one would only mislead.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion, getTauriVersion } from "@tauri-apps/api/app";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import SetupRequirements from "$lib/components/setup/SetupRequirements.svelte";
  import { AccountSection, SyncSection, EarlyAccessSection } from "$lib/components/account";
  import Segmented from "$lib/components/atoms/Segmented.svelte";
  import ColorSwatchGroup from "$lib/components/atoms/ColorSwatchGroup.svelte";

  import ImportSection from "$lib/components/imports/ImportSection.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { density, type Density } from "$lib/stores/density.svelte";
  import { theme, type Theme } from "$lib/stores/theme.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { updater } from "$lib/stores/updater.svelte";
  import type {
    AccentColor,
    DefaultSort,
    StartBehavior,
    AutoCleanSchedule,
  } from "$lib/stores/preferences.svelte";
  import { safeInvoke } from "$lib/ipc";
  // Canonical wire shapes — imported so they can't drift from the Rust side.
  import type { ResolverStatus, DomainMigration } from "$lib/types/dns";

  // ---- Existing data sources retained for the "Advanced" region ----
  let dnsStatus = $state<ResolverStatus | null>(null);
  let dnsBusy = $state<boolean>(false);

  interface DomainSettings {
    domainSuffix: string;
    projectCount: number;
  }
  let domainSettings = $state<DomainSettings | null>(null);
  let domainDraft = $state<string>("test");
  let domainBusy = $state<boolean>(false);

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

  let appVersion = $state<string>("…");
  let tauriVersion = $state<string>("…");

  // Theme has three options in the design — System maps to neither
  // dark nor light, so we surface it as a separate user pref that
  // overrides theme.value when set to "system". Stored locally
  // because the theme store today only knows dark|light.
  type ThemeChoice = "system" | Theme;
  function readSystemThemeChoice(): ThemeChoice {
    try {
      const v = localStorage.getItem("portbay.themeChoice");
      if (v === "system" || v === "dark" || v === "light") return v;
    } catch {
      /* private mode */
    }
    return theme.value;
  }

  let themeChoice = $state<ThemeChoice>(readSystemThemeChoice());

  function applyThemeChoice(choice: ThemeChoice) {
    themeChoice = choice;
    try {
      localStorage.setItem("portbay.themeChoice", choice);
    } catch {
      /* private mode */
    }
    if (choice === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      theme.set(mq.matches ? "dark" : "light");
    } else {
      theme.set(choice);
    }
  }

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
      const migration = await safeInvoke<DomainMigration>(
        "update_domain_suffix",
        { domainSuffix: next },
      );
      errorBus.push({
        code: "DOMAIN_SUFFIX_UPDATED",
        whatHappened: `Domain suffix changed from .${migration.oldSuffix} to .${migration.newSuffix}.`,
        whyItMatters: `${migration.changedProjects} project hostname(s) migrated. Reconciler will update DNS, Caddy, and certs.`,
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refreshDomainSettings();
      await refreshDnsStatus();
    } catch {
      /* toast */
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

  // ---- Settings actions ----
  let restoreArmed = $state<boolean>(false);
  let resetArmed = $state<boolean>(false);

  async function restoreDefaults() {
    if (!restoreArmed) {
      restoreArmed = true;
      setTimeout(() => (restoreArmed = false), 2_500);
      return;
    }
    restoreArmed = false;
    // "Restore defaults" is non-destructive — it just resets the
    // user-facing toggles, not the workspace/registry. Send a full
    // patch using the same defaults the Rust side ships with.
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
    applyThemeChoice("system");
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
    // "Reset all settings" matches "Restore defaults" today — there is
    // no separate destructive scope yet. The button still exists so
    // the design's "danger zone" affordance is visible and a future
    // wipe-onboarding/registry flow has a home.
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
    applyThemeChoice("system");
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

  async function pickWorkspaceFolder() {
    // Dialog plugin opens the native folder picker. Falls back to a
    // toast if the user denies the dialog (e.g. capability missing).
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        directory: true,
        multiple: false,
        title: "Choose default workspace folder",
        defaultPath:
          preferences.value.defaultWorkspaceFolder ||
          undefined,
      });
      if (typeof result === "string") {
        await preferences.update({ defaultWorkspaceFolder: result });
      }
    } catch {
      /* dialog plugin already toasted */
    }
  }

  onMount(() => {
    void preferences.load();
    void refreshDomainSettings();
    void refreshDnsStatus();
    void refreshTelemetry();

    // Arriving from the dashboard's "Fix it →" banner (/settings#setup):
    // bring the Setup surface into view once it has rendered.
    if (window.location.hash === "#setup") {
      requestAnimationFrame(() =>
        document.getElementById("setup")?.scrollIntoView({ block: "start" }),
      );
    }
    void (async () => {
      try {
        appVersion = await getVersion();
        tauriVersion = await getTauriVersion();
      } catch {
        appVersion = "unknown";
        tauriVersion = "unknown";
      }
    })();

    // If "System" is the theme choice and the OS preference flips
    // mid-session, follow it. We don't track in the store because
    // it's an inherently page-local concern.
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const listener = () => {
      if (themeChoice === "system") {
        theme.set(mq.matches ? "dark" : "light");
      }
    };
    mq.addEventListener("change", listener);
    return () => mq.removeEventListener("change", listener);
  });

  // ---- Static option lists ----
  const themeOptions: { value: ThemeChoice; label: string }[] = [
    { value: "system", label: "System" },
    { value: "light", label: "Light" },
    { value: "dark", label: "Dark" },
  ];

  const densityOptions: { value: Density; label: string }[] = [
    { value: "compact", label: "Compact" },
    { value: "comfortable", label: "Comfortable" },
  ];

  const sortOptions: { value: DefaultSort; label: string }[] = [
    { value: "name-asc", label: "Name (A–Z)" },
    { value: "name-desc", label: "Name (Z–A)" },
    { value: "status", label: "Status" },
    { value: "port", label: "Port" },
  ];

  const startOptions: { value: StartBehavior; label: string }[] = [
    { value: "manual", label: "Start manually" },
    { value: "auto", label: "Start automatically" },
  ];

  const cleanScheduleOptions: { value: AutoCleanSchedule; label: string }[] = [
    { value: "off", label: "Off" },
    { value: "weekly", label: "Weekly" },
    { value: "monthly", label: "Monthly" },
  ];

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

  const retentionOptions = [
    { value: 1, label: "1 day" },
    { value: 7, label: "7 days" },
    { value: 30, label: "30 days" },
    { value: 90, label: "90 days" },
    { value: 0, label: "Forever" },
  ];

  let advancedDiagnosticsOpen = $state<boolean>(false);
</script>

<div class="px-6 py-5 space-y-6">
  <!-- Page heading -->
  <header class="space-y-1">
    <h1 class="text-[22px] font-semibold tracking-tight text-fg">Settings</h1>
    <p class="text-[13px] text-fg-muted">
      Control how PortBay manages your local development environment.
    </p>
  </header>

  <!-- ============== Setup required (self-hides when healthy) ============== -->
  <SetupRequirements />

  <!-- ============== Account & Plan ============== -->
  <AccountSection />

  <!-- ============== Sync (Pro) ============== -->
  <SyncSection />

  <!-- ============== Early Access (Pro) ============== -->
  <EarlyAccessSection />

  <!-- ============== General ============== -->
  <section
    class="bg-surface border border-border rounded-2xl p-5
           grid grid-cols-[180px,1fr] gap-x-6"
  >
    <div class="flex items-start gap-2.5">
      <span
        class="inline-flex items-center justify-center w-8 h-8 rounded-lg
               bg-fg-muted/10 text-fg-muted"
      >
        <Icon name="settings" size={15} />
      </span>
      <span class="text-[14px] font-semibold text-fg pt-1">General</span>
    </div>

    <div class="divide-y divide-border/60">
      <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
        <span class="text-[13px] text-fg">Launch PortBay at login</span>
        <Toggle
          checked={preferences.value.launchAtLogin}
          label="Launch PortBay at login"
          onchange={(v) => preferences.update({ launchAtLogin: v })}
        />
      </div>
      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Reopen previous projects on launch</span>
        <Toggle
          checked={preferences.value.reopenPreviousProjects}
          label="Reopen previous projects on launch"
          onchange={(v) => preferences.update({ reopenPreviousProjects: v })}
        />
      </div>
      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Confirm before stopping all projects</span>
        <Toggle
          checked={preferences.value.confirmBeforeStopAll}
          label="Confirm before stopping all projects"
          onchange={(v) => preferences.update({ confirmBeforeStopAll: v })}
        />
      </div>
      <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
        <span class="text-[13px] text-fg">Show desktop notifications</span>
        <Toggle
          checked={preferences.value.desktopNotifications}
          label="Show desktop notifications"
          onchange={(v) => preferences.update({ desktopNotifications: v })}
        />
      </div>
    </div>
  </section>

  <!-- ============== Appearance ============== -->
  <section
    class="bg-surface border border-border rounded-2xl p-5
           grid grid-cols-[180px,1fr] gap-x-6"
  >
    <div class="flex items-start gap-2.5">
      <span
        class="inline-flex items-center justify-center w-8 h-8 rounded-lg
               bg-fg-muted/10 text-fg-muted"
      >
        <Icon name="layers" size={15} />
      </span>
      <span class="text-[14px] font-semibold text-fg pt-1">Appearance</span>
    </div>

    <div class="divide-y divide-border/60">
      <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
        <span class="text-[13px] text-fg">Theme</span>
        <Segmented
          value={themeChoice}
          options={themeOptions}
          label="Theme"
          onchange={(v) => applyThemeChoice(v)}
        />
      </div>
      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Density</span>
        <Segmented
          value={density.value}
          options={densityOptions}
          label="Density"
          onchange={(v) => density.set(v)}
        />
      </div>
      <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
        <span class="text-[13px] text-fg">Accent color</span>
        <ColorSwatchGroup
          value={preferences.value.accentColor}
          onchange={(v: AccentColor) => preferences.update({ accentColor: v })}
        />
      </div>
    </div>
  </section>

  <!-- ============== Workspace & Projects ============== -->
  <section
    class="bg-surface border border-border rounded-2xl p-5
           grid grid-cols-[180px,1fr] gap-x-6"
  >
    <div class="flex items-start gap-2.5">
      <span
        class="inline-flex items-center justify-center w-8 h-8 rounded-lg
               bg-fg-muted/10 text-fg-muted"
      >
        <Icon name="folder" size={15} />
      </span>
      <span class="text-[14px] font-semibold text-fg pt-1">
        Workspace &amp; Projects
      </span>
    </div>

    <div class="divide-y divide-border/60">
      <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
        <span class="text-[13px] text-fg">Default workspace folder</span>
        <div class="flex items-center gap-2">
          <input
            type="text"
            value={preferences.value.defaultWorkspaceFolder}
            oninput={(e) =>
              preferences.update({
                defaultWorkspaceFolder: (e.currentTarget as HTMLInputElement)
                  .value,
              })}
            placeholder="~/Projects"
            class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg font-mono focus:outline-none focus:border-accent/60"
          />
          <button
            type="button"
            onclick={pickWorkspaceFolder}
            class="h-8 px-3 rounded-md border border-border text-[12px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
          >
            Change
          </button>
        </div>
      </div>

      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Auto-detect new projects</span>
        <Toggle
          checked={preferences.value.autoDetectProjects}
          label="Auto-detect new projects"
          onchange={(v) => preferences.update({ autoDetectProjects: v })}
        />
      </div>

      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Default sort</span>
        <select
          value={preferences.value.defaultSort}
          onchange={(e) =>
            preferences.update({
              defaultSort: (e.currentTarget as HTMLSelectElement)
                .value as DefaultSort,
            })}
          class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
        >
          {#each sortOptions as opt (opt.value)}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
      </div>

      <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
        <span class="text-[13px] text-fg">Default start behavior</span>
        <select
          value={preferences.value.defaultStartBehavior}
          onchange={(e) =>
            preferences.update({
              defaultStartBehavior: (e.currentTarget as HTMLSelectElement)
                .value as StartBehavior,
            })}
          class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
        >
          {#each startOptions as opt (opt.value)}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
      </div>
    </div>
  </section>

  <!-- ============== Domains & HTTPS ============== -->
  <section
    class="bg-surface border border-border rounded-2xl p-5
           grid grid-cols-[180px,1fr] gap-x-6"
  >
    <div class="flex items-start gap-2.5">
      <span
        class="inline-flex items-center justify-center w-8 h-8 rounded-lg
               bg-fg-muted/10 text-fg-muted"
      >
        <Icon name="globe" size={15} />
      </span>
      <span class="text-[14px] font-semibold text-fg pt-1">
        Domains &amp; HTTPS
      </span>
    </div>

    <div class="divide-y divide-border/60">
      <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
        <span class="text-[13px] text-fg">Default domain suffix</span>
        <div class="flex items-center gap-2">
          <div class="relative">
            <span
              class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle text-[12px]"
            >.</span>
            <input
              type="text"
              value={domainDraft}
              oninput={(e) =>
                (domainDraft = (e.currentTarget as HTMLInputElement).value)}
              onkeydown={(e) => {
                if (e.key === "Enter") void saveDomainSuffix();
              }}
              placeholder="test"
              class="h-8 w-44 rounded-md bg-bg border border-border pl-5 pr-2 text-[12px] text-fg font-mono focus:outline-none focus:border-accent/60"
            />
          </div>
          <button
            type="button"
            onclick={saveDomainSuffix}
            disabled={domainBusy || !domainDraft.trim() ||
              domainDraft.trim().replace(/^\./, "") ===
                domainSettings?.domainSuffix}
            class="h-8 px-3 rounded-md text-[12px] text-accent border border-accent/40 hover:bg-accent/10 transition-colors disabled:opacity-50"
          >
            {domainBusy ? "Saving…" : "Apply"}
          </button>
        </div>
      </div>

      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Manage hosts file automatically</span>
        <Toggle
          checked={preferences.value.manageHostsAutomatically}
          label="Manage hosts file automatically"
          onchange={(v) =>
            preferences.update({ manageHostsAutomatically: v })}
        />
      </div>

      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Trust local CA</span>
        <span class="inline-flex items-center gap-1.5 text-[12px]">
          <span
            class="inline-flex items-center justify-center w-4 h-4 rounded-full
                   bg-status-running/15 text-status-running"
          >
            <Icon name="check" size={10} />
          </span>
          <span class="text-status-running font-medium">Trusted</span>
        </span>
      </div>

      <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
        <span class="text-[13px] text-fg">Auto-renew local certificates</span>
        <Toggle
          checked={preferences.value.autoRenewCertificates}
          label="Auto-renew local certificates"
          onchange={(v) => preferences.update({ autoRenewCertificates: v })}
        />
      </div>
    </div>
  </section>

  <!-- ============== Artifacts ============== -->
  <section
    class="bg-surface border border-border rounded-2xl p-5
           grid grid-cols-[180px,1fr] gap-x-6"
  >
    <div class="flex items-start gap-2.5">
      <span
        class="inline-flex items-center justify-center w-8 h-8 rounded-lg
               bg-fg-muted/10 text-fg-muted"
      >
        <Icon name="package" size={15} />
      </span>
      <span class="text-[14px] font-semibold text-fg pt-1">Artifacts</span>
    </div>

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
  </section>

  <!-- ============== Advanced ============== -->
  <section
    class="bg-surface border border-border rounded-2xl p-5
           grid grid-cols-[180px,1fr] gap-x-6"
  >
    <div class="flex items-start gap-2.5">
      <span
        class="inline-flex items-center justify-center w-8 h-8 rounded-lg
               bg-fg-muted/10 text-fg-muted"
      >
        <Icon name="file-code" size={15} />
      </span>
      <span class="text-[14px] font-semibold text-fg pt-1">Advanced</span>
    </div>

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
  </section>

  <!-- Footer actions — settings persist on change, so there's no Save. -->
  <div class="flex items-center justify-between gap-3 pt-2">
    <div class="flex items-center gap-3">
      <button
        type="button"
        onclick={restoreDefaults}
        class="text-[13px] text-fg-muted hover:text-fg transition-colors"
      >
        {restoreArmed ? "Click again to confirm" : "Restore defaults"}
      </button>
    </div>

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

  <!-- ============== Advanced toggles & diagnostics (collapsed) ============== -->
  <section
    class="bg-surface border border-border rounded-2xl overflow-hidden"
  >
    <button
      type="button"
      onclick={() => (advancedDiagnosticsOpen = !advancedDiagnosticsOpen)}
      class="w-full flex items-center justify-between gap-3 px-5 py-3
             text-left hover:bg-surface-2/40 transition-colors"
      aria-expanded={advancedDiagnosticsOpen}
    >
      <span class="flex items-center gap-2.5">
        <span
          class="inline-flex items-center justify-center w-8 h-8 rounded-lg
                 bg-fg-muted/10 text-fg-muted"
        >
          <Icon name="info" size={15} />
        </span>
        <span class="text-[14px] font-semibold text-fg">
          Advanced toggles &amp; diagnostics
        </span>
      </span>
      <Icon
        name={advancedDiagnosticsOpen ? "chevron-down" : "chevron-right"}
        size={14}
        class="text-fg-subtle"
      />
    </button>

    {#if advancedDiagnosticsOpen}
      <div class="px-5 pb-5 space-y-5 border-t border-border/60 pt-4">
        <!-- Tray toggles -->
        <div>
          <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
            Menu bar
          </h3>
          <div class="divide-y divide-border/60">
            <div class="flex items-center justify-between gap-3 py-2.5">
              <div>
                <p class="text-[13px] text-fg">Show menu bar icon</p>
                <p class="text-[11.5px] text-fg-muted max-w-md">
                  Adds a status-coloured PortBay icon to the macOS menu
                  bar. Gray when idle, blue while starting, green when
                  healthy, red when something needs attention.
                </p>
              </div>
              <Toggle
                checked={preferences.value.showTrayIcon}
                label="Show menu bar icon"
                onchange={(v) =>
                  preferences.update({ showTrayIcon: v })}
              />
            </div>
            <div class="flex items-center justify-between gap-3 py-2.5">
              <div>
                <p class="text-[13px] text-fg">Close to menu bar</p>
                <p class="text-[11.5px] text-fg-muted max-w-md">
                  Closing the window keeps the app and your projects
                  running. Quit from the tray (or ⌘Q) to stop
                  everything.
                </p>
              </div>
              <Toggle
                checked={preferences.value.closeToMenuBar}
                label="Close to menu bar"
                disabled={!preferences.value.showTrayIcon}
                onchange={(v) =>
                  preferences.update({ closeToMenuBar: v })}
              />
            </div>
          </div>
        </div>

        <!-- DNS routing (kept rich because it's a system-level install) -->
        <div>
          <h3 class="text-[12px] uppercase tracking-wide text-fg-subtle mb-2">
            DNS routing
          </h3>
          {#if dnsStatus}
            <p class="text-[11.5px] text-fg-muted mb-2">
              PortBay can route every <span class="font-mono">*.{dnsStatus.suffix}</span>
              query to its local dnsmasq daemon on port
              <span class="font-mono">{dnsStatus.currentPort}</span>.
              One macOS authorisation prompt; no
              <span class="font-mono">/etc/hosts</span> edits after.
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
          <div class="flex items-center justify-between gap-3 py-2.5 border-b border-border/60">
            <div>
              <p class="text-[13px] text-fg">Send anonymous diagnostics</p>
              <p class="text-[11.5px] text-fg-muted max-w-md">
                Off by default. When on, PortBay may send OS, app
                version, command name, and success/failure. Crashes
                include panic message and sanitised backtrace only.
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
                      disabled={telemetryBusy ||
                        !preferences.value.telemetryEnabled}
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
            <p class="mt-3 text-[12px] text-fg-subtle">
              No pending crash reports.
            </p>
          {/if}
        </div>

        <!-- Import / migration -->
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
              Re-run the welcome flow to scaffold a new project or
              replay the system health check.
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
            PortBay checks for new signed releases on launch. Updates are
            verified against PortBay's public key before they install.
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
              {updater.status === "checking"
                ? "Checking…"
                : "Check for updates"}
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
              · Last checked {new Date(
                updater.lastChecked,
              ).toLocaleString()}
            {/if}
          </p>
        </div>
      </div>
    {/if}
  </section>
</div>
