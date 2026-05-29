<!--
  AdvancedFields — exposes the lesser-used Project fields the
  Configuration card doesn't carry: tags, extra ports, services
  (per-project sidecar enablement), and PHP-specific document
  root + php_version.

  Saves through update_project the same way the Environment editor
  does. Each field area has its own dirty cycle so the user can
  edit one thing without touching another.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { ErrorEnvelope } from "$lib/components/errors";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { runtimes } from "$lib/stores/runtimes.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import CustomTunnelField from "./CustomTunnelField.svelte";
  import { account } from "$lib/stores/account.svelte";
  import type { CommandError } from "$lib/types/error";
  import type {
    MobileRunConfig,
    ProjectType,
    ProjectView,
  } from "$lib/types/projects";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  /** Services that PortBay knows how to wire into a project. The
   *  registry stores these as opaque strings; this set controls the
   *  multi-select offered by the UI. Add new entries here when the
   *  reconcile loop learns to wire them. */
  const KNOWN_SERVICES = ["caddy", "mailpit", "mysql", "postgres", "redis"];

  /** Versions PortBay knows about (Homebrew formula coverage). The
   *  picker marks any of these that aren't actually installed with a
   *  warning dot so the user sees they need to brew-install first. */
  const KNOWN_PHP_VERSIONS = ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4"];

  onMount(() => {
    void runtimes.refresh();
  });

  // ────────── Tags ──────────
  let tagsDraft = $state<string[]>([]);
  let newTag = $state<string>("");

  // ────────── Extra ports ──────────
  let extraPortsDraft = $state<string>(""); // comma-separated for easy editing

  // ────────── Services ──────────
  let servicesDraft = $state<string[]>([]);
  let serviceCustom = $state<string>("");

  // ────────── PHP-only ──────────
  let documentRootDraft = $state<string>("");
  let phpVersionDraft = $state<string>("");

  // ────────── Mobile-only ──────────
  let mobileFlavorDraft = $state<string>("");
  let mobileTargetDraft = $state<string>("");
  let mobileDeviceDraft = $state<string>("");

  // ────────── CORS (Pro) ──────────
  // The basic listen port stays free for everyone; only this custom
  // cross-origin policy is gated. Origins are edited one-per-line.
  let corsOriginsDraft = $state<string>("");
  let corsCredentialsDraft = $state<boolean>(false);
  const corsLocked = $derived(!entitlements.allows("custom_port_cors"));

  // Save state — shared across sub-sections.
  let saving = $state<boolean>(false);
  let error = $state<CommandError | null>(null);

  function syncFromProject() {
    tagsDraft = [...(project.tags ?? [])];
    newTag = "";
    extraPortsDraft = (project.extraPorts ?? []).join(", ");
    servicesDraft = [...(project.services ?? [])];
    serviceCustom = "";
    documentRootDraft = project.documentRoot ?? "";
    phpVersionDraft = project.phpVersion ?? "";
    mobileFlavorDraft = project.mobileRun?.flavor ?? "";
    mobileTargetDraft = project.mobileRun?.target ?? "";
    mobileDeviceDraft = project.mobileRun?.device ?? "";
    corsOriginsDraft = (project.cors?.allowedOrigins ?? []).join("\n");
    corsCredentialsDraft = project.cors?.allowCredentials ?? false;
    error = null;
  }

  // Parse the textarea into a clean origin list (trimmed, blank lines dropped).
  const corsOriginsParsed = $derived.by<string[]>(() =>
    corsOriginsDraft
      .split(/[\n,]+/)
      .map((s) => s.trim())
      .filter(Boolean),
  );

  $effect(() => {
    const _id = project.id;
    syncFromProject();
  });

  // ────────── Dirty derivations ──────────
  const tagsDirty = $derived.by(() => {
    const original = project.tags ?? [];
    if (tagsDraft.length !== original.length) return true;
    return tagsDraft.some((t, i) => t !== original[i]);
  });

  const extraPortsParsed = $derived.by<number[] | null>(() => {
    const trimmed = extraPortsDraft.trim();
    if (!trimmed) return [];
    const parts = trimmed.split(/[,\s]+/).filter(Boolean);
    const nums: number[] = [];
    for (const p of parts) {
      const n = Number(p);
      if (!Number.isInteger(n) || n < 1 || n > 65535) return null;
      nums.push(n);
    }
    return nums;
  });

  const extraPortsDirty = $derived.by(() => {
    if (extraPortsParsed === null) return false;
    const original = project.extraPorts ?? [];
    if (extraPortsParsed.length !== original.length) return true;
    return extraPortsParsed.some((p, i) => p !== original[i]);
  });

  const servicesDirty = $derived.by(() => {
    const original = project.services ?? [];
    const a = [...servicesDraft].sort();
    const b = [...original].sort();
    if (a.length !== b.length) return true;
    return a.some((s, i) => s !== b[i]);
  });

  const phpDirty = $derived(
    documentRootDraft !== (project.documentRoot ?? "") ||
      phpVersionDraft !== (project.phpVersion ?? ""),
  );

  const isPhp = $derived(project.type === "php");
  const isMobile = $derived(
    project.type === "flutter" ||
      project.type === "xcode" ||
      project.type === "android" ||
      project.type === "expo",
  );

  const mobileDraft = $derived<MobileRunConfig>({
    flavor: cleanOrNull(mobileFlavorDraft),
    target: cleanOrNull(mobileTargetDraft),
    device: cleanOrNull(mobileDeviceDraft),
  });

  const mobileCommand = $derived(
    commandForMobile(project.type, mobileDraft) ?? project.startCommand ?? "",
  );

  const mobileDirty = $derived.by(() => {
    const original = project.mobileRun ?? {};
    return (
      (mobileDraft.flavor ?? "") !== (original.flavor ?? "") ||
      (mobileDraft.target ?? "") !== (original.target ?? "") ||
      (mobileDraft.device ?? "") !== (original.device ?? "")
    );
  });

  const corsDirty = $derived.by(() => {
    const original = project.cors?.allowedOrigins ?? [];
    const a = corsOriginsParsed;
    const originsChanged =
      a.length !== original.length || a.some((o, i) => o !== original[i]);
    return originsChanged || corsCredentialsDraft !== (project.cors?.allowCredentials ?? false);
  });

  const anyDirty = $derived(
    tagsDirty ||
      extraPortsDirty ||
      servicesDirty ||
      (isPhp && phpDirty) ||
      (isMobile && mobileDirty) ||
      (!corsLocked && corsDirty),
  );

  // ────────── Tag actions ──────────
  function addTag() {
    const t = newTag.trim();
    if (!t) return;
    if (tagsDraft.includes(t)) {
      newTag = "";
      return;
    }
    tagsDraft = [...tagsDraft, t];
    newTag = "";
  }

  function removeTag(t: string) {
    tagsDraft = tagsDraft.filter((x) => x !== t);
  }

  // ────────── Service toggle ──────────
  function toggleService(s: string) {
    if (servicesDraft.includes(s)) {
      servicesDraft = servicesDraft.filter((x) => x !== s);
    } else {
      servicesDraft = [...servicesDraft, s];
    }
  }

  function addCustomService() {
    const s = serviceCustom.trim();
    if (!s || servicesDraft.includes(s)) {
      serviceCustom = "";
      return;
    }
    servicesDraft = [...servicesDraft, s];
    serviceCustom = "";
  }

  // ────────── Save / discard ──────────
  async function save() {
    if (!anyDirty || extraPortsParsed === null) return;
    saving = true;
    error = null;

    const patch: Record<string, unknown> = {};
    if (tagsDirty) patch.tags = tagsDraft;
    if (extraPortsDirty) patch.extraPorts = extraPortsParsed;
    if (servicesDirty) patch.services = servicesDraft;
    if (isPhp && phpDirty) {
      patch.documentRoot = documentRootDraft.trim();
      patch.phpVersion = phpVersionDraft.trim();
    }
    if (isMobile && mobileDirty) {
      patch.mobileRun = mobileDraft;
      patch.startCommand = mobileCommand;
    }
    if (!corsLocked && corsDirty) {
      patch.cors = {
        allowedOrigins: corsOriginsParsed,
        allowCredentials: corsCredentialsDraft,
      };
    }

    try {
      await safeInvoke<ProjectView>("update_project", {
        id: project.id,
        patch,
      });
      await projects.refresh();
      errorBus.push({
        code: "ADVANCED_SAVED",
        whatHappened: `${project.name} updated.`,
        whyItMatters: "Restart the project for changes to take effect.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch (e) {
      error = e as CommandError;
    } finally {
      saving = false;
    }
  }

  function discard() {
    syncFromProject();
  }

  function cleanOrNull(value: string): string | null {
    const trimmed = value.trim();
    return trimmed ? trimmed : null;
  }

  function shellQuote(value: string): string {
    return /^[A-Za-z0-9/._:-]+$/.test(value)
      ? value
      : `'${value.replaceAll("'", "'\\''")}'`;
  }

  function capitalizeAscii(value: string): string {
    return value ? value[0].toUpperCase() + value.slice(1) : value;
  }

  function commandForMobile(
    kind: ProjectType,
    cfg: MobileRunConfig,
  ): string | null {
    if (kind === "flutter") {
      const args = ["flutter", "run"];
      if (cfg.flavor) args.push("--flavor", shellQuote(cfg.flavor));
      if (cfg.device) args.push("-d", shellQuote(cfg.device));
      return args.join(" ");
    }
    if (kind === "xcode") {
      if (!cfg.target) return "xed .";
      const args = ["xcodebuild", "-scheme", shellQuote(cfg.target)];
      if (cfg.device) args.push("-destination", shellQuote(cfg.device));
      args.push("build");
      return args.join(" ");
    }
    if (kind === "android") {
      const module = cfg.target || "app";
      const variant = capitalizeAscii(cfg.flavor || "debug");
      const command = `./gradlew :${module}:install${variant}`;
      return cfg.device
        ? `ANDROID_SERIAL=${shellQuote(cfg.device)} ${command}`
        : command;
    }
    return null;
  }
</script>

<div class="space-y-5">
  <!-- Tags -->
  <section class="space-y-2">
    <div class="flex items-center justify-between">
      <span class="text-xs uppercase tracking-wide text-fg-subtle">Tags</span>
      {#if tagsDraft.length > 0}
        <span class="text-[10px] text-fg-subtle">{tagsDraft.length}</span>
      {/if}
    </div>
    <div class="flex flex-wrap gap-1.5">
      {#each tagsDraft as tag (tag)}
        <span
          class="inline-flex items-center gap-1 pl-2 pr-1 py-0.5 rounded-md
                 bg-surface-2 text-xs text-fg-muted border border-border"
        >
          {tag}
          <button
            type="button"
            onclick={() => removeTag(tag)}
            aria-label="Remove tag {tag}"
            class="p-0.5 rounded hover:bg-surface text-fg-subtle hover:text-status-crashed"
          >
            <Icon name="x" size={10} />
          </button>
        </span>
      {/each}
      <div class="flex items-center gap-1">
        <input
          type="text"
          bind:value={newTag}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addTag();
            }
          }}
          placeholder="add tag…"
          class="px-2 py-0.5 text-xs rounded-md bg-bg border border-border
                 focus:border-accent/60 outline-none w-32"
        />
        {#if newTag.trim()}
          <button
            type="button"
            onclick={addTag}
            class="p-1 rounded-md text-accent hover:bg-accent/10"
            aria-label="Add tag"
          >
            <Icon name="plus" size={11} />
          </button>
        {/if}
      </div>
    </div>
    <p class="text-[10px] text-fg-subtle">
      Free-form labels for grouping. Press Enter to add.
    </p>
  </section>

  <!-- Extra ports -->
  <section class="space-y-2">
    <span class="text-xs uppercase tracking-wide text-fg-subtle">Extra ports</span>
    <input
      type="text"
      bind:value={extraPortsDraft}
      placeholder="24678, 9000"
      spellcheck="false"
      class="w-full px-2.5 py-1.5 rounded-md bg-bg border border-border
             focus:border-accent/60 outline-none text-fg font-mono text-xs"
      class:border-status-crashed={extraPortsParsed === null}
    />
    <p class="text-[10px] text-fg-subtle">
      Additional ports Caddy should route to this project. Comma-separated.
      {#if extraPortsParsed === null}
        <span class="text-status-crashed">
          Must be integers between 1 and 65535.
        </span>
      {/if}
    </p>
  </section>

  <!-- CORS (Pro) -->
  <section class="space-y-2">
    <div class="flex items-center justify-between">
      <span class="text-xs uppercase tracking-wide text-fg-subtle">CORS</span>
      <span class="text-[10px] px-1.5 py-0.5 rounded bg-accent/10 text-accent font-medium">Pro</span>
    </div>
    {#if corsLocked}
      <div class="flex items-start gap-2 rounded-md border border-border bg-surface-2/40 p-2.5">
        <Icon name="lock" size={13} class="text-fg-subtle mt-0.5 shrink-0" />
        <div class="flex-1 space-y-1.5">
          <p class="text-[11px] leading-relaxed text-fg-muted">
            Let specific origins call this project across origins (custom
            <code class="text-fg">Access-Control-Allow-Origin</code>). Custom CORS is a
            <span class="text-fg">PortBay Pro</span> feature — the project's port stays free.
          </p>
          <button
            type="button"
            onclick={() => account.open({ intent: "pro" })}
            class="inline-flex items-center gap-1 text-[11px] font-medium text-accent hover:underline"
          >
            <Icon name="sparkles" size={11} /> Unlock with Pro
          </button>
        </div>
      </div>
    {:else}
      <textarea
        bind:value={corsOriginsDraft}
        rows="3"
        placeholder={"https://app.example.test\nhttps://admin.example.test"}
        spellcheck="false"
        class="w-full px-2.5 py-1.5 rounded-md bg-bg border border-border
               focus:border-accent/60 outline-none text-fg font-mono text-xs resize-y"
      ></textarea>
      <label class="flex items-center gap-2 text-[11px] text-fg-muted">
        <input type="checkbox" bind:checked={corsCredentialsDraft} class="accent-accent" />
        Allow credentials (<code class="text-fg-subtle">Access-Control-Allow-Credentials</code>)
      </label>
      <p class="text-[10px] text-fg-subtle">
        One allowed origin per line. Only these origins are echoed back — never a
        blanket <code>*</code>. Leave empty to remove the policy.
      </p>
    {/if}
  </section>

  <!-- Services -->
  <section class="space-y-2">
    <span class="text-xs uppercase tracking-wide text-fg-subtle">Services</span>
    <div class="flex flex-wrap gap-1.5">
      {#each KNOWN_SERVICES as svc (svc)}
        {@const on = servicesDraft.includes(svc)}
        <button
          type="button"
          onclick={() => toggleService(svc)}
          class="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-xs
                 border transition-colors"
          class:bg-accent={on}
          class:text-on-accent={on}
          class:border-accent={on}
          class:bg-surface-2={!on}
          class:text-fg-muted={!on}
          class:border-border={!on}
        >
          {#if on}
            <Icon name="check" size={10} />
          {/if}
          {svc}
        </button>
      {/each}
      {#each servicesDraft.filter((s) => !KNOWN_SERVICES.includes(s)) as svc (svc)}
        <span
          class="inline-flex items-center gap-1 pl-2 pr-1 py-0.5 rounded-md
                 bg-accent text-on-accent text-xs"
        >
          <Icon name="check" size={10} />
          {svc}
          <button
            type="button"
            onclick={() => toggleService(svc)}
            aria-label="Remove custom service {svc}"
            class="p-0.5 rounded hover:bg-white/20"
          >
            <Icon name="x" size={10} />
          </button>
        </span>
      {/each}
    </div>
    <div class="flex items-center gap-1.5">
      <input
        type="text"
        bind:value={serviceCustom}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            addCustomService();
          }
        }}
        placeholder="custom service id…"
        class="flex-1 px-2 py-0.5 text-xs rounded-md bg-bg border border-border
               focus:border-accent/60 outline-none"
      />
      {#if serviceCustom.trim()}
        <button
          type="button"
          onclick={addCustomService}
          class="p-1 rounded-md text-accent hover:bg-accent/10"
          aria-label="Add custom service"
        >
          <Icon name="plus" size={11} />
        </button>
      {/if}
    </div>
    <p class="text-[10px] text-fg-subtle">
      Per-project sidecars to wire (e.g. <code>mysql</code> if the project
      depends on a local database).
    </p>
  </section>

  {#if isPhp}
    <section class="space-y-2">
      <span class="text-xs uppercase tracking-wide text-fg-subtle">PHP</span>
      <div class="grid grid-cols-[110px,1fr] gap-x-3 gap-y-2 items-center text-sm">
        <label for="advanced-doc-root" class="text-fg-muted">Document root</label>
        <input
          id="advanced-doc-root"
          type="text"
          bind:value={documentRootDraft}
          placeholder="public"
          spellcheck="false"
          class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                 focus:border-accent/60 outline-none text-fg font-mono text-xs"
        />
        <label for="advanced-php-ver" class="text-fg-muted">PHP version</label>
        <div class="flex flex-wrap gap-1.5 items-center">
          {#each KNOWN_PHP_VERSIONS as v (v)}
            {@const on = phpVersionDraft === v}
            {@const installed = runtimes.isInstalled("php", v)}
            <button
              type="button"
              onclick={() => (phpVersionDraft = v)}
              title={installed
                ? `PHP ${v} detected`
                : `Not installed — run brew install php@${v}`}
              class="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-xs border transition-colors"
              class:bg-accent={on}
              class:text-on-accent={on}
              class:border-accent={on}
              class:bg-surface-2={!on}
              class:text-fg-muted={!on}
              class:border-border={!on}
              class:opacity-60={!installed && !on}
            >
              {#if !installed}
                <Icon name="info" size={10} />
              {/if}
              {v}
            </button>
          {/each}
          <input
            id="advanced-php-ver"
            type="text"
            bind:value={phpVersionDraft}
            placeholder="custom"
            spellcheck="false"
            class="px-2 py-0.5 text-xs rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none w-24 font-mono"
          />
        </div>
        {#if phpVersionDraft && !runtimes.isInstalled("php", phpVersionDraft)}
          <span></span>
          <p class="text-[11px] text-status-unhealthy">
            PHP {phpVersionDraft} isn't installed. Run
            <code class="font-mono">brew install php@{phpVersionDraft}</code>
            then re-detect from the Languages panel.
          </p>
        {/if}
      </div>
      <p class="text-[10px] text-fg-subtle">
        Document root is typically <code>public</code> for Laravel. PHP version
        selects which PHP-FPM binary handles requests.
      </p>
    </section>
  {/if}

  {#if isMobile}
    <section class="space-y-2">
      <span class="text-xs uppercase tracking-wide text-fg-subtle">Mobile run</span>
      <div class="grid grid-cols-[110px,1fr] gap-x-3 gap-y-2 items-center text-sm">
        {#if project.type === "flutter"}
          <label for="advanced-mobile-flavor" class="text-fg-muted">Flavor</label>
          <input
            id="advanced-mobile-flavor"
            type="text"
            bind:value={mobileFlavorDraft}
            placeholder="staging"
            spellcheck="false"
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-fg font-mono text-xs"
          />
          <label for="advanced-mobile-device" class="text-fg-muted">Device</label>
          <input
            id="advanced-mobile-device"
            type="text"
            bind:value={mobileDeviceDraft}
            placeholder="iPhone 16 or emulator-5554"
            spellcheck="false"
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-fg font-mono text-xs"
          />
        {:else if project.type === "xcode"}
          <label for="advanced-mobile-target" class="text-fg-muted">Scheme</label>
          <input
            id="advanced-mobile-target"
            type="text"
            bind:value={mobileTargetDraft}
            placeholder="App"
            spellcheck="false"
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-fg font-mono text-xs"
          />
          <label for="advanced-mobile-device" class="text-fg-muted">Destination</label>
          <input
            id="advanced-mobile-device"
            type="text"
            bind:value={mobileDeviceDraft}
            placeholder="platform=iOS Simulator,name=iPhone 16"
            spellcheck="false"
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-fg font-mono text-xs"
          />
        {:else if project.type === "android"}
          <label for="advanced-mobile-target" class="text-fg-muted">Module</label>
          <input
            id="advanced-mobile-target"
            type="text"
            bind:value={mobileTargetDraft}
            placeholder="app"
            spellcheck="false"
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-fg font-mono text-xs"
          />
          <label for="advanced-mobile-flavor" class="text-fg-muted">Variant</label>
          <input
            id="advanced-mobile-flavor"
            type="text"
            bind:value={mobileFlavorDraft}
            placeholder="debug"
            spellcheck="false"
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-fg font-mono text-xs"
          />
          <label for="advanced-mobile-device" class="text-fg-muted">Device</label>
          <input
            id="advanced-mobile-device"
            type="text"
            bind:value={mobileDeviceDraft}
            placeholder="emulator-5554"
            spellcheck="false"
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-fg font-mono text-xs"
          />
        {/if}
      </div>
      <div class="rounded-md border border-border bg-bg px-2.5 py-1.5">
        <code class="block text-[11px] text-fg-muted break-all">{mobileCommand}</code>
      </div>
    </section>
  {/if}

  {#if project.workspace}
    <section class="space-y-2">
      <span class="text-xs uppercase tracking-wide text-fg-subtle">Monorepo</span>
      <div class="grid grid-cols-[110px,1fr] gap-x-3 gap-y-1 text-xs">
        <span class="text-fg-subtle">Package</span>
        <span class="font-mono text-fg-muted break-all">{project.workspace.package}</span>
        <span class="text-fg-subtle">App dir</span>
        <span class="font-mono text-fg-muted break-all">{project.workspace.relDir}</span>
        <span class="text-fg-subtle">Filter</span>
        <span class="font-mono text-fg-muted">{project.workspace.tool}</span>
      </div>
      <p class="text-[10px] text-fg-subtle">
        Runs one app of a monorepo from the repo root via a workspace filter.
        Set when the project is added; remove and re-add to change it.
      </p>
    </section>
  {/if}

  <!-- Custom tunnel (Pro) — self-saving, independent of the bar below. -->
  <section class="space-y-2">
    <div class="flex items-center gap-2">
      <span class="text-xs uppercase tracking-wide text-fg-subtle">Public tunnel</span>
    </div>
    <CustomTunnelField {project} />
  </section>

  {#if anyDirty}
    <div
      class="flex items-center justify-end gap-2 pt-2 border-t border-border"
    >
      <button
        type="button"
        onclick={discard}
        class="px-2.5 py-1 text-xs rounded-md text-fg-muted hover:text-fg
               hover:bg-surface-2 transition-colors"
      >
        Discard
      </button>
      <button
        type="button"
        onclick={save}
        disabled={saving || extraPortsParsed === null}
        class="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs
               rounded-md text-accent border border-accent/40
               hover:bg-accent/10 disabled:opacity-50 transition-colors"
      >
        {#if saving}
          <Icon name="refresh-cw" size={11} class="animate-spin" /> Saving…
        {:else}
          <Icon name="check" size={11} /> Save advanced
        {/if}
      </button>
    </div>
  {/if}

  {#if error}
    <ErrorEnvelope envelope={error} tone="inline" />
  {/if}
</div>
