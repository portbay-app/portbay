<!--
  /certificates — project-backed certificate settings.

  Certificates are not standalone vault records in PortBay: a certificate is
  always the TLS configuration for a project hostname. This page mirrors that
  model directly. The left rail lists projects and their certificate state; the
  right pane edits the same ProjectView.domain SSL fields used by /domains.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import ProjectAvatar from "$lib/components/atoms/ProjectAvatar.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { openUrl } from "$lib/security/openUrl";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import CreateCertificatePane from "$lib/components/certificates/CreateCertificatePane.svelte";
  import type { CertInfo } from "$lib/types/certs";
  import type { CommandError } from "$lib/types/error";
  import { statusLabel } from "$lib/types/status";
  import {
    defaultAcmeConfig,
    defaultDomainConfig,
    typeLabel,
    type AcmeDnsProvider,
    type AcmeEnvironment,
    type AcmeIssuer,
    type AcmeKeyType,
    type DomainConfig,
    type ProjectView,
    type SslMode,
  } from "$lib/types/projects";

  const ISSUER = "mkcert local CA";

  type CertCell = {
    info: CertInfo | null;
    error: string | null;
    loading: boolean;
  };

  interface Draft {
    https: boolean;
    autoManageCert: boolean;
    sslMode: SslMode;
    customCertPath: string;
    customKeyPath: string;
    acmeIssuer: AcmeIssuer;
    acmeEnvironment: AcmeEnvironment;
    acmeEmail: string;
    acmeKeyType: AcmeKeyType;
    acmeEabKeyId: string;
    acmeEabHmacKey: string;
    acmeZerosslApiKey: string;
    acmeDnsProvider: AcmeDnsProvider;
    acmeDnsApiToken: string;
    acmeForceRequest: boolean;
    acmeDebug: boolean;
    includeWildcardSubdomains: boolean;
  }

  let certs = $state<Record<string, CertCell>>({});
  let query = $state<string>("");
  let selectedId = $state<string | null>(null);
  let draft = $state<Draft | null>(null);
  let pristine = $state<string>("");
  let loadedFor = $state<string | null>(null);
  let saving = $state<boolean>(false);
  let reissuing = $state<string | null>(null);
  // When true the right pane shows the "New certificate" creation view instead
  // of the per-project editor.
  let adding = $state<boolean>(false);

  const rows = $derived.by<ProjectView[]>(() => {
    const list = [...projects.value].sort((a, b) => {
      if (a.https !== b.https) return a.https ? -1 : 1;
      return a.hostname.localeCompare(b.hostname);
    });
    const q = query.trim().toLowerCase();
    if (!q) return list;
    return list.filter((p) => {
      const sans = certs[p.id]?.info?.sans ?? [];
      return (
        p.hostname.toLowerCase().includes(q) ||
        p.name.toLowerCase().includes(q) ||
        (p.domain?.sslMode ?? "automatic_local").includes(q) ||
        sans.some((s) => s.toLowerCase().includes(q))
      );
    });
  });

  const selected = $derived<ProjectView | null>(
    projects.value.find((p) => p.id === selectedId) ?? null,
  );
  const selectedCell = $derived(selected ? certs[selected.id] : undefined);
  const dirty = $derived(draft !== null && JSON.stringify(draft) !== pristine);
  const total = $derived(projects.value.length);
  const configured = $derived(projects.value.filter((p) => p.https).length);
  const managed = $derived(
    projects.value.filter(
      (p) => p.https && (p.domain?.sslMode ?? "automatic_local") === "automatic_local",
    ).length,
  );
  const custom = $derived(
    projects.value.filter((p) => p.https && p.domain?.sslMode === "custom_certificate")
      .length,
  );
  const needsAttention = $derived(
    Object.values(certs).filter((c) => c.info && toneFromInfo(c.info) !== "valid").length,
  );

  onMount(() => {
    void (async () => {
      await projects.start();
      await loadAll();
    })();
  });

  $effect(() => {
    const ids = projects.value.filter((p) => p.https).map((p) => p.id).join("|");
    if (!ids) return;
    untrack(() => {
      for (const p of projects.value) {
        if (p.https && !certs[p.id]) void loadCert(p.id);
      }
    });
  });

  $effect(() => {
    if (!selectedId && rows.length > 0) {
      const first = rows[0].id;
      untrack(() => (selectedId = first));
    }
    if (selectedId && rows.length > 0 && !projects.value.some((p) => p.id === selectedId)) {
      const first = rows[0].id;
      untrack(() => (selectedId = first));
    }
  });

  $effect(() => {
    const id = selectedId;
    if (!id) {
      untrack(() => {
        draft = null;
        pristine = "";
        loadedFor = null;
      });
      return;
    }
    if (id === loadedFor) return;
    const p = projects.value.find((x) => x.id === id);
    if (p) {
      untrack(() => {
        loadDraft(p);
        loadedFor = id;
      });
    }
  });

  async function loadAll(): Promise<void> {
    await Promise.all(projects.value.filter((p) => p.https).map((p) => loadCert(p.id)));
  }

  async function loadCert(id: string): Promise<void> {
    certs[id] = { info: certs[id]?.info ?? null, error: null, loading: true };
    try {
      const info = await safeInvoke<CertInfo>("cert_info", { id });
      certs[id] = { info, error: null, loading: false };
    } catch (e) {
      const err = e as CommandError | undefined;
      const message =
        err && err.code !== "PROJECT_NOT_FOUND" ? err.whatHappened : null;
      certs[id] = { info: null, error: message, loading: false };
    }
  }

  function loadDraft(p: ProjectView) {
    const d = p.domain ?? defaultDomainConfig();
    const acme = d.acme ?? defaultAcmeConfig();
    draft = {
      https: p.https,
      autoManageCert: d.autoManageCert ?? true,
      sslMode: d.sslMode ?? "automatic_local",
      customCertPath: d.customCertPath ?? "",
      customKeyPath: d.customKeyPath ?? "",
      acmeIssuer: acme.issuer ?? "lets_encrypt",
      acmeEnvironment: acme.environment ?? "production",
      acmeEmail: acme.email ?? "",
      acmeKeyType: acme.keyType ?? "p384",
      acmeEabKeyId: acme.eabKeyId ?? "",
      acmeEabHmacKey: acme.eabHmacKey ?? "",
      acmeZerosslApiKey: acme.zerosslApiKey ?? "",
      acmeDnsProvider: acme.dnsProvider ?? "none",
      acmeDnsApiToken: acme.dnsApiToken ?? "",
      acmeForceRequest: acme.forceRequest ?? false,
      acmeDebug: acme.debug ?? false,
      includeWildcardSubdomains: d.includeWildcardSubdomains ?? false,
    };
    pristine = JSON.stringify(draft);
  }

  function buildDomain(base: DomainConfig | null | undefined, d: Draft): DomainConfig {
    const current = base ?? defaultDomainConfig();
    return {
      ...current,
      autoManageCert: d.https && d.sslMode === "automatic_local" ? d.autoManageCert : false,
      sslMode: d.sslMode,
      customCertPath: d.customCertPath.trim() ? d.customCertPath.trim() : null,
      customKeyPath: d.customKeyPath.trim() ? d.customKeyPath.trim() : null,
      acme:
        d.sslMode === "public_acme"
          ? {
              issuer: d.acmeIssuer,
              environment: d.acmeEnvironment,
              email: d.acmeEmail.trim() ? d.acmeEmail.trim() : null,
              keyType: d.acmeKeyType,
              eabKeyId: d.acmeEabKeyId.trim() ? d.acmeEabKeyId.trim() : null,
              eabHmacKey: d.acmeEabHmacKey.trim() ? d.acmeEabHmacKey.trim() : null,
              zerosslApiKey: d.acmeZerosslApiKey.trim()
                ? d.acmeZerosslApiKey.trim()
                : null,
              dnsProvider: d.acmeDnsProvider,
              dnsApiToken: d.acmeDnsApiToken.trim() ? d.acmeDnsApiToken.trim() : null,
              forceRequest: d.acmeForceRequest,
              debug: d.acmeDebug,
            }
          : null,
      includeWildcardSubdomains: d.includeWildcardSubdomains,
    };
  }

  async function save(): Promise<void> {
    if (!selected || !draft || !dirty || saving) return;
    saving = true;
    const target = selected.id;
    const wildcardWas = selected.domain?.includeWildcardSubdomains ?? false;
    const modeWas = selected.domain?.sslMode ?? "automatic_local";
    try {
      const domain = buildDomain(selected.domain, draft);
      await safeInvoke("update_project", {
        id: target,
        patch: { https: draft.https, domain },
      });

      if (
        draft.https &&
        draft.sslMode === "automatic_local" &&
        (draft.includeWildcardSubdomains && !wildcardWas || modeWas !== "automatic_local")
      ) {
        try {
          await safeInvoke("reissue_cert", { id: target });
        } catch {
          /* non-fatal; cert_info will surface the state */
        }
      }

      await projects.refresh();
      await loadCert(target);
      const fresh = projects.value.find((p) => p.id === target);
      if (fresh) {
        loadDraft(fresh);
        loadedFor = target;
      }
      errorBus.push({
        code: "CERTIFICATE_SAVED",
        category: "infrastructure",
        whatHappened: `${selected.hostname} certificate settings saved.`,
        whyItMatters: draft.https
          ? "Caddy will serve this hostname with the selected TLS source."
          : "HTTPS is disabled for this project.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* safeInvoke already pushed a toast */
    } finally {
      saving = false;
    }
  }

  function revert() {
    if (selected) loadDraft(selected);
  }

  async function reissue(p: ProjectView): Promise<void> {
    if (reissuing || !canReissue(p)) return;
    reissuing = p.id;
    try {
      await safeInvoke("reissue_cert", { id: p.id });
      await new Promise((r) => setTimeout(r, 400));
      await loadCert(p.id);
      errorBus.push({
        code: "REISSUE_OK",
        category: "infrastructure",
        whatHappened: `Cert reissued for ${p.name}.`,
        whyItMatters: "Caddy reloaded the cert; refresh your browser tab.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* safeInvoke already pushed a toast */
    } finally {
      reissuing = null;
    }
  }

  async function revealCertFolder(p: ProjectView): Promise<void> {
    const path = certs[p.id]?.info?.certificatePath;
    if (!path) return;
    const dir = path.replace(/\/cert\.pem$/, "");
    try {
      await openUrl(`file://${dir}`);
    } catch {
      /* opener pushes its own toast */
    }
  }

  function openSite(p: ProjectView): void {
    void openUrl(p.url);
  }

  function openProject(p: ProjectView): void {
    projectDetailPanel.show(p.id);
  }

  function addCertificate(): void {
    // Swap the right pane into the "New certificate" creation view rather than
    // jumping the editor into another project.
    adding = true;
  }

  async function exportCert(p: ProjectView): Promise<void> {
    if (!certs[p.id]?.info) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const dir = await open({
        directory: true,
        title: `Export ${p.hostname} certificate to…`,
      });
      if (typeof dir !== "string") return;
      const written = await safeInvoke<string>("export_cert_bundle", {
        id: p.id,
        destDir: dir,
      });
      errorBus.push({
        code: "CERT_EXPORTED",
        category: "infrastructure",
        whatHappened: `Exported ${p.hostname} certificate to ${written}`,
        whyItMatters:
          "The folder has cert.pem, key.pem, rootCA.pem, and a README with install steps.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* dialog cancelled or safeInvoke already pushed a toast */
    }
  }

  function canReissue(p: ProjectView): boolean {
    return p.https && (p.domain?.sslMode ?? "automatic_local") === "automatic_local";
  }

  function certModeLabel(p: ProjectView): string {
    switch (p.domain?.sslMode ?? "automatic_local") {
      case "automatic_local":
        return "Automatic local";
      case "custom_certificate":
        return "Custom";
      case "self_signed":
        return "Self-signed";
      case "public_acme":
        return "Public ACME";
    }
  }

  function fmtDate(iso: string | null): string {
    if (!iso) return "—";
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleDateString(undefined, {
      day: "numeric",
      month: "short",
      year: "numeric",
    });
  }

  type Tone = "valid" | "expiring" | "missing" | "error" | "disabled";

  function toneFromInfo(info: CertInfo | null | undefined): Tone {
    if (!info) return "missing";
    if (
      info.status === "missingCa" ||
      info.status === "untrusted" ||
      info.status === "expired" ||
      info.status === "error"
    ) {
      return "error";
    }
    if (info.status === "regenerateNeeded") return "expiring";
    const days = info.daysUntilExpiry;
    if (days !== null && days < 30) return "expiring";
    return "valid";
  }

  function tone(p: ProjectView): Tone {
    if (!p.https) return "disabled";
    return toneFromInfo(certs[p.id]?.info);
  }

  const toneDot: Record<Tone, string> = {
    valid: "bg-status-running",
    expiring: "bg-status-unhealthy",
    missing: "bg-fg-subtle/60",
    error: "bg-status-crashed",
    disabled: "bg-fg-subtle/35",
  };

  function statusText(p: ProjectView, info: CertInfo | null | undefined): string {
    if (!p.https) return "HTTPS off";
    if (!info) return "Not issued";
    switch (info.status) {
      case "ready": {
        if (info.trustStoreVerified === false) return "Unverified";
        const days = info.daysUntilExpiry;
        if (days !== null) {
          if (days <= 0) return "Expired";
          if (days === 1) return "Expires tomorrow";
          if (days < 30) return `Expires in ${days} days`;
        }
        return "Active";
      }
      case "missingCa":
        return "Missing CA";
      case "expired":
        return "Expired";
      case "untrusted":
        return "Untrusted";
      case "regenerateNeeded":
        return "Regenerate needed";
      case "error":
        return "Error";
    }
  }

  const sslModeOptions: { value: SslMode; label: string }[] = [
    { value: "automatic_local", label: "Automatic local HTTPS" },
    { value: "custom_certificate", label: "Custom certificate" },
    { value: "self_signed", label: "Self-signed fallback" },
    { value: "public_acme", label: "Public ACME / AutoSSL" },
  ];

  const acmeIssuerOptions: { value: AcmeIssuer; label: string }[] = [
    { value: "lets_encrypt", label: "Let's Encrypt" },
    { value: "zero_ssl", label: "ZeroSSL" },
    { value: "google_trust_services", label: "Google Trust Services" },
  ];
  const acmeEnvironmentOptions: { value: AcmeEnvironment; label: string }[] = [
    { value: "production", label: "Production" },
    { value: "staging", label: "Staging" },
  ];
  const acmeKeyTypeOptions: { value: AcmeKeyType; label: string }[] = [
    { value: "p384", label: "ECC P-384" },
    { value: "p256", label: "ECC P-256" },
    { value: "rsa2048", label: "RSA 2048" },
    { value: "rsa4096", label: "RSA 4096" },
  ];
  const dnsProviderOptions: { value: AcmeDnsProvider; label: string }[] = [
    { value: "none", label: "HTTP/TLS challenge" },
    { value: "cloudflare", label: "Cloudflare" },
  ];
</script>

{#snippet toggle(on: boolean, flip: () => void, label: string, disabled = false)}
  <button
    type="button"
    role="switch"
    aria-checked={on}
    aria-label={label}
    disabled={disabled}
    onclick={flip}
    class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full
           transition-colors active:scale-95 focus-visible:outline-none
           focus-visible:ring-2 focus-visible:ring-accent/40 disabled:opacity-45
           disabled:cursor-not-allowed
           {on ? 'bg-accent' : 'bg-surface-2 border border-border'}"
  >
    <span
      class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm
             transition-transform duration-150
             {on ? 'translate-x-[18px]' : 'translate-x-0.5'}"
    ></span>
  </button>
{/snippet}

{#snippet toggleRow(label: string, help: string, on: boolean, flip: () => void, disabled = false)}
  <div class="flex items-start justify-between gap-4 py-2">
    <div class="min-w-0">
      <div class="text-[12.5px] text-fg">{label}</div>
      <div class="mt-0.5 text-[11px] text-fg-subtle leading-relaxed">{help}</div>
    </div>
    {@render toggle(on, flip, label, disabled)}
  </div>
{/snippet}

{#snippet projectRow(p: ProjectView)}
  {@const cell = certs[p.id]}
  {@const isActive = selectedId === p.id}
  <button
    type="button"
    onclick={() => {
      selectedId = p.id;
      adding = false;
    }}
    aria-current={isActive ? "true" : undefined}
    class="w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg text-left
           transition-colors cursor-pointer focus-visible:outline-none
           focus-visible:ring-2 focus-visible:ring-accent/40
           {isActive
      ? 'bg-accent/10 ring-1 ring-inset ring-accent/40'
      : 'hover:bg-surface-2/60'}"
  >
    <ProjectAvatar id={p.id} name={p.name} type={p.type} size={28} />
    <span class="min-w-0 flex-1 leading-tight">
      <span class="block text-[12.5px] font-semibold text-fg truncate">
        {p.name}
      </span>
      <span class="block text-[11.5px] font-mono text-fg-subtle truncate">
        {p.hostname}
      </span>
      <span class="block text-[10.5px] text-fg-subtle truncate">
        {statusText(p, cell?.info)} · {certModeLabel(p)}
      </span>
    </span>
    <span class="shrink-0 text-right leading-tight">
      <span
        class="inline-flex items-center rounded px-1.5 py-0.5 text-[10.5px]
               bg-surface-2 text-fg-subtle"
      >
        {p.https ? "HTTPS" : "Off"}
      </span>
      <span class="mt-1 flex items-center justify-end gap-1">
        <StatusDot status={p.status} size="sm" />
        <span class="text-[10.5px] text-fg-subtle">{statusLabel[p.status]}</span>
      </span>
    </span>
    <Icon name="chevron-right" size={13} class="text-fg-subtle shrink-0" />
  </button>
{/snippet}

<div class="h-full flex flex-col">
  <header
    class="shrink-0 flex items-center gap-6 flex-wrap px-6 py-4 border-b border-border"
  >
    <div class="min-w-0 flex-1">
      <h1 class="text-[20px] font-semibold text-fg leading-none">
        SSL Certificates
      </h1>
      <p class="mt-1.5 text-[12.5px] text-fg-subtle leading-relaxed">
        Create and edit the TLS settings attached to each project hostname.
      </p>
    </div>

    <dl class="flex items-stretch gap-2.5">
      {#snippet stat(icon: import("$lib/components/atoms/Icon.svelte").IconName, tint: string, label: string, value: number)}
        <div
          class="flex items-center gap-2.5 rounded-xl border border-border
                 bg-surface px-3.5 py-2.5 min-w-[116px]"
        >
          <span class="grid place-items-center w-8 h-8 rounded-lg bg-surface-2 {tint}">
            <Icon name={icon} size={16} />
          </span>
          <div class="leading-tight">
            <dt class="text-[11px] text-fg-subtle whitespace-nowrap">{label}</dt>
            <dd class="text-[17px] font-semibold tabular-nums text-fg">{value}</dd>
          </div>
        </div>
      {/snippet}
      {@render stat("shield", "text-status-running", "Configured", configured)}
      {@render stat("rotate-cw", "text-fg-muted", "Managed", managed)}
      {@render stat("key", "text-accent", "Custom", custom)}
      {@render stat(
        "circle-alert",
        needsAttention > 0 ? "text-status-unhealthy" : "text-fg-subtle",
        "Attention",
        needsAttention,
      )}
    </dl>
  </header>

  <div class="flex-1 min-h-0 flex">
    <aside
      class="w-[336px] shrink-0 border-r border-border bg-surface/40 flex flex-col"
      aria-label="Certificates"
    >
      <div class="shrink-0 p-3 space-y-2.5 border-b border-border/60">
        <button
          type="button"
          onclick={addCertificate}
          class="w-full inline-flex items-center justify-center gap-1.5 h-9 rounded-lg
                 text-[13px] font-medium bg-accent text-on-accent
                 hover:brightness-110 active:scale-[0.99] transition"
        >
          <Icon name="plus" size={15} />
          Add Certificate
        </button>
        <label class="relative flex items-center" aria-label="Filter certificates">
          <Icon
            name="search"
            size={14}
            class="absolute left-2.5 text-fg-subtle pointer-events-none"
          />
          <input
            type="search"
            bind:value={query}
            placeholder="Filter certificates…"
            class="h-8 w-full pl-8 pr-3 rounded-lg bg-surface border border-border
                   text-[12.5px] text-fg placeholder:text-fg-subtle
                   focus:outline-none focus:ring-2 focus:ring-accent/40"
          />
        </label>
      </div>

      <div class="flex-1 min-h-0 overflow-y-auto p-2 space-y-1">
        {#if rows.length === 0}
          <p class="px-2 py-8 text-center text-[12.5px] text-fg-subtle">
            {#if total === 0}
              No projects yet. Add a project to create its first certificate.
            {:else}
              No certificates match “{query}”.
            {/if}
          </p>
        {:else}
          {#each rows as p (p.id)}
            {@render projectRow(p)}
          {/each}
        {/if}
      </div>

      {#if rows.length > 0}
        <div
          class="shrink-0 flex items-center gap-2 px-3 py-2.5 border-t border-border/60"
        >
          <span class="text-[11px] text-fg-subtle">
            {configured} configured of {total} project{total === 1 ? "" : "s"}
          </span>
        </div>
      {/if}
    </aside>

    <section class="flex-1 min-w-0 overflow-y-auto">
      {#if adding}
        <CreateCertificatePane
          presetProjectId={selectedId}
          onClose={(id) => {
            adding = false;
            if (id) selectedId = id;
          }}
        />
      {:else if !selected || !draft}
        <div class="h-full grid place-items-center">
          <div class="text-center max-w-xs px-6">
            <span
              class="inline-grid place-items-center w-12 h-12 rounded-xl bg-surface-2 text-fg-subtle mx-auto"
            >
              <Icon name="shield" size={24} />
            </span>
            <p class="mt-3 text-[13px] text-fg-muted">
              Select a project to inspect its certificate and edit the TLS source.
            </p>
          </div>
        </div>
      {:else}
        {@const d = draft}
        {@const cell = selectedCell}
        {@const info = cell?.info}
        {@const t = tone(selected)}
        <div class="px-6 py-6">
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <h2 class="text-[15px] font-semibold text-fg">Certificate Settings</h2>
              <p class="mt-1 text-[12px] text-fg-subtle">
                <code class="font-mono">{selected.hostname}</code> · {selected.name}
              </p>
            </div>
            <span
              class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1
                     text-[11.5px] bg-surface-2 text-fg-muted"
            >
              <span class="inline-block w-2 h-2 rounded-full {toneDot[t]}"></span>
              {statusText(selected, info)}
            </span>
          </div>

          <div class="mt-5 grid grid-cols-1 xl:grid-cols-[minmax(0,1fr)_300px] gap-5">
            <div class="space-y-5">
              <div class="rounded-xl border border-border bg-surface overflow-hidden">
                <div class="px-4 py-3 border-b border-border/60">
                  <h3 class="text-[12.5px] font-semibold text-fg">TLS Source</h3>
                </div>
                <div class="p-4 space-y-4">
                  <div class="rounded-xl border border-border divide-y divide-border/60 px-4">
                    {@render toggleRow(
                      "Enable HTTPS",
                      "Create and serve a certificate for this project hostname.",
                      d.https,
                      () => {
                        d.https = !d.https;
                        if (d.https && d.sslMode === "automatic_local") d.autoManageCert = true;
                      },
                    )}
                  </div>

                  <div class="space-y-1.5">
                    <label for="cert-ssl-mode" class="text-[12px] font-medium text-fg-muted">
                      SSL mode
                    </label>
                    <select
                      id="cert-ssl-mode"
                      bind:value={d.sslMode}
                      onchange={() => {
                        d.autoManageCert = d.https && d.sslMode === "automatic_local";
                      }}
                      disabled={!d.https}
                      class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                             text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                             disabled:opacity-50"
                    >
                      {#each sslModeOptions as o (o.value)}
                        <option value={o.value}>{o.label}</option>
                      {/each}
                    </select>
                    <p class="text-[11px] text-fg-subtle leading-relaxed">
                      {#if !d.https}
                        Turn on HTTPS to create a certificate for this project.
                      {:else if d.sslMode === "automatic_local"}
                        PortBay issues a locally trusted mkcert certificate and keeps
                        its SANs aligned with this hostname.
                      {:else if d.sslMode === "custom_certificate"}
                        Use a company or hand-issued certificate. The cert and key must
                        match and cover this hostname.
                      {:else if d.sslMode === "self_signed"}
                        Intended only as a fallback. Browsers will show warnings until
                        you switch back to a trusted mode.
                      {:else}
                        Reserved for public domains only; local <code class="font-mono">.test</code>
                        names are not eligible for public ACME.
                      {/if}
                    </p>
                  </div>

                  {#if d.sslMode === "custom_certificate"}
                    <div class="grid grid-cols-1 gap-3">
                      <div class="space-y-1.5">
                        <label for="cert-custom-cert" class="text-[12px] font-medium text-fg-muted">
                          Certificate path
                        </label>
                        <input
                          id="cert-custom-cert"
                          bind:value={d.customCertPath}
                          disabled={!d.https}
                          placeholder="/absolute/path/cert.pem"
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                                 focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        />
                      </div>
                      <div class="space-y-1.5">
                        <label for="cert-custom-key" class="text-[12px] font-medium text-fg-muted">
                          Private key path
                        </label>
                        <input
                          id="cert-custom-key"
                          bind:value={d.customKeyPath}
                          disabled={!d.https}
                          placeholder="/absolute/path/key.pem"
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                                 focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        />
                      </div>
                    </div>
                  {/if}

                  {#if d.sslMode === "public_acme"}
                    <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
                      <div class="space-y-1.5">
                        <label for="cert-acme-issuer" class="text-[12px] font-medium text-fg-muted">
                          Issuer
                        </label>
                        <select
                          id="cert-acme-issuer"
                          bind:value={d.acmeIssuer}
                          disabled={!d.https}
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                                 text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        >
                          {#each acmeIssuerOptions as o (o.value)}
                            <option value={o.value}>{o.label}</option>
                          {/each}
                        </select>
                      </div>
                      <div class="space-y-1.5">
                        <label for="cert-acme-env" class="text-[12px] font-medium text-fg-muted">
                          Environment
                        </label>
                        <select
                          id="cert-acme-env"
                          bind:value={d.acmeEnvironment}
                          disabled={!d.https}
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                                 text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        >
                          {#each acmeEnvironmentOptions as o (o.value)}
                            <option value={o.value}>{o.label}</option>
                          {/each}
                        </select>
                      </div>
                      <div class="space-y-1.5">
                        <label for="cert-acme-email" class="text-[12px] font-medium text-fg-muted">
                          Account email
                        </label>
                        <input
                          id="cert-acme-email"
                          type="email"
                          bind:value={d.acmeEmail}
                          disabled={!d.https}
                          placeholder="admin@example.com"
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                                 text-[13px] text-fg placeholder:text-fg-subtle
                                 focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        />
                      </div>
                      <div class="space-y-1.5">
                        <label for="cert-acme-key" class="text-[12px] font-medium text-fg-muted">
                          Algorithm
                        </label>
                        <select
                          id="cert-acme-key"
                          bind:value={d.acmeKeyType}
                          disabled={!d.https}
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                                 text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        >
                          {#each acmeKeyTypeOptions as o (o.value)}
                            <option value={o.value}>{o.label}</option>
                          {/each}
                        </select>
                      </div>
                      <div class="space-y-1.5">
                        <label for="cert-acme-dns-provider" class="text-[12px] font-medium text-fg-muted">
                          DNS API provider
                        </label>
                        <select
                          id="cert-acme-dns-provider"
                          bind:value={d.acmeDnsProvider}
                          disabled={!d.https}
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                                 text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        >
                          {#each dnsProviderOptions as o (o.value)}
                            <option value={o.value}>{o.label}</option>
                          {/each}
                        </select>
                      </div>
                      <div class="rounded-xl border border-border divide-y divide-border/60 px-4">
                        {@render toggleRow(
                          "Enable debug",
                          "Emit extra ACME diagnostics from Caddy.",
                          d.acmeDebug,
                          () => (d.acmeDebug = !d.acmeDebug),
                          !d.https,
                        )}
                        {@render toggleRow(
                          "Force request",
                          "Force Caddy to attempt issuance again on the next reload.",
                          d.acmeForceRequest,
                          () => (d.acmeForceRequest = !d.acmeForceRequest),
                          !d.https,
                        )}
                      </div>
                    </div>

                    {#if d.acmeIssuer === "zero_ssl"}
                      <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
                        <input
                          aria-label="ZeroSSL API key"
                          bind:value={d.acmeZerosslApiKey}
                          disabled={!d.https}
                          placeholder="ZeroSSL API key"
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                                 focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        />
                        <p class="text-[11px] text-fg-subtle leading-relaxed">
                          ZeroSSL can use its Caddy issuer API key, or EAB key id and
                          HMAC below.
                        </p>
                      </div>
                    {/if}

                    {#if d.acmeIssuer === "zero_ssl" || d.acmeIssuer === "google_trust_services"}
                      <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
                        <input
                          aria-label="ACME EAB key id"
                          bind:value={d.acmeEabKeyId}
                          disabled={!d.https}
                          placeholder="EAB key id"
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                                 focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        />
                        <input
                          aria-label="ACME EAB HMAC key"
                          type="password"
                          bind:value={d.acmeEabHmacKey}
                          disabled={!d.https}
                          placeholder="EAB HMAC key"
                          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                                 focus:outline-none focus:ring-2 focus:ring-accent/40
                                 disabled:opacity-50"
                        />
                      </div>
                    {/if}

                    {#if d.acmeDnsProvider === "cloudflare"}
                      <textarea
                        aria-label="Cloudflare DNS API token"
                        bind:value={d.acmeDnsApiToken}
                        disabled={!d.https}
                        rows="3"
                        placeholder="Cloudflare API token with Zone:DNS:Edit for this domain"
                        class="w-full px-3 py-2 rounded-lg bg-bg border border-border
                               text-[13px] text-fg font-mono placeholder:text-fg-subtle
                               focus:outline-none focus:ring-2 focus:ring-accent/40
                               disabled:opacity-50"
                      ></textarea>
                      <p class="text-[11px] text-fg-subtle leading-relaxed -mt-2">
                        Use a scoped token with Zone:Zone:Read and Zone:DNS:Edit for
                        this zone. Wildcard public certificates use Cloudflare DNS-01.
                      </p>
                    {/if}
                  {/if}

                  <div class="rounded-xl border border-border divide-y divide-border/60 px-4">
                    {@render toggleRow(
                      "Auto-manage certificate",
                      "PortBay issues and renews this hostname's local certificate.",
                      d.autoManageCert,
                      () => (d.autoManageCert = !d.autoManageCert),
                      !d.https || d.sslMode !== "automatic_local",
                    )}
                    {@render toggleRow(
                      "Include wildcard subdomains",
                      "Also route and certify *." + selected.hostname + ".",
                      d.includeWildcardSubdomains,
                      () => (d.includeWildcardSubdomains = !d.includeWildcardSubdomains),
                      !d.https,
                    )}
                  </div>

                  {#if d.https && d.sslMode === "automatic_local" && !d.autoManageCert}
                    <p
                      class="flex items-start gap-2 text-[11.5px] text-status-unhealthy
                             bg-status-unhealthy/10 rounded-lg px-3 py-2 leading-relaxed"
                    >
                      <Icon name="circle-alert" size={13} class="mt-px shrink-0" />
                      HTTPS is on but certificate auto-management is off. Re-enable it,
                      or provide another TLS source.
                    </p>
                  {/if}
                  {#if d.includeWildcardSubdomains}
                    <p class="text-[11px] text-fg-subtle leading-relaxed -mt-2">
                      Subdomains resolve only under the dnsmasq wildcard resolver.
                    </p>
                  {/if}
                </div>
              </div>
            </div>

            <aside class="space-y-4">
              <div class="rounded-xl border border-border bg-surface overflow-hidden">
                <div class="px-4 py-3 border-b border-border/60">
                  <h3 class="text-[12.5px] font-semibold text-fg">Issued Certificate</h3>
                </div>
                <div class="p-4 space-y-3">
                  {#if cell?.loading && !info}
                    <p class="text-[12px] text-fg-subtle">Loading certificate metadata…</p>
                  {:else if info}
                    <dl class="space-y-3">
                      <div>
                        <dt class="text-[10.5px] uppercase tracking-wide text-fg-subtle">
                          Issuer
                        </dt>
                        <dd class="mt-1 text-[12.5px] text-fg">{ISSUER}</dd>
                      </div>
                      <div>
                        <dt class="text-[10.5px] uppercase tracking-wide text-fg-subtle">
                          Validity
                        </dt>
                        <dd class="mt-1 text-[12.5px] text-fg">
                          {fmtDate(info.issuedAt)} → {fmtDate(info.expiresAt)}
                        </dd>
                        {#if info.daysUntilExpiry !== null}
                          <dd class="text-[11px] text-fg-subtle">
                            {info.daysUntilExpiry} day{info.daysUntilExpiry === 1 ? "" : "s"} left
                          </dd>
                        {/if}
                      </div>
                      <div>
                        <dt class="text-[10.5px] uppercase tracking-wide text-fg-subtle">
                          SANs
                        </dt>
                        <dd class="mt-1 text-[11.5px] text-fg-muted font-mono break-all">
                          {info.sans.length > 0 ? info.sans.join(", ") : "—"}
                        </dd>
                      </div>
                      <div>
                        <dt class="text-[10.5px] uppercase tracking-wide text-fg-subtle">
                          Certificate path
                        </dt>
                        <dd class="mt-1 text-[11.5px] text-fg-muted font-mono break-all">
                          {info.certificatePath}
                        </dd>
                      </div>
                    </dl>
                    {#if info.errors.length > 0}
                      <p
                        class="text-[11.5px] text-status-crashed bg-status-crashed/10
                               rounded-lg px-3 py-2 leading-relaxed"
                      >
                        {info.errors[0]}
                      </p>
                    {/if}
                  {:else if selected.https}
                    <p class="text-[12px] text-fg-subtle leading-relaxed">
                      No certificate has been issued yet. The reconciler creates one
                      after these settings are saved and the project syncs.
                    </p>
                    {#if cell?.error}
                      <p class="text-[11.5px] text-status-crashed leading-relaxed">
                        {cell.error}
                      </p>
                    {/if}
                  {:else}
                    <p class="text-[12px] text-fg-subtle leading-relaxed">
                      HTTPS is disabled. Turn it on and save to create this
                      project's certificate settings.
                    </p>
                  {/if}
                </div>
              </div>

              <div class="rounded-xl border border-border bg-surface p-3 space-y-2">
                <button
                  type="button"
                  onclick={() => reissue(selected)}
                  disabled={!canReissue(selected) || reissuing === selected.id}
                  class="w-full inline-flex items-center justify-center gap-1.5 h-8
                         rounded-lg text-[12px] font-medium bg-surface-2 text-fg-muted
                         hover:text-fg hover:bg-surface-2/80 transition-colors
                         disabled:opacity-45 disabled:cursor-not-allowed"
                >
                  <Icon
                    name="rotate-cw"
                    size={13}
                    class={reissuing === selected.id ? "animate-spin" : ""}
                  />
                  Reissue Local Cert
                </button>
                <button
                  type="button"
                  onclick={() => revealCertFolder(selected)}
                  disabled={!info}
                  class="w-full inline-flex items-center justify-center gap-1.5 h-8
                         rounded-lg text-[12px] font-medium bg-surface-2 text-fg-muted
                         hover:text-fg hover:bg-surface-2/80 transition-colors
                         disabled:opacity-45 disabled:cursor-not-allowed"
                >
                  <Icon name="folder" size={13} />
                  Reveal PEM Folder
                </button>
                <button
                  type="button"
                  onclick={() => exportCert(selected)}
                  disabled={!info}
                  title="Copy cert.pem, key.pem, and the CA root to a folder for use on another machine or server"
                  class="w-full inline-flex items-center justify-center gap-1.5 h-8
                         rounded-lg text-[12px] font-medium bg-surface-2 text-fg-muted
                         hover:text-fg hover:bg-surface-2/80 transition-colors
                         disabled:opacity-45 disabled:cursor-not-allowed"
                >
                  <Icon name="download" size={13} />
                  Export Certificate…
                </button>
                <button
                  type="button"
                  onclick={() => openSite(selected)}
                  class="w-full inline-flex items-center justify-center gap-1.5 h-8
                         rounded-lg text-[12px] font-medium bg-surface-2 text-fg-muted
                         hover:text-fg hover:bg-surface-2/80 transition-colors"
                >
                  <Icon name="external-link" size={13} />
                  Open Site
                </button>
                <button
                  type="button"
                  onclick={() => openProject(selected)}
                  class="w-full inline-flex items-center justify-center gap-1.5 h-8
                         rounded-lg text-[12px] font-medium bg-surface-2 text-fg-muted
                         hover:text-fg hover:bg-surface-2/80 transition-colors"
                >
                  <Icon name="package" size={13} />
                  Project Details
                </button>
              </div>
            </aside>
          </div>

          <div class="mt-6 flex items-center gap-2 pt-4 border-t border-border">
            {#if dirty}
              <button
                type="button"
                onclick={revert}
                disabled={saving}
                class="h-9 px-3 rounded-lg text-[12.5px] text-fg-muted hover:bg-surface-2
                       transition-colors disabled:opacity-50"
              >
                Revert
              </button>
            {/if}
            <button
              type="button"
              onclick={save}
              disabled={!dirty || saving}
              class="ml-auto inline-flex items-center gap-1.5 h-9 px-4 rounded-lg
                     text-[12.5px] font-medium bg-accent text-on-accent hover:brightness-110
                     active:scale-[0.98] transition disabled:opacity-50
                     disabled:cursor-not-allowed"
            >
              <Icon
                name={saving ? "refresh-cw" : "check"}
                size={13}
                class={saving ? "animate-spin" : ""}
              />
              {saving ? "Saving…" : "Save Certificate"}
            </button>
          </div>
        </div>
      {/if}
    </section>
  </div>
</div>
