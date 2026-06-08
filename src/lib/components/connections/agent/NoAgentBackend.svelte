<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { AgentInfo } from "$lib/ssh/agent";

  let { info }: { info: AgentInfo | null } = $props();
</script>

<div class="flex-1 overflow-y-auto px-8 py-8">
  <div class="mx-auto max-w-lg rounded-xl border border-border/70 bg-surface px-5 py-5">
    <div class="flex items-center gap-2">
      <Icon name="bot" size={16} class="text-fg-muted" />
      <h3 class="text-[13.5px] font-semibold text-fg">No agent available on this host</h3>
    </div>
    <p class="mt-2 text-[12.5px] text-fg-muted leading-relaxed">
      PortBay runs the agent's model on the host itself, so nothing leaves the box and there are
      no API keys. No local model (ollama) and no installed agent CLI
      (<code class="font-mono">claude</code> / <code class="font-mono">codex</code>) were detected
      here.
      {#if info?.hasOllama}
        ollama is installed but reported no models — pull one below, then reopen this tab.
      {:else if info && !info.hasCurl && !info.hasWget}
        Neither <code class="font-mono">curl</code> nor <code class="font-mono">wget</code> is
        installed, so the agent can't reach a local model API.
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
      Prefer your own agent? Point Claude Code / Codex / Cursor at this host through PortBay's MCP
      server and drive it from there — no model needed on the host.
    </p>
  </div>
</div>
