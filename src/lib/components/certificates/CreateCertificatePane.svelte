<!--
  CreateCertificatePane — the "New certificate" view rendered inside the
  Certificates page's right pane (it replaces the per-project editor while
  adding, rather than opening a separate side panel).

  Pick which project to attach a cert to, configure the SSL source, then create
  it. A certificate is the TLS config on a project, so "create" writes through
  `update_project` and (for automatic-local) forces issuance via `reissue_cert`,
  then verifies the issued cert covers the hostname.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import ProjectSelector from "$lib/components/shared/ProjectSelector.svelte";
  import TlsSourceFields from "$lib/components/certificates/TlsSourceFields.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import {
    buildDomainFromTls,
    tlsDraftFromDomain,
    type TlsDraft,
  } from "$lib/certs/tlsDraft";
  import type { CertInfo } from "$lib/types/certs";
  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";

  let {
    presetProjectId = null,
    onClose,
  }: {
    presetProjectId?: string | null;
    onClose: (createdId?: string) => void;
  } = $props();

  type Result =
    | { kind: "issued"; info: CertInfo; covered: boolean }
    | { kind: "pending"; reason: string }
    | { kind: "error"; message: string };

  let projectId = $state<string | null>(null);
  let draft = $state<TlsDraft | null>(null);
  let loadedFor = $state<string | null>(null);
  let submitting = $state<boolean>(false);
  let result = $state<Result | null>(null);

  const options = $derived(
    [...projects.value].sort((a, b) => a.hostname.localeCompare(b.hostname)),
  );
  const selected = $derived<ProjectView | null>(
    projects.value.find((p) => p.id === projectId) ?? null,
  );
  const hostname = $derived(selected?.hostname ?? "");

  onMount(() => {
    void projects.start();
  });

  // Pick the initial project: the preset → else the first project.
  $effect(() => {
    if (projectId !== null) return;
    const first = untrack(() => options[0]?.id ?? null);
    const next =
      presetProjectId && projects.value.some((p) => p.id === presetProjectId)
        ? presetProjectId
        : first;
    if (next) untrack(() => (projectId = next));
  });

  // Seed the draft from the chosen project whenever the selection changes.
  $effect(() => {
    const id = projectId;
    if (!id || id === loadedFor) return;
    const p = projects.value.find((x) => x.id === id);
    if (!p) return;
    untrack(() => {
      // Always default to HTTPS on — the point of this view is to add a cert.
      draft = tlsDraftFromDomain(p.domain, true);
      if (draft.sslMode === "automatic_local") draft.autoManageCert = true;
      loadedFor = id;
      result = null;
    });
  });

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

  async function create(): Promise<void> {
    if (!selected || !draft || submitting || !draft.https) return;
    submitting = true;
    const target = selected.id;
    const host = selected.hostname;
    try {
      const domain = buildDomainFromTls(selected.domain, draft);
      await safeInvoke("update_project", {
        id: target,
        patch: { https: draft.https, domain },
      });

      // Automatic-local certs are issued by the reconciler; force a tick now so
      // the confirmation below can read a freshly-issued cert.
      if (draft.sslMode === "automatic_local") {
        try {
          await safeInvoke("reissue_cert", { id: target });
        } catch {
          /* non-fatal — cert_info surfaces the real state below */
        }
      }

      await projects.refresh();

      errorBus.push({
        code: "CERTIFICATE_SAVED",
        category: "infrastructure",
        whatHappened: `${host} certificate created.`,
        whyItMatters:
          "Caddy will serve this hostname with the selected TLS source.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });

      // Validate-on-save: confirm a cert is attached and covers the hostname.
      try {
        const info = await safeInvoke<CertInfo>("cert_info", { id: target });
        if (info.errors.length > 0) {
          result = { kind: "error", message: info.errors[0] };
        } else {
          result = { kind: "issued", info, covered: info.sans.includes(host) };
        }
      } catch (e) {
        const err = e as CommandError | undefined;
        // No cert on disk yet (e.g. public ACME / not yet reconciled) is a
        // normal "will be issued shortly" state, not a failure.
        result = {
          kind: "pending",
          reason:
            err && err.code !== "PROJECT_NOT_FOUND" && err.code !== "NOT_FOUND"
              ? err.whatHappened
              : "The reconciler issues the certificate on the next project sync.",
        };
      }
    } catch (e) {
      const err = e as CommandError | undefined;
      result = {
        kind: "error",
        message: err?.whatHappened ?? "Could not save the certificate settings.",
      };
    } finally {
      submitting = false;
    }
  }
</script>

<div class="px-6 py-6">
  <div class="flex items-start justify-between gap-3">
    <div class="min-w-0">
      <h2 class="text-[15px] font-semibold text-fg">New Certificate</h2>
      <p class="mt-1 text-[12px] text-fg-subtle leading-relaxed">
        Attach a TLS certificate to one of your projects.
      </p>
    </div>
    <button
      type="button"
      onclick={() => onClose()}
      title="Close"
      aria-label="Close new certificate"
      class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2
             transition-colors shrink-0"
    >
      <Icon name="x" size={16} />
    </button>
  </div>

  <div class="mt-5 w-full space-y-5">
    {#if options.length === 0}
      <div class="text-center px-4 py-10">
        <span
          class="inline-grid place-items-center w-12 h-12 rounded-xl bg-surface-2
                 text-fg-subtle mx-auto"
        >
          <Icon name="shield" size={22} />
        </span>
        <p class="mt-3 text-[13px] text-fg-muted leading-relaxed">
          No projects yet. Add a project first — a certificate is always attached
          to a project's hostname.
        </p>
      </div>
    {:else if result && result.kind !== "error"}
      <!-- Confirmation: the cert is attached / pending issuance. -->
      {#if result.kind === "issued"}
        <div
          class="rounded-xl border border-status-running/30 bg-status-running/10
                 p-4 space-y-3"
        >
          <div class="flex items-center gap-2 text-status-running">
            <Icon name="check" size={16} />
            <span class="text-[13px] font-semibold">Certificate attached</span>
          </div>
          <p class="text-[12px] text-fg-muted leading-relaxed">
            <code class="font-mono">{hostname}</code> is now served over HTTPS.
          </p>
          <dl class="space-y-2 text-[12px]">
            <div class="flex justify-between gap-3">
              <dt class="text-fg-subtle">Validity</dt>
              <dd class="text-fg text-right">
                {fmtDate(result.info.issuedAt)} → {fmtDate(result.info.expiresAt)}
              </dd>
            </div>
            <div>
              <dt class="text-fg-subtle">Covers</dt>
              <dd class="mt-1 text-[11.5px] text-fg-muted font-mono break-all">
                {result.info.sans.length > 0 ? result.info.sans.join(", ") : "—"}
              </dd>
            </div>
          </dl>
          {#if !result.covered}
            <p
              class="flex items-start gap-2 text-[11.5px] text-status-unhealthy
                     bg-status-unhealthy/10 rounded-lg px-3 py-2 leading-relaxed"
            >
              <Icon name="circle-alert" size={13} class="mt-px shrink-0" />
              The issued certificate does not list
              <code class="font-mono">{hostname}</code> in its SANs. Reissue if
              the hostname later changes.
            </p>
          {/if}
        </div>
      {:else}
        <div class="rounded-xl border border-border bg-surface-2/40 p-4 space-y-2">
          <div class="flex items-center gap-2 text-fg">
            <Icon name="check" size={16} class="text-status-running" />
            <span class="text-[13px] font-semibold">Settings saved</span>
          </div>
          <p class="text-[12px] text-fg-muted leading-relaxed">
            {result.reason}
          </p>
        </div>
      {/if}
    {:else if draft}
      {#if result && result.kind === "error"}
        <p
          class="flex items-start gap-2 text-[11.5px] text-status-crashed
                 bg-status-crashed/10 rounded-lg px-3 py-2 leading-relaxed"
        >
          <Icon name="circle-alert" size={13} class="mt-px shrink-0" />
          {result.message}
        </p>
      {/if}

      <!-- Project picker (shared dropdown) -->
      <div class="space-y-1.5">
        <span class="block text-[12px] font-medium text-fg">Project</span>
        <ProjectSelector
          projects={options}
          selectedId={projectId}
          includeAllOption={false}
          showStatusDot={false}
          fullWidth
          onAddNew={() => addProjectWizard.requestAdd()}
          onselect={(id) => (projectId = id)}
        />
        {#if selected?.https}
          <p class="text-[11px] text-fg-subtle leading-relaxed">
            This project already has HTTPS — saving updates its existing
            certificate settings.
          </p>
        {/if}
      </div>

      <!-- Enable HTTPS -->
      <div class="rounded-xl border border-border px-4 py-1">
        <div class="flex items-start justify-between gap-4 py-2">
          <div class="min-w-0">
            <div class="text-[12.5px] text-fg">Enable HTTPS</div>
            <div class="mt-0.5 text-[11px] text-fg-subtle leading-relaxed">
              Create and serve a certificate for this project hostname.
            </div>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={draft.https}
            aria-label="Enable HTTPS"
            onclick={() => {
              if (!draft) return;
              draft.https = !draft.https;
              if (draft.https && draft.sslMode === "automatic_local")
                draft.autoManageCert = true;
            }}
            class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full
                   transition-colors active:scale-95 focus-visible:outline-none
                   focus-visible:ring-2 focus-visible:ring-accent/40
                   {draft.https ? 'bg-accent' : 'bg-surface-2 border border-border'}"
          >
            <span
              class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm
                     transition-transform duration-150
                     {draft.https ? 'translate-x-[18px]' : 'translate-x-0.5'}"
            ></span>
          </button>
        </div>
      </div>

      <!-- TLS source -->
      <TlsSourceFields bind:draft {hostname} idPrefix="add-cert" />

      {#if !draft.https}
        <p class="text-[11px] text-fg-subtle leading-relaxed">
          Turn on HTTPS above to create a certificate.
        </p>
      {/if}
    {/if}
  </div>

  <div class="mt-6 flex items-center gap-2 pt-4 border-t border-border w-full">
    {#if result && result.kind !== "error"}
      <button
        type="button"
        onclick={() => onClose(loadedFor ?? undefined)}
        class="ml-auto inline-flex items-center gap-1.5 h-9 px-4 rounded-lg
               text-[12.5px] font-medium bg-accent text-on-accent
               hover:brightness-110 active:scale-[0.98] transition"
      >
        Done
      </button>
    {:else}
      <button
        type="button"
        onclick={() => onClose()}
        class="h-9 px-3 rounded-lg text-[12.5px] text-fg-muted hover:bg-surface-2
               transition-colors"
      >
        Cancel
      </button>
      <button
        type="button"
        onclick={create}
        disabled={!selected || !draft || !draft.https || submitting}
        class="ml-auto inline-flex items-center gap-1.5 h-9 px-4 rounded-lg
               text-[12.5px] font-medium bg-accent text-on-accent
               hover:brightness-110 active:scale-[0.98] transition
               disabled:opacity-50 disabled:cursor-not-allowed"
      >
        <Icon
          name={submitting ? "refresh-cw" : "plus"}
          size={13}
          class={submitting ? "animate-spin" : ""}
        />
        {submitting ? "Creating…" : "Create certificate"}
      </button>
    {/if}
  </div>
</div>
