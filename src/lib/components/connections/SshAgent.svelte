<!--
  SshAgent — the Agent tab. A chat with a model running ON the remote host
  (ollama via the cached SSH session), with a per-command approval gate: the
  model proposes a shell command in a ```run block, the user reviews/edits and
  runs it, and the output is fed back. No API keys in PortBay, no third-party
  cloud — inference and execution both happen on the box.

  When the host has no local model, falls back to guidance for using your own
  agent against the host (via PortBay's MCP server) plus an install hint.
-->
<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import {
    agentChat,
    agentClose,
    agentRun,
    openAgent,
    type AgentEvent,
    type AgentInfo,
    type ChatMessage,
  } from "$lib/ssh/agent";

  let {
    connectionId,
    label,
    onClose,
  }: { connectionId: string; label: string; onClose?: () => void } = $props();

  const SYSTEM_PROMPT =
    "You are a careful operations assistant working on a remote Linux host over SSH. " +
    "When you need to inspect or change the system, propose ONE shell command in a fenced " +
    "code block labelled `run`, e.g.\n```run\nls -la /var/log\n```\nThen stop — the command " +
    "only executes after the user approves it, and its output is sent back to you. Prefer " +
    "read-only commands first, and never destructive ones without explaining why. Be concise.";

  type Role = "user" | "assistant";
  interface Turn {
    role: Role;
    content: string;
  }

  let status = $state<"connecting" | "ready" | "nomodel" | "error">("connecting");
  let errorMsg = $state<string | null>(null);
  let info = $state<AgentInfo | null>(null);
  let model = $state("");

  let turns = $state<Turn[]>([]);
  let input = $state("");
  let busy = $state(false);
  let streaming = $state("");
  let pendingCommand = $state<string | null>(null);
  let scroller = $state<HTMLDivElement | null>(null);

  // Pull the first ```run / ```bash / ```sh command out of an assistant reply.
  function extractCommand(content: string): string | null {
    const m = content.match(/```(?:run|bash|sh)\s*\n([\s\S]*?)```/i);
    return m ? m[1].trim() || null : null;
  }

  function formatResult(stdout: string, stderr: string, exitCode: number): string {
    let out = stdout;
    if (stderr.trim()) out += (out ? "\n" : "") + stderr;
    out = out.trim();
    if (out.length > 6000) out = out.slice(0, 6000) + "\n…(truncated)";
    const code = exitCode !== 0 ? `\n[exit ${exitCode}]` : "";
    return (out || "(no output)") + code;
  }

  async function scrollToEnd() {
    await tick();
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  // Stream one assistant turn from the current transcript.
  async function runTurn() {
    busy = true;
    streaming = "";
    let content = "";
    let turnError: string | null = null;
    const messages: ChatMessage[] = [
      { role: "system", content: SYSTEM_PROMPT },
      ...turns.map((t) => ({ role: t.role, content: t.content })),
    ];
    const onEvent = (e: AgentEvent) => {
      if (e.type === "token") {
        content += e.text;
        streaming = content;
        void scrollToEnd();
      } else if (e.type === "done") {
        content = e.content;
      } else if (e.type === "error") {
        turnError = e.message;
      }
    };
    try {
      await agentChat(connectionId, model, messages, info?.port ?? 11434, onEvent);
    } catch (e) {
      turnError =
        e && typeof e === "object" && "whatHappened" in e
          ? String((e as { whatHappened: unknown }).whatHappened)
          : "The model request failed.";
    }
    streaming = "";
    if (turnError && !content) {
      turns = [...turns, { role: "assistant", content: `⚠️ ${turnError}` }];
    } else {
      turns = [...turns, { role: "assistant", content }];
      pendingCommand = extractCommand(content);
    }
    busy = false;
    void scrollToEnd();
  }

  function send() {
    const text = input.trim();
    if (!text || busy) return;
    input = "";
    pendingCommand = null;
    turns = [...turns, { role: "user", content: text }];
    void scrollToEnd();
    void runTurn();
  }

  // Approve + run the proposed command, feed its output back, continue the loop.
  async function approveRun() {
    const cmd = (pendingCommand ?? "").trim();
    if (!cmd || busy) return;
    pendingCommand = null;
    busy = true;
    try {
      const result = await agentRun(connectionId, cmd);
      const output = formatResult(result.stdout, result.stderr, result.exitCode);
      turns = [
        ...turns,
        { role: "user", content: `Output of \`${cmd}\`:\n\n\`\`\`\n${output}\n\`\`\`` },
      ];
      void scrollToEnd();
      await runTurn();
    } catch {
      // agentRun uses invokeQuiet; surface inline rather than a toast.
      turns = [...turns, { role: "user", content: `Running \`${cmd}\` failed.` }];
      busy = false;
    }
  }

  function skipRun() {
    pendingCommand = null;
  }

  function reset() {
    turns = [];
    pendingCommand = null;
    streaming = "";
  }

  function onComposerKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  onMount(() => {
    void (async () => {
      try {
        info = await openAgent(connectionId, label);
        if (info.ollamaModels.length === 0) {
          status = "nomodel";
          return;
        }
        model = info.ollamaModels[0];
        status = "ready";
      } catch (e) {
        status = "error";
        errorMsg =
          e && typeof e === "object" && "whatHappened" in e
            ? String((e as { whatHappened: unknown }).whatHappened)
            : "Couldn't reach this host.";
      }
    })();
  });

  onDestroy(() => agentClose(connectionId));
</script>

<div class="flex h-full min-h-0 flex-col bg-surface">
  <!-- Header -->
  <header class="flex items-center gap-2 border-b border-border/60 px-6 py-3">
    <Icon name="bot" size={15} class="text-fg-muted" />
    <div class="min-w-0 flex-1">
      <h2 class="text-[13px] font-semibold text-fg">Agent</h2>
      <p class="truncate text-[11px] text-fg-subtle">
        {status === "ready"
          ? "Runs on the host · proposes commands you approve"
          : "Server-side model — nothing leaves the host"}
      </p>
    </div>
    {#if status === "ready"}
      <select
        bind:value={model}
        disabled={busy}
        class="h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg disabled:opacity-50"
      >
        {#each info?.ollamaModels ?? [] as m (m)}
          <option value={m}>{m}</option>
        {/each}
      </select>
      <button
        type="button"
        onclick={reset}
        disabled={busy || turns.length === 0}
        class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
      >
        <Icon name="eraser" size={12} /> Clear
      </button>
    {/if}
    {#if onClose}
      <button
        type="button"
        onclick={onClose}
        class="grid h-8 w-8 shrink-0 place-items-center rounded-md text-fg-muted hover:bg-surface-2 hover:text-fg"
        aria-label="Close agent panel"
        title="Close agent panel"
      >
        <Icon name="x" size={15} />
      </button>
    {/if}
  </header>

  {#if status === "connecting"}
    <div class="flex flex-1 items-center justify-center text-[12px] text-fg-subtle">
      <Icon name="refresh-cw" size={14} class="mr-2 animate-spin" /> Connecting…
    </div>
  {:else if status === "error"}
    <div class="flex flex-1 items-center justify-center p-6">
      <div class="max-w-sm rounded-lg border border-status-crashed/40 bg-status-crashed/10 p-4 text-center">
        <Icon name="circle-alert" size={18} class="mx-auto text-status-crashed" />
        <p class="mt-2 text-[12.5px] text-fg">{errorMsg}</p>
      </div>
    </div>
  {:else if status === "nomodel"}
    <!-- A3 fallback: no local model on this host. -->
    <div class="flex-1 overflow-y-auto px-8 py-8">
      <div class="mx-auto max-w-lg rounded-xl border border-border/70 bg-surface px-5 py-5">
        <div class="flex items-center gap-2">
          <Icon name="bot" size={16} class="text-fg-muted" />
          <h3 class="text-[13.5px] font-semibold text-fg">No local model on this host</h3>
        </div>
        <p class="mt-2 text-[12.5px] text-fg-muted leading-relaxed">
          PortBay runs the agent's model on the host itself, so nothing leaves the box and
          there are no API keys.
          {#if info?.hasOllama}
            ollama is installed here but reported no models — pull one below, then reopen this tab.
          {:else if info && !info.hasCurl && !info.hasWget}
            Neither <code class="font-mono">curl</code> nor <code class="font-mono">wget</code> is
            installed, so the agent can't reach a local model API.
          {:else}
            No local model runtime (ollama) was detected on this host.
          {/if}
        </p>
        <div class="mt-4 rounded-lg border border-border/60 bg-surface-2/40 p-3">
          <p class="text-[11.5px] font-medium text-fg">Install a local model (ollama)</p>
          <pre class="mt-1.5 overflow-x-auto rounded bg-surface px-2.5 py-2 font-mono text-[11px] text-fg">curl -fsSL https://ollama.com/install.sh | sh
ollama pull llama3.1</pre>
          <p class="mt-2 text-[11px] text-fg-subtle leading-relaxed">
            Then reopen this tab. For GPU/cluster hosts this is both fast and fully private.
          </p>
        </div>
        <p class="mt-3 text-[11.5px] text-fg-subtle leading-relaxed">
          Prefer your own agent? Point Claude Code / Codex / Cursor at this host through
          PortBay's MCP server and drive it from there — no model needed on the host.
        </p>
      </div>
    </div>
  {:else}
    <!-- Transcript -->
    <div bind:this={scroller} class="min-h-0 flex-1 overflow-y-auto px-6 py-4">
      {#if turns.length === 0 && !streaming}
        <p class="mx-auto mt-8 max-w-md text-center text-[12.5px] text-fg-subtle leading-relaxed">
          Ask the model to inspect or operate this host. It proposes commands; you approve
          each one before it runs. e.g. <span class="text-fg">"what's using the most disk?"</span>
        </p>
      {/if}
      <div class="mx-auto max-w-2xl space-y-3">
        {#each turns as t, i (i)}
          <div class="flex {t.role === 'user' ? 'justify-end' : 'justify-start'}">
            <div
              class="max-w-[85%] rounded-xl px-3.5 py-2 text-[12.5px] leading-relaxed whitespace-pre-wrap break-words
                     {t.role === 'user'
                ? 'bg-accent/10 text-fg'
                : 'border border-border/70 bg-surface text-fg'}"
            >
              {t.content}
            </div>
          </div>
        {/each}
        {#if streaming}
          <div class="flex justify-start">
            <div class="max-w-[85%] rounded-xl border border-border/70 bg-surface px-3.5 py-2 text-[12.5px] leading-relaxed whitespace-pre-wrap break-words text-fg">
              {streaming}<span class="ml-0.5 inline-block h-3 w-1.5 animate-pulse bg-fg-muted align-middle"></span>
            </div>
          </div>
        {/if}
      </div>
    </div>

    <!-- Approval gate -->
    {#if pendingCommand}
      <div class="border-t border-border/60 bg-surface-2/30 px-6 py-3">
        <div class="mx-auto max-w-2xl">
          <div class="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium uppercase text-fg-subtle">
            <Icon name="shield" size={12} /> Command proposed — review before it runs
          </div>
          <textarea
            bind:value={pendingCommand}
            rows="2"
            spellcheck="false"
            class="w-full resize-y rounded-md border border-border bg-surface px-2.5 py-2 font-mono text-[12px] text-fg outline-none focus:border-accent"
          ></textarea>
          <div class="mt-2 flex items-center gap-2">
            <button
              type="button"
              onclick={approveRun}
              disabled={busy}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium bg-accent text-on-accent hover:brightness-110 disabled:opacity-50"
            >
              <Icon name="play" size={12} /> Run on host
            </button>
            <button
              type="button"
              onclick={skipRun}
              disabled={busy}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
            >
              Skip
            </button>
          </div>
        </div>
      </div>
    {/if}

    <!-- Composer -->
    <div class="border-t border-border/60 px-6 py-3">
      <div class="mx-auto flex max-w-2xl items-end gap-2">
        <textarea
          bind:value={input}
          onkeydown={onComposerKeydown}
          rows="1"
          placeholder={busy ? "Working…" : "Ask the agent about this host…"}
          disabled={busy}
          class="min-h-9 max-h-40 flex-1 resize-y rounded-lg border border-border bg-surface px-3 py-2 text-[12.5px] text-fg outline-none focus:border-accent disabled:opacity-60"
        ></textarea>
        <button
          type="button"
          onclick={send}
          disabled={busy || !input.trim()}
          class="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-accent text-on-accent hover:brightness-110 disabled:opacity-50"
          aria-label="Send"
        >
          <Icon name={busy ? "refresh-cw" : "arrow-up"} size={15} class={busy ? "animate-spin" : ""} />
        </button>
      </div>
    </div>
  {/if}
</div>
