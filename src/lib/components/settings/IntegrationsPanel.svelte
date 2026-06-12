<!--
  IntegrationsPanel — two stacked cards:
    1. Task board agents — global dispatch defaults (preferred agent + terminal)
       and the agent/LLM detection review (locate a binary, re-scan).
    2. MCP setup snippets for each supported AI client.

  Smart Dictation moved to the AI page (SmartDictationPanel) so all local-AI
  consumers are managed in one place.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { openUrl } from "$lib/security/openUrl";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { devTools } from "$lib/stores/devTools.svelte";
  import { AGENT_ICONS, type AgentOption } from "$lib/ssh/agentModels";
  import Popover from "$lib/components/atoms/Popover.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

  /** Bundled logos for the detected terminals (ids from `integrations.rs`). */
  const TERMINAL_ICONS: Record<string, string> = {
    warp: "/apps/warp.png",
    iterm: "/apps/iterm2.png",
    ghostty: "/apps/ghostty.png",
    terminal: "/apps/terminal.png",
  };

  const MCP_FALLBACK_PATH = "/Applications/PortBay.app/Contents/MacOS/portbay-mcp";
  let mcpPath = $state<string>(MCP_FALLBACK_PATH);
  /** Which copy button last fired; resets after 1.5 s for the check-mark feedback. */
  let copiedKey = $state<string | null>(null);

  // --- Task board: agent detection + global dispatch defaults --------------
  let agents = $state<AgentOption[]>([]);
  let scanning = $state(false);

  /** One row in the detected list: a specific *form* of an agent (its CLI or its
   * desktop app). An agent with both forms contributes two rows; `selectable`
   * marks the rows the user chooses between (the per-agent launch preference). */
  type AgentForm = {
    agent: AgentOption;
    mode: "cli" | "app";
    formLabel: string;
    selectable: boolean;
    isCli: boolean;
  };
  // Only show what we found — plus `custom`, the always-available per-project
  // command. Each detected agent is expanded into its installed forms so "Codex
  // CLI" and "Codex Desktop" are listed (and picked) separately.
  const detectedForms = $derived<AgentForm[]>(
    agents
      .filter((a) => a.installed)
      .flatMap((a): AgentForm[] => {
        if (a.id === "custom")
          return [{ agent: a, mode: "cli", formLabel: a.label, selectable: false, isCli: true }];
        const both = a.cliInstalled && a.appInstalled;
        const forms: AgentForm[] = [];
        if (a.cliInstalled)
          forms.push({ agent: a, mode: "cli", formLabel: `${a.label} CLI`, selectable: both, isCli: true });
        if (a.appInstalled)
          forms.push({ agent: a, mode: "app", formLabel: `${a.label} Desktop`, selectable: both, isCli: false });
        return forms;
      }),
  );
  // Selectable preferred-agent choices: one entry per installed form (CLI and/or
  // Desktop), excluding the template `custom`. Picking one sets both the preferred
  // agent and that agent's launch form (see `selectPreferredForm`).
  const preferredAgentForms = $derived(
    detectedForms.filter((f) => f.agent.id !== "custom"),
  );
  // Tools we couldn't find in any form go behind a disclosure so the list isn't
  // a wall of "Not found".
  const undetectedAgents = $derived(
    agents.filter((a) => !a.installed && a.id !== "custom"),
  );
  const terminals = $derived(devTools.value.filter((t) => t.kind === "terminal"));

  // Currently-selected entries, for the picker triggers (undefined → "Auto").
  const selectedAgent = $derived(
    preferences.value.preferredAgent
      ? agents.find((a) => a.id === preferences.value.preferredAgent)
      : undefined,
  );
  const selectedTerminal = $derived(
    preferences.value.preferredTerminal
      ? terminals.find((t) => t.id === preferences.value.preferredTerminal)
      : undefined,
  );
  /** Form suffix ("CLI" / "Desktop") for an agent option's current launch mode. */
  const formSuffix = (a: AgentOption) => (a.mode === "app" ? "Desktop" : "CLI");

  /** Pick which form (CLI vs Desktop) an agent dispatches as — persisted as the
   * per-agent launch preference. */
  async function selectForm(agentId: string, mode: "cli" | "app") {
    agents = await safeInvoke<AgentOption[]>("set_agent_launch_mode", {
      agent: agentId,
      mode,
    });
  }

  /** Make a specific form the global default: persist the agent's launch form,
   * then mark the agent preferred. The two prefs share one source of truth, so
   * the "in use" form in the detected list stays in sync. */
  async function selectPreferredForm(agentId: string, mode: "cli" | "app") {
    agents = await safeInvoke<AgentOption[]>("set_agent_launch_mode", {
      agent: agentId,
      mode,
    });
    preferences.update({ preferredAgent: agentId });
  }

  async function loadAgents() {
    agents = await safeInvoke<AgentOption[]>("agents_installed").catch(() => []);
  }

  async function rescan() {
    scanning = true;
    try {
      await loadAgents();
    } finally {
      scanning = false;
    }
  }

  /** Point an agent at a binary the auto-scan missed (external drive / custom prefix). */
  async function locateAgent(agentId: string, label: string) {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        multiple: false,
        directory: false,
        title: `Locate the ${label} binary`,
      });
      if (typeof result !== "string") return;
      agents = await safeInvoke<AgentOption[]>("set_agent_path", {
        agent: agentId,
        path: result,
      });
    } catch {
      /* dialog / validation already surfaced a toast */
    }
  }

  async function clearAgentOverride(agentId: string) {
    agents = await safeInvoke<AgentOption[]>("clear_agent_path", { agent: agentId });
  }

  const claudeCodeSnippet = $derived(
    `claude mcp add --transport stdio --scope user portbay -- ${mcpPath}`,
  );

  const claudeDesktopSnippet = $derived(
    JSON.stringify(
      { mcpServers: { portbay: { command: mcpPath, args: [], env: {} } } },
      null,
      2,
    ),
  );

  const cursorSnippet = $derived(
    JSON.stringify(
      { mcpServers: { portbay: { command: mcpPath, args: [], env: {} } } },
      null,
      2,
    ),
  );

  const vscodeSnippet = $derived(
    JSON.stringify(
      {
        servers: {
          portbay: { type: "stdio", command: mcpPath, args: [], env: {} },
        },
      },
      null,
      2,
    ),
  );

  // Codex uses TOML at ~/.codex/config.toml (mirrors docs-site/agents).
  const codexSnippet = $derived(`[mcp_servers.portbay]\ncommand = "${mcpPath}"`);

  // Antigravity (~/.gemini/antigravity/mcp_config.json) and OpenClaw
  // (~/.openclaw/openclaw.json) both take the standard `mcpServers` JSON shape.
  const antigravitySnippet = $derived(
    JSON.stringify(
      { mcpServers: { portbay: { command: mcpPath, args: [], env: {} } } },
      null,
      2,
    ),
  );
  const openclawSnippet = $derived(
    JSON.stringify(
      { mcpServers: { portbay: { command: mcpPath, args: [], env: {} } } },
      null,
      2,
    ),
  );

  // Hermes reads a YAML `mcp_servers` block from its config.yaml.
  const hermesSnippet = $derived(
    `mcp_servers:\n  portbay:\n    command: "${mcpPath}"\n    args: []`,
  );

  // The integration panel shows one environment at a time, chosen from a
  // dropdown — most users wire up a single agent, so listing every client's
  // config at once just buried the one snippet they needed.
  type EnvKey =
    | "antigravity"
    | "claude-code"
    | "claude-desktop"
    | "codex"
    | "cursor"
    | "hermes"
    | "openclaw"
    | "vscode";
  let selectedEnv = $state<EnvKey>("claude-code");
  /** Whether the environment picker menu is open (mirrors the "Open in" menu). */
  let envMenuOpen = $state<boolean>(false);

  interface IntegrationEnv {
    key: EnvKey;
    label: string;
    /** App logo (served from static/apps), reused from the "Open in" menu.
     *  Omitted for clients we don't ship a brand mark for — a generic icon
     *  stands in. */
    logo?: string;
    /** Shown instead of a config path for command-style setup (Claude Code). */
    runHint?: string;
    /** Config file the snippet is pasted into. */
    configPath?: string;
    snippet: string;
    /** Optional one-click install affordance. */
    deepLink?: { run: () => void; label: string; note: string };
  }

  // Listed alphabetically by label.
  const integrationEnvs = $derived<IntegrationEnv[]>([
    {
      key: "antigravity",
      label: "Antigravity",
      logo: "/apps/antigravity.png",
      configPath: "~/.gemini/antigravity/mcp_config.json",
      snippet: antigravitySnippet,
    },
    {
      key: "claude-code",
      label: "Claude Code",
      logo: "/apps/claude.png",
      runHint: "Run once in any terminal to register PortBay for all projects.",
      snippet: claudeCodeSnippet,
    },
    {
      key: "claude-desktop",
      label: "Claude Desktop",
      logo: "/apps/claude.png",
      configPath:
        "~/Library/Application Support/Claude/claude_desktop_config.json",
      snippet: claudeDesktopSnippet,
    },
    {
      key: "codex",
      label: "Codex",
      logo: "/apps/codex.png",
      configPath: "~/.codex/config.toml",
      snippet: codexSnippet,
    },
    {
      key: "cursor",
      label: "Cursor",
      logo: "/apps/cursor.png",
      configPath: "~/.cursor/mcp.json",
      snippet: cursorSnippet,
      deepLink: {
        run: openCursorDeepLink,
        label: "Add to Cursor",
        note: "opens Cursor's MCP install flow.",
      },
    },
    {
      key: "hermes",
      label: "Hermes",
      configPath: "Hermes config.yaml (mcp_servers)",
      snippet: hermesSnippet,
    },
    {
      key: "openclaw",
      label: "OpenClaw",
      configPath: "~/.openclaw/openclaw.json",
      snippet: openclawSnippet,
    },
    {
      key: "vscode",
      label: "VS Code",
      logo: "/apps/vscode.png",
      configPath: ".vscode/mcp.json",
      snippet: vscodeSnippet,
      deepLink: {
        run: openVscodeDeepLink,
        label: "Add to VS Code",
        note: "attempts VS Code's MCP install deep link (best-effort; copy the JSON if it doesn't open).",
      },
    },
  ]);

  const activeEnv = $derived(
    integrationEnvs.find((e) => e.key === selectedEnv) ?? integrationEnvs[0],
  );

  function selectEnv(key: EnvKey) {
    selectedEnv = key;
    envMenuOpen = false;
  }

  // Close the picker on outside-click / Escape — matches the "Open in" menu.
  function onEnvWindowClick(e: MouseEvent) {
    if (!envMenuOpen) return;
    const t = e.target as Element | null;
    if (t && t.closest("[data-env-picker]")) return;
    envMenuOpen = false;
  }
  function onEnvWindowKey(e: KeyboardEvent) {
    if (envMenuOpen && e.key === "Escape") envMenuOpen = false;
  }

  async function copySnippet(text: string, key: string) {
    try {
      await navigator.clipboard.writeText(text);
      copiedKey = key;
      setTimeout(() => {
        if (copiedKey === key) copiedKey = null;
      }, 1_500);
      errorBus.push({
        code: "COPIED",
        category: "agent-board",
        whatHappened: "Snippet copied.",
        whyItMatters: "Paste it into your tool's config.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* clipboard unavailable */
    }
  }

  function openCursorDeepLink() {
    const config = btoa(JSON.stringify({ command: mcpPath, args: [], env: {} }));
    void openUrl(
      `cursor://anysphere.cursor-deeplink/mcp/install?name=portbay&config=${config}`,
    );
  }

  function openVscodeDeepLink() {
    const config = encodeURIComponent(
      JSON.stringify({ type: "stdio", command: mcpPath, args: [], env: {} }),
    );
    void openUrl(
      `https://insiders.vscode.dev/redirect/mcp/install?name=portbay&config=${config}`,
    );
  }

  onMount(() => {
    void safeInvoke<string | null>("resolve_mcp_binary_path").then((resolved) => {
      if (resolved) mcpPath = resolved;
    });
    void loadAgents();
    void devTools.start();
    void preferences.load();
  });
</script>

<svelte:window onclick={onEnvWindowClick} onkeydown={onEnvWindowKey} />

<div class="space-y-6">
  <SettingsPanel
    title="Task board agents"
    description="Global defaults for dispatching agents from card boards, plus which agent/LLM CLIs PortBay detects on this machine."
  >
    <div class="space-y-5">
      <!-- Global dispatch defaults -->
      <div class="divide-y divide-border/60">
        <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
          <div class="min-w-0">
            <span class="text-[13px] text-fg">Preferred agent</span>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              Default agent dispatched for new project boards. Each board can
              override it in its own automation settings.
            </p>
          </div>
          <Popover align="right" width="14rem">
            {#snippet trigger(toggle, open)}
              <button
                type="button"
                onclick={toggle}
                aria-expanded={open}
                class="h-8 w-56 shrink-0 inline-flex items-center gap-2 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg hover:border-accent/60 transition-colors"
              >
                {#if selectedAgent}
                  {#if AGENT_ICONS[selectedAgent.id]}
                    <img src={AGENT_ICONS[selectedAgent.id]} alt="" class="w-4 h-4 rounded-[3px] object-cover shrink-0" />
                  {:else}
                    <Icon name="sparkles" size={13} class="shrink-0 text-fg-subtle" />
                  {/if}
                  <span class="truncate">{selectedAgent.label} {formSuffix(selectedAgent)}</span>
                {:else}
                  <Icon name="sparkles" size={13} class="shrink-0 text-fg-subtle" />
                  <span class="truncate text-fg-muted">Auto (Claude Code)</span>
                {/if}
                <Icon name="chevron-down" size={13} class="ml-auto shrink-0 text-fg-subtle" />
              </button>
            {/snippet}
            {#snippet children(close)}
              <div class="space-y-0.5 min-w-[13rem]">
                <button
                  type="button"
                  onclick={() => { preferences.update({ preferredAgent: null }); close(); }}
                  class="w-full text-left rounded px-2 py-1 text-[12px] flex items-center gap-2 hover:bg-surface-2 {!preferences.value.preferredAgent ? 'text-fg font-medium' : 'text-fg-muted'}"
                >
                  <Icon name="sparkles" size={13} class="shrink-0 text-fg-subtle" /> Auto (Claude Code)
                </button>
                {#each preferredAgentForms as f (f.agent.id + f.mode)}
                  <button
                    type="button"
                    onclick={() => { void selectPreferredForm(f.agent.id, f.mode); close(); }}
                    class="w-full text-left rounded px-2 py-1 text-[12px] flex items-center gap-2 hover:bg-surface-2 {preferences.value.preferredAgent === f.agent.id && f.agent.mode === f.mode ? 'text-fg font-medium' : 'text-fg-muted'}"
                  >
                    {#if AGENT_ICONS[f.agent.id]}
                      <img src={AGENT_ICONS[f.agent.id]} alt="" class="w-4 h-4 rounded-[3px] object-cover shrink-0" />
                    {:else}
                      <Icon name="sparkles" size={13} class="shrink-0" />
                    {/if}
                    {f.formLabel}
                  </button>
                {/each}
              </div>
            {/snippet}
          </Popover>
        </div>

        <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
          <div class="min-w-0">
            <span class="text-[13px] text-fg">Terminal for agents</span>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              Where an interactive agent (e.g. Claude Code) opens when a card is
              dispatched. Independent of which agent runs.
            </p>
          </div>
          <Popover align="right" width="14rem">
            {#snippet trigger(toggle, open)}
              <button
                type="button"
                onclick={toggle}
                aria-expanded={open}
                class="h-8 w-56 shrink-0 inline-flex items-center gap-2 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg hover:border-accent/60 transition-colors"
              >
                {#if selectedTerminal}
                  {#if TERMINAL_ICONS[selectedTerminal.id]}
                    <img src={TERMINAL_ICONS[selectedTerminal.id]} alt="" class="w-4 h-4 rounded-[3px] object-cover shrink-0" />
                  {:else}
                    <Icon name="terminal" size={13} class="shrink-0 text-fg-subtle" />
                  {/if}
                  <span class="truncate">{selectedTerminal.label}</span>
                {:else}
                  <Icon name="terminal" size={13} class="shrink-0 text-fg-subtle" />
                  <span class="truncate text-fg-muted">Auto (first detected)</span>
                {/if}
                <Icon name="chevron-down" size={13} class="ml-auto shrink-0 text-fg-subtle" />
              </button>
            {/snippet}
            {#snippet children(close)}
              <div class="space-y-0.5 min-w-[13rem]">
                <button
                  type="button"
                  onclick={() => { preferences.update({ preferredTerminal: null }); close(); }}
                  class="w-full text-left rounded px-2 py-1 text-[12px] flex items-center gap-2 hover:bg-surface-2 {!preferences.value.preferredTerminal ? 'text-fg font-medium' : 'text-fg-muted'}"
                >
                  <Icon name="terminal" size={13} class="shrink-0 text-fg-subtle" /> Auto (first detected)
                </button>
                {#each terminals as t (t.id)}
                  <button
                    type="button"
                    onclick={() => { preferences.update({ preferredTerminal: t.id }); close(); }}
                    class="w-full text-left rounded px-2 py-1 text-[12px] flex items-center gap-2 hover:bg-surface-2 {preferences.value.preferredTerminal === t.id ? 'text-fg font-medium' : 'text-fg-muted'}"
                  >
                    {#if TERMINAL_ICONS[t.id]}
                      <img src={TERMINAL_ICONS[t.id]} alt="" class="w-4 h-4 rounded-[3px] object-cover shrink-0" />
                    {:else}
                      <Icon name="terminal" size={13} class="shrink-0" />
                    {/if}
                    {t.label}
                  </button>
                {/each}
              </div>
            {/snippet}
          </Popover>
        </div>

        <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
          <div class="min-w-0">
            <span class="text-[13px] text-fg">Run inside tmux</span>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              Wrap interactive dispatches in a <code>portbay-&lt;run&gt;</code> tmux
              session so a run survives closing its window — detach and reattach
              like the remote tmux sessions in the SSH Jobs panel. Falls back to a
              plain window when tmux isn't installed.
            </p>
          </div>
          <Toggle
            checked={preferences.value.dispatchMultiplexer === "tmux"}
            label="Run interactive dispatches inside tmux"
            onchange={(next) =>
              preferences.update({ dispatchMultiplexer: next ? "tmux" : null })}
          />
        </div>
      </div>

      <!-- Detection review -->
      <div class="space-y-2">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <span class="text-[12px] font-semibold text-fg uppercase tracking-wide">Detected tools</span>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              Only detected agents can be assigned to cards. Installed elsewhere
              and not showing? Re-scan, or locate it below.
            </p>
          </div>
          <button
            type="button"
            onclick={rescan}
            disabled={scanning}
            class="inline-flex items-center gap-1.5 shrink-0 h-8 px-2.5 rounded-md border border-border text-[12px] text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50 transition-colors"
          >
            <Icon name="refresh-cw" size={13} /> Re-scan
          </button>
        </div>

        <ul class="divide-y divide-border/60 rounded-lg border border-border overflow-hidden">
          {#each detectedForms as f (f.agent.id + ":" + f.mode)}
            {@const a = f.agent}
            {@const active = a.mode === f.mode}
            <li class="flex items-center gap-3 px-3 py-2">
              {#if AGENT_ICONS[a.id]}
                <img src={AGENT_ICONS[a.id]} alt="" class="w-5 h-5 rounded-[4px] object-cover shrink-0" />
              {:else}
                <span class="w-5 h-5 rounded-[4px] bg-fg-muted/15 inline-flex items-center justify-center text-fg-muted shrink-0"><Icon name="sparkles" size={11} /></span>
              {/if}
              <div class="min-w-0 flex-1">
                <div class="flex items-center gap-2">
                  <span class="text-[13px] text-fg">{f.formLabel}</span>
                  {#if a.id === "custom"}
                    <span class="text-[10px] rounded px-1.5 py-0.5 bg-surface-2 text-fg-subtle">per-project command</span>
                  {:else}
                    <span class="text-[10px] rounded px-1.5 py-0.5 bg-green-500/15 text-green-400">Detected</span>
                    {#if f.isCli && a.overridden}<span class="text-[10px] rounded px-1.5 py-0.5 bg-surface-2 text-fg-subtle">custom path</span>{/if}
                  {/if}
                </div>
                {#if f.isCli && a.path}
                  <div class="text-[11px] font-mono text-fg-subtle truncate" title={a.path}>{a.path}</div>
                {/if}
              </div>
              <div class="flex items-center gap-1.5 shrink-0">
                {#if f.selectable}
                  <button
                    type="button"
                    onclick={() => selectForm(a.id, f.mode)}
                    aria-pressed={active}
                    title={active ? "This form is used for dispatch" : "Use this form for dispatch"}
                    class="inline-flex items-center gap-1.5 h-7 px-2 rounded-md border text-[11.5px] transition-colors {active
                      ? 'border-accent/60 text-accent bg-accent/10'
                      : 'border-border text-fg-subtle hover:text-fg hover:bg-surface-2'}"
                  >
                    <span class="w-2 h-2 rounded-full {active ? 'bg-accent' : 'bg-fg-muted/40'}"></span>
                    {active ? "In use" : "Use"}
                  </button>
                {/if}
                {#if f.isCli && a.id !== "custom"}
                  <button
                    type="button"
                    onclick={() => locateAgent(a.id, a.label)}
                    class="h-7 px-2 rounded-md border border-border text-[11.5px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
                  >
                    {a.path ? "Change…" : "Locate…"}
                  </button>
                  {#if a.overridden}
                    <button
                      type="button"
                      onclick={() => clearAgentOverride(a.id)}
                      class="h-7 px-2 rounded-md border border-border text-[11.5px] text-fg-subtle hover:text-red-400 transition-colors"
                    >
                      Clear
                    </button>
                  {/if}
                {/if}
              </div>
            </li>
          {/each}
        </ul>

        {#if undetectedAgents.length > 0}
          <details class="group rounded-lg border border-border/60">
            <summary class="flex items-center gap-1.5 px-3 py-2 text-[11.5px] text-fg-muted hover:text-fg cursor-pointer select-none list-none">
              <Icon name="chevron-right" size={13} class="transition-transform group-open:rotate-90" />
              Tool installed elsewhere? Locate it ({undetectedAgents.length})
            </summary>
            <ul class="divide-y divide-border/60 border-t border-border/60">
              {#each undetectedAgents as a (a.id)}
                <li class="flex items-center gap-3 px-3 py-2">
                  {#if AGENT_ICONS[a.id]}
                    <img src={AGENT_ICONS[a.id]} alt="" class="w-5 h-5 rounded-[4px] object-cover shrink-0 opacity-60" />
                  {:else}
                    <span class="w-5 h-5 rounded-[4px] bg-fg-muted/15 inline-flex items-center justify-center text-fg-muted shrink-0"><Icon name="sparkles" size={11} /></span>
                  {/if}
                  <div class="min-w-0 flex-1 flex items-center gap-2">
                    <span class="text-[13px] text-fg-muted">{a.label}</span>
                    <span class="text-[10px] rounded px-1.5 py-0.5 bg-surface-2 text-fg-subtle">Not found</span>
                  </div>
                  <button
                    type="button"
                    onclick={() => locateAgent(a.id, a.label)}
                    class="h-7 px-2 rounded-md border border-border text-[11.5px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors shrink-0"
                  >
                    Locate…
                  </button>
                </li>
              {/each}
            </ul>
          </details>
        {/if}
      </div>
    </div>
  </SettingsPanel>

  <SettingsPanel
    title="AI Integrations"
    description="Connect your MCP-aware agent to drive PortBay without touching the app."
>
  <div class="space-y-4">
    <!-- Intro -->
    <p class="text-[13px] text-fg-muted leading-relaxed">
      <button
        type="button"
        onclick={() => openUrl("https://docs.portbay.app/agents/")}
        class="text-accent hover:underline"
      >
        Full setup guide →
      </button>
    </p>

    <!-- Security callout -->
    <div
      class="flex items-start gap-2.5 rounded-xl border border-amber-500/30
             bg-amber-500/8 px-3.5 py-2.5"
    >
      <span
        class="inline-flex items-center justify-center w-5 h-5 shrink-0 mt-0.5
               rounded-full bg-amber-500/15 text-amber-400"
      >
        <Icon name="circle-alert" size={11} />
      </span>
      <p class="text-[12px] text-fg-muted leading-relaxed">
        The MCP server runs as your macOS user with your full filesystem access.
        Only connect AI tools you trust.
      </p>
    </div>

    <!-- Environment selector — one client's config shown at a time -->
    <div class="space-y-3">
      <div class="flex flex-col gap-1.5">
        <span class="text-[12px] font-medium text-fg-muted">
          Select your environment
        </span>
        <!-- Logo + name picker (mirrors the project "Open in" menu) -->
        <div class="relative w-full max-w-72" data-env-picker>
          <button
            type="button"
            onclick={() => (envMenuOpen = !envMenuOpen)}
            aria-haspopup="menu"
            aria-expanded={envMenuOpen}
            class="w-full inline-flex items-center gap-2 px-2.5 h-9 rounded-md
                   border border-border bg-bg hover:bg-surface-2 text-[12.5px]
                   text-fg transition-colors"
          >
            {#if activeEnv.logo}
              <img
                src={activeEnv.logo}
                alt=""
                class="w-4 h-4 rounded-[3px] object-cover flex-shrink-0"
              />
            {:else}
              <span
                class="w-4 h-4 rounded-[3px] inline-flex items-center justify-center
                       bg-fg-muted/15 text-fg-muted flex-shrink-0"
              >
                <Icon name="sparkles" size={11} />
              </span>
            {/if}
            <span class="flex-1 text-left truncate">{activeEnv.label}</span>
            <Icon
              name="chevron-down"
              size={12}
              class="text-fg-subtle shrink-0 transition-transform {envMenuOpen
                ? 'rotate-180'
                : ''}"
            />
          </button>

          {#if envMenuOpen}
            <div
              role="menu"
              aria-label="Select your environment"
              class="absolute z-30 mt-1 w-full py-1 rounded-md border
                     border-border bg-surface shadow-2xl"
            >
              {#each integrationEnvs as env (env.key)}
                {@const isSel = env.key === selectedEnv}
                <button
                  type="button"
                  role="menuitemradio"
                  aria-checked={isSel}
                  onclick={() => selectEnv(env.key)}
                  class="w-full text-left px-2.5 py-1.5 flex items-center gap-2
                         text-[12.5px] transition-colors
                         {isSel
                    ? 'text-fg bg-surface-2/60'
                    : 'text-fg-muted hover:text-fg hover:bg-surface-2'}"
                >
                  {#if env.logo}
                    <img
                      src={env.logo}
                      alt=""
                      class="w-4 h-4 rounded-[3px] object-cover flex-shrink-0"
                    />
                  {:else}
                    <span
                      class="w-4 h-4 rounded-[3px] inline-flex items-center justify-center
                             bg-fg-muted/15 text-fg-muted flex-shrink-0"
                    >
                      <Icon name="sparkles" size={11} />
                    </span>
                  {/if}
                  <span class="flex-1 truncate">{env.label}</span>
                  {#if isSel}
                    <Icon name="check" size={12} class="text-accent shrink-0" />
                  {/if}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      </div>

      <!-- Selected environment config -->
      <div class="space-y-1.5">
        {#if activeEnv.runHint}
          <p class="text-[11.5px] text-fg-subtle">{activeEnv.runHint}</p>
        {:else if activeEnv.configPath}
          <p class="text-[11.5px] text-fg-subtle">
            Add to
            <code class="font-mono text-fg-muted break-all">{activeEnv.configPath}</code>
          </p>
        {/if}

        <div class="flex items-start gap-2">
          <pre
            class="flex-1 min-w-0 rounded-lg bg-bg border
                   border-border px-3 py-2 text-[11.5px] font-mono text-fg
                   leading-relaxed whitespace-pre-wrap break-all">{activeEnv.snippet}</pre>
          <div class="flex flex-col gap-1.5 shrink-0">
            <button
              type="button"
              onclick={() => copySnippet(activeEnv.snippet, activeEnv.key)}
              aria-label="Copy {activeEnv.label} config"
              title="Copy"
              class="inline-flex items-center justify-center w-8 h-8
                     rounded-md border border-border text-fg-muted
                     hover:text-fg hover:bg-surface-2 transition-colors"
            >
              <Icon name={copiedKey === activeEnv.key ? "check" : "copy"} size={13} />
            </button>
            {#if activeEnv.deepLink}
              <button
                type="button"
                onclick={activeEnv.deepLink.run}
                aria-label={activeEnv.deepLink.label}
                title={activeEnv.deepLink.label}
                class="inline-flex items-center justify-center w-8 h-8
                       rounded-md border border-border text-fg-muted
                       hover:text-fg hover:bg-surface-2 transition-colors"
              >
                <Icon name="zap" size={13} />
              </button>
            {/if}
          </div>
        </div>

        {#if activeEnv.deepLink}
          <p class="text-[11px] text-fg-subtle">
            The one-click button (<Icon name="zap" size={11} />)
            {activeEnv.deepLink.note}
          </p>
        {/if}
      </div>
    </div>
  </div>
  </SettingsPanel>
</div>
