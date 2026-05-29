<!-- IntegrationsPanel — MCP setup snippets for each supported AI client. -->
<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { openUrl } from "$lib/security/openUrl";
  import SettingsPanel from "./SettingsPanel.svelte";

  const MCP_FALLBACK_PATH = "/Applications/PortBay.app/Contents/MacOS/portbay-mcp";
  let mcpPath = $state<string>(MCP_FALLBACK_PATH);
  /** Which copy button last fired; resets after 1.5 s for the check-mark feedback. */
  let copiedKey = $state<string | null>(null);

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
  });
</script>

<svelte:window onclick={onEnvWindowClick} onkeydown={onEnvWindowKey} />

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
