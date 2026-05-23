<!--
  AddDatabaseWizard — slide-over panel for provisioning a new database
  instance. Mirrors AddProjectWizard's chrome (backdrop + right panel,
  ESC/×/backdrop to close).

  Flow:
    1. Pick an engine (cards). Engines that aren't installed show an
       Install button that runs `brew install` inline.
    2. Name the instance + optional explicit port (auto-allocated if blank).
    3. Create → PortBay provisions the data dir + config and registers the
       instance, then it appears in the list.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { ErrorEnvelope } from "$lib/components/errors";
  import DatabaseMark from "./DatabaseMark.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { databases } from "$lib/stores/databases.svelte";
  import type {
    CommandError,
  } from "$lib/types/error";
  import type {
    DatabaseEngineId,
    DatabaseEngineView,
  } from "$lib/types/databases";

  let selectedEngine = $state<DatabaseEngineId | null>(null);
  let name = $state<string>("");
  let port = $state<number | null>(null);
  let autoStart = $state<boolean>(true);
  let submitting = $state<boolean>(false);
  let installingEngine = $state<DatabaseEngineId | null>(null);
  let formError = $state<CommandError | null>(null);

  const engine = $derived<DatabaseEngineView | null>(
    databases.engines.find((e) => e.id === selectedEngine) ?? null,
  );

  // Default the name to "<engine>-local" the first time an engine is picked.
  function pickEngine(e: DatabaseEngineView) {
    selectedEngine = e.id;
    if (!name.trim()) name = `${e.id}-local`;
    port = null;
    formError = null;
  }

  async function installEngine(e: DatabaseEngineView) {
    if (installingEngine) return;
    installingEngine = e.id;
    errorBus.push({
      code: "DB_ENGINE_INSTALL",
      whatHappened: `Installing ${e.label} via Homebrew…`,
      whyItMatters: "First install can take a minute or two.",
      whoCausedIt: "system",
      severity: "info",
      actions: [],
    });
    try {
      await safeInvoke("install_database_engine", { engine: e.id });
      await databases.refreshEngines();
      errorBus.push({
        code: "DB_ENGINE_INSTALL_OK",
        whatHappened: `${e.label} installed.`,
        whyItMatters: "You can create an instance now.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    } finally {
      installingEngine = null;
    }
  }

  async function create() {
    if (!selectedEngine) {
      formError = envelope("Pick a database engine first.");
      return;
    }
    if (!engine?.installed) {
      formError = envelope(`Install ${engine?.label ?? "the engine"} first.`);
      return;
    }
    if (!name.trim()) {
      formError = envelope("Give the instance a name.");
      return;
    }
    submitting = true;
    formError = null;
    try {
      await safeInvoke("create_database_instance", {
        input: {
          engine: selectedEngine,
          name: name.trim(),
          port: port ?? null,
          autoStart,
        },
      });
      errorBus.push({
        code: "DB_CREATE_OK",
        whatHappened: `${name.trim()} created.`,
        whyItMatters: autoStart
          ? "It's starting up now."
          : "Press Start in the right pane when you want it running.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await databases.refresh();
      reset();
      databases.hideWizard();
    } catch (e) {
      formError = e as CommandError;
    } finally {
      submitting = false;
    }
  }

  function envelope(msg: string): CommandError {
    return {
      code: "BAD_INPUT",
      whatHappened: msg,
      whyItMatters: "",
      whoCausedIt: "user",
      actions: [],
    };
  }

  function reset() {
    selectedEngine = null;
    name = "";
    port = null;
    autoStart = true;
    formError = null;
  }

  function close() {
    databases.hideWizard();
  }

  function onKeydown(e: KeyboardEvent) {
    if (databases.wizardOpen && e.key === "Escape") close();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if databases.wizardOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="fixed inset-0 z-40 bg-bg/60 backdrop-blur-sm" onclick={close}></div>

  <aside
    class="fixed inset-y-0 right-0 z-50 w-[560px] max-w-[92vw] bg-surface
           border-l border-border shadow-2xl flex flex-col"
    aria-label="Add Database"
  >
    <header
      class="shrink-0 flex items-center justify-between px-5 py-4 border-b border-border"
    >
      <h2 class="text-base font-semibold text-fg">Add database</h2>
      <button
        type="button"
        onclick={close}
        title="Close"
        aria-label="Close add database"
        class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="x" size={16} />
      </button>
    </header>

    <div class="flex-1 min-h-0 overflow-y-auto p-5 space-y-5">
      {#if formError}
        <ErrorEnvelope envelope={formError} tone="inline" />
      {/if}

      <!-- Engine picker -->
      <section>
        <h3 class="text-[11px] uppercase tracking-wide text-fg-subtle mb-2">
          Engine
        </h3>
        <div class="grid grid-cols-1 gap-2">
          {#each databases.engines as e (e.id)}
            {@const isSel = selectedEngine === e.id}
            <div
              role="button"
              tabindex="0"
              onclick={() => pickEngine(e)}
              onkeydown={(ev) => {
                if (ev.key === "Enter" || ev.key === " ") {
                  ev.preventDefault();
                  pickEngine(e);
                }
              }}
              class="flex items-center gap-3 px-3 py-2.5 rounded-lg border
                     text-left transition-colors cursor-pointer
                     focus-visible:outline-none focus-visible:ring-2
                     focus-visible:ring-accent/40
                     {isSel
                ? 'border-accent/60 bg-accent/10'
                : 'border-border bg-surface hover:bg-surface-2'}"
            >
              <DatabaseMark id={e.id} size={30} class="shrink-0" />
              <div class="min-w-0 flex-1">
                <div class="flex items-center gap-2">
                  <span class="text-[13px] font-semibold text-fg">{e.label}</span>
                  {#if e.installed}
                    <span class="text-[10.5px] font-mono text-fg-subtle">
                      {e.version ? `v${e.version}` : "installed"}
                    </span>
                  {/if}
                </div>
                <p class="text-[11px] text-fg-subtle">
                  Default port {e.defaultPort}
                </p>
              </div>
              {#if e.installed}
                {#if isSel}
                  <Icon name="check" size={15} class="text-accent shrink-0" />
                {/if}
              {:else}
                <button
                  type="button"
                  onclick={(ev) => {
                    ev.stopPropagation();
                    void installEngine(e);
                  }}
                  disabled={installingEngine !== null}
                  class="shrink-0 inline-flex items-center gap-1 px-2 h-7 rounded-md
                         border border-accent/40 text-accent text-[11px]
                         hover:bg-accent/10 disabled:opacity-50 transition-colors"
                >
                  {#if installingEngine === e.id}
                    <Icon name="refresh-cw" size={10} class="animate-spin" />
                    Installing
                  {:else}
                    Install
                  {/if}
                </button>
              {/if}
            </div>
          {/each}
        </div>
      </section>

      <!-- Details -->
      {#if engine}
        <section class="space-y-3">
          <h3 class="text-[11px] uppercase tracking-wide text-fg-subtle">
            Instance
          </h3>
          <div>
            <label
              for="db-name"
              class="block text-[11px] font-medium text-fg-muted mb-1.5"
            >
              Name
            </label>
            <input
              id="db-name"
              type="text"
              bind:value={name}
              placeholder="myapp-{engine.id}"
              class="w-full px-3 h-9 rounded-md bg-bg border border-border
                     text-[13px] text-fg placeholder:text-fg-subtle
                     focus:outline-none focus:ring-1 focus:ring-accent/50
                     focus:border-accent/40 transition-colors"
            />
            <p class="mt-1 text-[10.5px] text-fg-subtle">
              Used for the instance id, data directory, and process name.
            </p>
          </div>

          <div>
            <label
              for="db-port"
              class="block text-[11px] font-medium text-fg-muted mb-1.5"
            >
              Port
              <span class="text-fg-subtle font-normal">(blank = auto)</span>
            </label>
            <input
              id="db-port"
              type="number"
              bind:value={port}
              placeholder={engine.defaultPort.toString()}
              class="w-40 px-3 h-9 rounded-md bg-bg border border-border
                     text-[13px] font-mono text-fg placeholder:text-fg-subtle
                     focus:outline-none focus:ring-1 focus:ring-accent/50
                     focus:border-accent/40 transition-colors"
            />
            <p class="mt-1 text-[10.5px] text-fg-subtle">
              PortBay allocates a free port near {engine.defaultPort} when
              left blank.
            </p>
          </div>

          <label class="flex items-center gap-2.5 cursor-pointer select-none">
            <input type="checkbox" bind:checked={autoStart} class="accent-accent" />
            <span class="text-[12.5px] text-fg">
              Start automatically when PortBay launches
            </span>
          </label>
        </section>
      {/if}
    </div>

    <footer
      class="shrink-0 flex items-center justify-end gap-2 px-5 py-4 border-t border-border"
    >
      <button
        type="button"
        onclick={close}
        class="px-3 h-9 rounded-md border border-border text-[12.5px]
               text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors"
      >
        Cancel
      </button>
      <button
        type="button"
        onclick={create}
        disabled={submitting || !engine?.installed || !name.trim()}
        class="inline-flex items-center gap-2 px-4 h-9 rounded-md bg-accent
               text-on-accent text-[12.5px] font-medium
               hover:brightness-110 active:brightness-95
               disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
      >
        {#if submitting}
          <Icon name="refresh-cw" size={12} class="animate-spin" />
          Creating…
        {:else}
          <Icon name="plus" size={13} />
          Create database
        {/if}
      </button>
    </footer>
  </aside>
{/if}
