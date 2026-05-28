<!--
  /onboarding — first-run welcome screen.

  Three internal screens managed by a `step` state machine:

    welcome  → "I have a project" / "Start fresh" with a live health
               check footer.
    gallery  → 5-template grid; click → folder picker → name input.
    running  → live scaffolder log scroller with a Cancel option.

  Skip lands the user on `/` with the marker written so subsequent
  launches go straight to the projects table.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { Channel, invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import LighthouseLogo from "$lib/components/atoms/LighthouseLogo.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { onboarding } from "$lib/stores/onboarding.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import {
    TEMPLATES,
    type ScaffoldEvent,
    type ScaffoldKind,
  } from "$lib/types/onboarding";

  type Step = "welcome" | "gallery" | "running";

  interface DoctorFinding {
    check: string;
    verdict: "ok" | "warn" | "fail";
    detail: string;
  }
  interface DoctorReport {
    findings: DoctorFinding[];
  }

  // The hosted web demo (try.portbay.app) has no Tauri runtime: the native
  // folder picker and the scaffolder don't work, so the welcome CTAs would
  // dead-end and trap the visitor. In that mode every path completes into the
  // populated demo workspace instead.
  const isSimulator = import.meta.env.PUBLIC_SIMULATOR === "true";

  let step = $state<Step>("welcome");
  let doctor = $state<DoctorReport | null>(null);
  let doctorLoading = $state<boolean>(false);

  // Gallery → scaffold flow.
  let chosenKind = $state<ScaffoldKind | null>(null);
  let chosenParent = $state<string>("");
  let chosenName = $state<string>("");
  let busy = $state<boolean>(false);

  // Live scaffolder output.
  let logLines = $state<string[]>([]);
  let scrollerEl: HTMLDivElement | null = $state(null);

  onMount(async () => {
    await runDoctor();
  });

  async function runDoctor() {
    doctorLoading = true;
    try {
      doctor = await safeInvoke<DoctorReport>("doctor");
    } catch {
      // safeInvoke already pushed a toast; render the warning state.
    } finally {
      doctorLoading = false;
    }
  }

  function openAddWizard() {
    if (isSimulator) {
      // The add-project wizard relies on a native folder dialog that the web
      // demo can't show — land the visitor in the demo workspace instead.
      void skip();
      return;
    }
    // The existing wizard is mounted at the layout root; opening it
    // works from anywhere. We do NOT mark onboarded yet — the user
    // can still cancel the wizard. Marker writes happen on
    // successful add_project from inside the wizard flow's success
    // path (a separate concern), OR when they hit Skip here.
    addProjectWizard.show();
  }

  async function startFresh() {
    step = "gallery";
  }

  async function pickFolderForTemplate(kind: ScaffoldKind) {
    const tmpl = TEMPLATES.find((t) => t.kind === kind);
    if (!tmpl) return;
    if (isSimulator) {
      // No native dialog in the demo: skip the folder picker and preview the
      // scaffold step with a sample parent path. "Scaffold" then completes
      // into the demo workspace (the mock scaffolder is a no-op).
      chosenKind = kind;
      chosenParent = "~/Projects";
      chosenName = tmpl.defaultName;
      return;
    }
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: `Where should ${tmpl.name} scaffold the new project?`,
    });
    if (typeof picked !== "string") return;
    chosenKind = kind;
    chosenParent = picked;
    chosenName = tmpl.defaultName;
  }

  function cancelTemplate() {
    chosenKind = null;
    chosenParent = "";
    chosenName = "";
  }

  async function runScaffold() {
    if (!chosenKind || !chosenParent || !chosenName.trim()) return;
    busy = true;
    logLines = [];
    step = "running";

    const ch = new Channel<ScaffoldEvent>();
    ch.onmessage = (event) => {
      if (event.kind === "log") {
        logLines = logLines.concat(event.line);
        requestAnimationFrame(() => {
          if (scrollerEl) scrollerEl.scrollTop = scrollerEl.scrollHeight;
        });
      } else if (event.kind === "done") {
        // Will be handled by the await resolution below; no extra
        // work needed here.
      }
    };

    try {
      await invoke("scaffold_template", {
        kind: chosenKind,
        parentPath: chosenParent,
        name: chosenName.trim(),
        onEvent: ch,
      });
      await onboarding.markOnboarded();
      await goto("/");
    } catch (raw) {
      // Already toasted via the underlying envelope; surface inline
      // too so the user sees what went wrong in the log scroller.
      const message =
        typeof raw === "object" && raw !== null && "whatHappened" in raw
          ? (raw as { whatHappened: string }).whatHappened
          : String(raw);
      logLines = logLines.concat(`\n✗ ${message}`);
    } finally {
      busy = false;
    }
  }

  async function skip() {
    await onboarding.markOnboarded();
    await goto("/");
  }

  // Verdict aggregation for the health-check pill cluster.
  const verdictCount = $derived.by(() => {
    const acc = { ok: 0, warn: 0, fail: 0 };
    for (const f of doctor?.findings ?? []) acc[f.verdict] += 1;
    return acc;
  });
  const healthIsGreen = $derived(
    doctor !== null && verdictCount.warn === 0 && verdictCount.fail === 0,
  );

  /** Findings that need user action (warn or fail) get a remediation
   *  hint mapped from the check name. Returning null means no hint —
   *  the pill itself is informational. */
  function remediation(check: string): string | null {
    if (check === "registry") {
      return "PortBay couldn't read your registry. Restart the app, or remove ~/Library/Application Support/PortBay/registry.json and add a project to recreate it.";
    }
    if (check === "process-compose" || check === "caddy") {
      return `Open Settings → Diagnostics and click the restart action for ${check}. The bundled sidecar should come back up within a few seconds.`;
    }
    if (check === "tool: mkcert") {
      return "PortBay bundles mkcert — this only matters for CLI standalone use. Install via Homebrew: brew install mkcert.";
    }
    if (check === "tool: caddy" || check === "tool: process-compose") {
      return `PortBay bundles ${check.replace("tool: ", "")} — this finding only affects the CLI binary, not the .app. Safe to ignore for GUI-only use.`;
    }
    if (check === "/etc/hosts") {
      return "Open Settings → DNS routing and install the resolver file, or run `sudo portbay hosts reconcile` from a terminal.";
    }
    return null;
  }

  const actionable = $derived(
    (doctor?.findings ?? []).filter(
      (f) => f.verdict !== "ok" && remediation(f.check) !== null,
    ),
  );
