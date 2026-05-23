<!--
  Settings — Phase 2 deliverable is intentionally minimal: density toggle
  and the version line. Full settings (registry path override, sidecar
  versions, log location, etc.) come in Phase 3.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { DashboardCard } from "$lib/components/atoms";
  import ImportSection from "$lib/components/imports/ImportSection.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { density, type Density } from "$lib/stores/density.svelte";
  import { theme, type Theme } from "$lib/stores/theme.svelte";
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

  interface MailStatusInfo {
    running: boolean;
    smtpPort: number | null;
    uiPort: number | null;
  }
  let mailInfo = $state<MailStatusInfo>({ running: false, smtpPort: null, uiPort: null });

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

  onMount(() => {
    void refreshDnsStatus();
    void refreshMailStatus();
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
      <dt class="text-fg-muted">Version</dt>
      <dd class="text-fg font-mono">0.1.0</dd>
      <dt class="text-fg-muted">Phase</dt>
      <dd class="text-fg">2 (GUI MVP, in progress)</dd>
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
