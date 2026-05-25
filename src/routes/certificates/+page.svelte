<!--
  /certificates — the SSL Certificates overview.

  PortBay doesn't manage a CA "vault" the way ServBay does — every HTTPS
  project gets a locally-trusted cert minted by the bundled mkcert at
  reconcile time. So this page surfaces *reality*: one row per HTTPS project,
  showing the cert PortBay issued for it (common name, SANs, validity window)
  plus the per-cert actions that already exist on the project detail panel —
  reissue, reveal the PEM in Finder, and open the site.

  Data: `list_projects` (the projects store) filtered to `https`, then
  `cert_info(id)` per project. A `PROJECT_NOT_FOUND`-style miss means the
  reconciler hasn't issued the cert yet — that's the per-row empty state, not
  an error.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { openUrl } from "$lib/security/openUrl";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { CertInfo } from "$lib/types/certs";
  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";

  // Every PortBay cert is minted by the bundled mkcert against its local root
  // CA — there's a single issuer, so we label it rather than parse it out of
  // each PEM (cert_info doesn't surface the issuer field).
  const ISSUER = "mkcert local CA";

  type CertCell = {
    info: CertInfo | null;
    error: string | null;
    loading: boolean;
  };

  // Per-project cert results, keyed by project id. Reactive so a reissue or a
  // late-arriving fetch repaints just that row.
  let certs = $state<Record<string, CertCell>>({});
  let query = $state<string>("");
  let reissuing = $state<string | null>(null);

  // Only HTTPS projects have a cert; plain-HTTP ones never appear here.
  const httpsProjects = $derived<ProjectView[]>(
    projects.value.filter((p) => p.https),
  );

  const filtered = $derived.by<ProjectView[]>(() => {
    const q = query.trim().toLowerCase();
    if (!q) return httpsProjects;
    return httpsProjects.filter((p) => {
      const sans = certs[p.id]?.info?.sans ?? [];
      return (
        p.hostname.toLowerCase().includes(q) ||
        p.name.toLowerCase().includes(q) ||
        sans.some((s) => s.toLowerCase().includes(q))
      );
    });
  });

  onMount(() => {
    void (async () => {
      await projects.refresh();
      await loadAll();
    })();
  });

  async function loadAll(): Promise<void> {
    await Promise.all(httpsProjects.map((p) => loadCert(p.id)));
  }

  async function loadCert(id: string): Promise<void> {
    certs[id] = { info: certs[id]?.info ?? null, error: null, loading: true };
    try {
      const info = await safeInvoke<CertInfo>("cert_info", { id });
      certs[id] = { info, error: null, loading: false };
    } catch (e) {
      const err = e as CommandError | undefined;
      // PROJECT_NOT_FOUND / NotFound means "not issued yet" — the empty state.
      const message =
        err && err.code !== "PROJECT_NOT_FOUND" ? err.whatHappened : null;
      certs[id] = { info: null, error: message, loading: false };
    }
  }

  async function reissue(p: ProjectView): Promise<void> {
    if (reissuing) return;
    reissuing = p.id;
    try {
      await safeInvoke("reissue_cert", { id: p.id });
      // The reconcile tick mints the new cert; give it a beat then refetch.
      await new Promise((r) => setTimeout(r, 400));
      await loadCert(p.id);
      errorBus.push({
        code: "REISSUE_OK",
        whatHappened: `Cert reissued for ${p.name}.`,
        whyItMatters: "Caddy reloaded the cert; refresh your browser tab.",
        whoCausedIt: "system",
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

  /** Short, locale-aware date. Falls back to the raw string if unparseable. */
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

  type Tone = "valid" | "expiring" | "missing";

  function tone(p: ProjectView): Tone {
    const cell = certs[p.id];
    if (!cell?.info) return "missing";
    const days = cell.info.daysUntilExpiry;
    if (days !== null && days < 30) return "expiring";
    return "valid";
  }

  const toneDot: Record<Tone, string> = {
    valid: "bg-status-running",
    expiring: "bg-status-unhealthy",
    missing: "bg-fg-subtle/60",
  };
</script>

<div class="h-full overflow-y-auto">
  <div class="px-6 py-6 space-y-5">
    <!-- Header -->
    <header class="flex items-center gap-4 flex-wrap">
      <div class="min-w-0 flex-1">
        <h1 class="text-[20px] font-semibold text-fg leading-none">
          SSL Certificates
        </h1>
        <p class="mt-1.5 text-[12.5px] text-fg-subtle leading-relaxed">
          Locally-trusted certificates PortBay issued for your HTTPS projects,
          via the bundled mkcert CA.
        </p>
      </div>

      <!-- Search — filters by common name, project, or SAN. -->
      <label
        class="relative shrink-0 flex items-center"
        aria-label="Search certificates"
      >
        <Icon
          name="search"
          size={14}
          class="absolute left-2.5 text-fg-subtle pointer-events-none"
        />
        <input
          type="search"
          bind:value={query}
          placeholder="Search"
          class="h-8 w-56 pl-8 pr-3 rounded-lg bg-surface border border-border
                 text-[12.5px] text-fg placeholder:text-fg-subtle
                 focus:outline-none focus:ring-2 focus:ring-accent/40"
        />
      </label>
    </header>

    {#if httpsProjects.length === 0}
      <!-- No HTTPS projects at all. -->
      <div
        class="rounded-xl border border-dashed border-border px-6 py-12 text-center"
      >
        <Icon name="shield" size={28} class="text-fg-subtle mx-auto" />
        <p class="mt-3 text-[13px] text-fg-muted">
          No HTTPS projects yet.
        </p>
        <p class="mt-1 text-[12px] text-fg-subtle">
          Enable HTTPS on a project and PortBay issues a trusted cert
          automatically.
        </p>
      </div>
    {:else if filtered.length === 0}
      <!-- Search matched nothing. -->
      <p class="px-1 py-8 text-center text-[13px] text-fg-subtle">
        No certificates match “{query}”.
      </p>
    {:else}
      <!-- Certificates table -->
      <div class="rounded-xl border border-border overflow-hidden">
        <table class="w-full text-left border-collapse">
          <thead>
            <tr
              class="text-[11px] uppercase tracking-wide text-fg-subtle
                     border-b border-border bg-surface/40"
            >
              <th class="font-medium px-4 py-2.5">Common Name</th>
              <th class="font-medium px-4 py-2.5">Subject Alternative Name</th>
              <th class="font-medium px-4 py-2.5">Issuer</th>
              <th class="font-medium px-4 py-2.5 whitespace-nowrap">Issued</th>
              <th class="font-medium px-4 py-2.5 whitespace-nowrap">Expires</th>
              <th class="font-medium px-4 py-2.5 text-right">Action</th>
            </tr>
          </thead>
          <tbody>
            {#each filtered as p (p.id)}
              {@const cell = certs[p.id]}
              {@const t = tone(p)}
              <tr
                class="border-b border-border/60 last:border-0
                       hover:bg-surface-2/40 transition-colors align-middle"
              >
                <!-- Common Name -->
                <td class="px-4 py-3">
                  <span class="flex items-center gap-2.5 min-w-0">
                    <span
                      class="inline-block w-2 h-2 rounded-full shrink-0 {toneDot[
                        t
                      ]}"
                      title={t === "expiring"
                        ? "Expires soon"
                        : t === "missing"
                          ? "Not issued yet"
                          : "Valid"}
                      aria-hidden="true"
                    ></span>
                    <span class="min-w-0 leading-tight">
                      <span class="block text-[13px] font-semibold text-fg truncate">
                        {p.name}
                      </span>
                      <span class="block text-[11.5px] font-mono text-fg-subtle truncate">
                        {p.hostname}
                      </span>
                    </span>
                  </span>
                </td>

                <!-- SANs -->
                <td class="px-4 py-3 text-[12px] text-fg-muted">
                  {#if cell?.loading && !cell?.info}
                    <span class="text-fg-subtle">…</span>
                  {:else if cell?.info && cell.info.sans.length > 0}
                    <span class="font-mono break-all">
                      {cell.info.sans.join(", ")}
                    </span>
                  {:else}
                    <span class="text-fg-subtle">—</span>
                  {/if}
                </td>

                <!-- Issuer -->
                <td class="px-4 py-3 text-[12px] text-fg-muted whitespace-nowrap">
                  {#if cell?.info}
                    <span class="inline-flex items-center gap-1.5">
                      <Icon name="shield" size={13} class="text-fg-subtle" />
                      {ISSUER}
                    </span>
                  {:else}
                    <span class="text-fg-subtle">—</span>
                  {/if}
                </td>

                <!-- Issued -->
                <td class="px-4 py-3 text-[12px] font-mono text-fg-muted whitespace-nowrap">
                  {fmtDate(cell?.info?.issuedAt ?? null)}
                </td>

                <!-- Expires -->
                <td class="px-4 py-3 text-[12px] font-mono whitespace-nowrap">
                  {#if cell?.info}
                    <span class={t === "expiring" ? "text-status-unhealthy" : "text-fg-muted"}>
                      {fmtDate(cell.info.expiresAt)}
                    </span>
                    {#if cell.info.daysUntilExpiry !== null}
                      <span class="block text-[10.5px] text-fg-subtle">
                        {cell.info.daysUntilExpiry} day{cell.info.daysUntilExpiry === 1
                          ? ""
                          : "s"} left
                      </span>
                    {/if}
                  {:else if cell?.error}
                    <span class="text-status-crashed" title={cell.error}>error</span>
                  {:else}
                    <span class="text-fg-subtle">not issued</span>
                  {/if}
                </td>

                <!-- Actions -->
                <td class="px-4 py-3">
                  <div class="flex items-center justify-end gap-1">
                    <button
                      type="button"
                      onclick={() => reissue(p)}
                      disabled={reissuing === p.id}
                      title="Reissue certificate"
                      aria-label="Reissue certificate for {p.name}"
                      class="p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
                             transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                      <Icon
                        name="rotate-cw"
                        size={14}
                        class={reissuing === p.id ? "animate-spin" : ""}
                      />
                    </button>
                    <button
                      type="button"
                      onclick={() => revealCertFolder(p)}
                      disabled={!cell?.info}
                      title="Reveal certificate folder in Finder"
                      aria-label="Reveal certificate folder for {p.name}"
                      class="p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
                             transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                    >
                      <Icon name="folder" size={14} />
                    </button>
                    <button
                      type="button"
                      onclick={() => openSite(p)}
                      title="Open {p.url}"
                      aria-label="Open {p.name} in browser"
                      class="p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
                             transition-colors"
                    >
                      <Icon name="external-link" size={14} />
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
</div>