</script>

<div class="h-full w-full flex flex-col">
  <!-- Top strip with brand mark + skip. macOS traffic lights sit in
       the top-left corner; the strip leaves room for them. -->
  <header
    class="flex items-center justify-between pl-24 pr-8 py-5 border-b border-border bg-bg"
  >
    <div class="flex items-center gap-2 text-fg-muted">
      <LighthouseLogo size={20} />
      <span class="text-sm font-medium tracking-wide">PortBay</span>
    </div>
    <button
      type="button"
      onclick={skip}
      class="text-xs text-fg-subtle hover:text-fg transition-colors"
      title="Skip onboarding and go to the empty projects table"
    >
      Skip
    </button>
  </header>

  <!-- Main panel -->
  <div class="flex-1 min-h-0 overflow-y-auto">
    <div class="max-w-3xl mx-auto px-8 py-12">
      {#if step === "welcome"}
        <div class="text-center mb-10">
          <div class="mx-auto w-14 h-14 flex items-center justify-center mb-5">
            <LighthouseLogo size={56} />
          </div>
          <h1 class="text-2xl font-semibold tracking-tight mb-2">
            Welcome to PortBay
          </h1>
          <p class="text-fg-muted text-sm max-w-md mx-auto">
            Run multiple local projects side by side. One Play button per
            project; one universal Stop. Pick how you want to start.
          </p>
        </div>

        <div class="grid grid-cols-2 gap-4">
          <button
            type="button"
            onclick={openAddWizard}
            class="group flex flex-col items-start gap-3 p-6 rounded-xl
                   border border-border bg-surface hover:border-accent/60
                   hover:bg-surface-2 transition-all text-left"
          >
            <div
              class="w-10 h-10 rounded-lg bg-accent/10 text-accent
                     flex items-center justify-center"
            >
              <Icon name="folder" size={18} />
            </div>
            <div class="flex-1 min-h-0">
              <div class="font-medium mb-1">I have a project</div>
              <p class="text-xs text-fg-muted leading-relaxed">
                Point PortBay at an existing folder. We'll detect the
                framework and pick sensible defaults.
              </p>
            </div>
            <div
              class="flex items-center gap-1 text-xs text-fg-subtle
                     group-hover:text-accent transition-colors"
            >
              Add existing
              <Icon name="arrow-right" size={12} />
            </div>
          </button>

          <button
            type="button"
            onclick={startFresh}
            class="group flex flex-col items-start gap-3 p-6 rounded-xl
                   border border-border bg-surface hover:border-accent/60
                   hover:bg-surface-2 transition-all text-left"
          >
            <div
              class="w-10 h-10 rounded-lg bg-accent/10 text-accent
                     flex items-center justify-center"
            >
              <Icon name="sparkles" size={18} />
            </div>
            <div class="flex-1 min-h-0">
              <div class="font-medium mb-1">Start fresh</div>
              <p class="text-xs text-fg-muted leading-relaxed">
                Scaffold a new Next.js, Vite, Astro, Laravel, or plain
                PHP project from a clean template.
              </p>
            </div>
            <div
              class="flex items-center gap-1 text-xs text-fg-subtle
                     group-hover:text-accent transition-colors"
            >
              Pick a template
              <Icon name="arrow-right" size={12} />
            </div>
          </button>
        </div>

        <!-- Health check strip -->
        <section class="mt-10 p-5 rounded-xl border border-border bg-surface">
          <div class="flex items-center justify-between mb-3">
            <div class="flex items-center gap-2">
              <span class="text-sm font-medium">System check</span>
              {#if doctorLoading}
                <span class="text-xs text-fg-subtle">running…</span>
              {:else if healthIsGreen}
                <span
                  class="inline-flex items-center gap-1 text-xs text-success"
                >
                  <Icon name="circle-check" size={12} /> ready
                </span>
              {:else if doctor}
                <span class="text-xs text-warn">
                  {verdictCount.warn + verdictCount.fail} issue{(verdictCount.warn +
                    verdictCount.fail) ===
                  1
                    ? ""
                    : "s"}
                </span>
              {/if}
            </div>
            <button
              type="button"
              onclick={runDoctor}
              disabled={doctorLoading}
              class="text-xs text-fg-subtle hover:text-fg disabled:opacity-50
                     inline-flex items-center gap-1"
            >
              <Icon name="refresh-cw" size={11} /> Recheck
            </button>
          </div>
          <div class="flex flex-wrap gap-2">
            {#each doctor?.findings ?? [] as f (f.check)}
              <span
                class="inline-flex items-center gap-1.5 px-2 py-1 rounded-md
                       border border-border bg-bg text-xs"
                title={f.detail}
              >
                <StatusDot
                  status={f.verdict === "ok"
                    ? "running"
                    : f.verdict === "warn"
                      ? "unhealthy"
                      : "crashed"}
                />
                <span class="text-fg-muted">{f.check}</span>
              </span>
            {/each}
            {#if doctorLoading && !doctor}
              <span class="text-xs text-fg-subtle">collecting findings…</span>
            {/if}
          </div>
          {#if actionable.length > 0}
            <div class="mt-4 pt-4 border-t border-border space-y-2">
              <div class="text-[11px] uppercase tracking-wide text-fg-subtle">
                How to fix
              </div>
              {#each actionable as f (f.check)}
                <div class="flex items-start gap-2 text-xs">
                  <StatusDot
                    status={f.verdict === "fail" ? "crashed" : "unhealthy"}
                  />
                  <div class="flex-1 min-w-0">
                    <div class="font-mono text-fg-muted">{f.check}</div>
                    <div class="text-fg-muted leading-relaxed">
                      {remediation(f.check)}
                    </div>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </section>
      {/if}

      {#if step === "gallery"}
        <div class="mb-6 flex items-center justify-between">
          <div>
            <h2 class="text-xl font-semibold tracking-tight mb-1">
              Pick a template
            </h2>
            <p class="text-sm text-fg-muted">
              We'll run the upstream scaffolder, then register the new
              project automatically.
            </p>
          </div>
          <button
            type="button"
            onclick={() => (step = "welcome")}
            class="text-xs text-fg-subtle hover:text-fg"
          >
            ← Back
          </button>
        </div>

        {#if chosenKind === null}
          <div class="grid grid-cols-2 gap-3">
            {#each TEMPLATES as tmpl (tmpl.kind)}
              <button
                type="button"
                onclick={() => pickFolderForTemplate(tmpl.kind)}
                class="group flex items-start gap-3 p-4 rounded-lg
                       border border-border bg-surface hover:border-accent/60
                       hover:bg-surface-2 transition-all text-left"
              >
                <div
                  class="w-9 h-9 shrink-0 rounded-md bg-accent/10 text-accent
                         flex items-center justify-center"
                >
                  <Icon name={tmpl.icon as never} size={16} />
                </div>
                <div class="flex-1 min-w-0">
                  <div class="font-medium text-sm mb-0.5">{tmpl.name}</div>
                  <p class="text-xs text-fg-muted leading-relaxed">
                    {tmpl.description}
                  </p>
                  {#if tmpl.requiresBinary}
                    <div
                      class="mt-1.5 text-[10px] text-fg-subtle font-mono"
                    >
                      needs <code>{tmpl.requiresBinary}</code> on PATH
                    </div>
                  {/if}
                </div>
              </button>
            {/each}
          </div>
        {:else}
          <div class="p-5 rounded-xl border border-border bg-surface">
            <div class="mb-4">
              <div class="text-xs uppercase tracking-wide text-fg-subtle mb-1">
                Template
              </div>
              <div class="font-medium">
                {TEMPLATES.find((t) => t.kind === chosenKind)?.name}
              </div>
            </div>
            <div class="mb-4">
              <div class="text-xs uppercase tracking-wide text-fg-subtle mb-1">
                Parent folder
              </div>
              <div class="text-sm font-mono text-fg-muted truncate">
                {chosenParent}
              </div>
            </div>
            <label class="block mb-5">
              <span
                class="text-xs uppercase tracking-wide text-fg-subtle"
              >
                Project name
              </span>
              <input
                type="text"
                bind:value={chosenName}
                placeholder="my-app"
                class="mt-1 w-full px-3 py-2 rounded-md border border-border
                       bg-bg text-sm focus:outline-none focus:border-accent
                       transition-colors"
              />
              <span class="text-xs text-fg-subtle mt-1 block">
                Will create <code class="text-fg-muted"
                  >{chosenParent}/{chosenName || "…"}</code
                >
              </span>
            </label>
            <div class="flex items-center justify-end gap-2">
              <button
                type="button"
                onclick={cancelTemplate}
                class="px-3 py-1.5 text-sm rounded-md text-fg-muted
                       hover:bg-surface-2 transition-colors"
              >
                Pick another
              </button>
              <button
                type="button"
                onclick={runScaffold}
                disabled={!chosenName.trim() || busy}
                class="px-4 py-1.5 text-sm rounded-md bg-accent text-on-accent
                       font-medium hover:opacity-90 disabled:opacity-50
                       transition-opacity inline-flex items-center gap-1.5"
              >
                <Icon name="play" size={12} /> Scaffold
              </button>
            </div>
          </div>
        {/if}
      {/if}

      {#if step === "running"}
        <div class="mb-4">
          <h2 class="text-xl font-semibold tracking-tight mb-1">
            Scaffolding…
          </h2>
          <p class="text-sm text-fg-muted">
            Running the {TEMPLATES.find((t) => t.kind === chosenKind)?.name}
            scaffolder. This typically takes 30–90 seconds.
          </p>
        </div>
        <div
          bind:this={scrollerEl}
          class="h-80 p-3 rounded-lg border border-border bg-bg
                 font-mono text-xs text-fg-muted overflow-y-auto
                 whitespace-pre-wrap leading-relaxed"
        >
          {#each logLines as line, i (i)}<div>{line}</div>{/each}
          {#if logLines.length === 0}
            <span class="text-fg-subtle">waiting for output…</span>
          {/if}
        </div>
        {#if !busy}
          <div class="mt-4 flex items-center justify-end gap-2">
            <button
              type="button"
              onclick={() => {
                logLines = [];
                cancelTemplate();
                step = "gallery";
              }}
              class="px-3 py-1.5 text-sm rounded-md text-fg-muted
                     hover:bg-surface-2 transition-colors"
            >
              Back to templates
            </button>
          </div>
        {/if}
      {/if}
    </div>
  </div>
</div>
