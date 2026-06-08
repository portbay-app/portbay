<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
</script>

<div class="mx-auto mb-2 max-w-2xl rounded-lg border border-border/70 bg-surface-2/40 px-3 py-2.5 text-[12px] text-fg-muted">
  <div class="flex items-center gap-1.5 font-medium text-fg">
    <Icon name="server" size={13} /> No Ollama model on this host yet
  </div>
  <p class="mt-1 leading-relaxed">
    Ollama is installed here but has no model pulled. Pull one on the host, then reopen this tab:
  </p>
  <pre class="mt-1.5 overflow-x-auto rounded bg-surface px-2.5 py-1.5 font-mono text-[11px] text-fg">ollama pull llama3.1</pre>
</div>
