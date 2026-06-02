<script lang="ts">
  interface Props {
    value: unknown;
    label?: string;
  }

  let { value, label = "value" }: Props = $props();

  function isRecord(v: unknown): v is Record<string, unknown> {
    return typeof v === "object" && v !== null && !Array.isArray(v);
  }

  function entries(v: unknown): Array<[string, unknown]> {
    if (Array.isArray(v)) return v.map((item, index) => [String(index), item]);
    if (isRecord(v)) return Object.entries(v);
    return [];
  }

  function scalar(v: unknown): string {
    if (v === null) return "null";
    if (typeof v === "string") return JSON.stringify(v);
    if (typeof v === "number" || typeof v === "boolean") return String(v);
    return JSON.stringify(v);
  }

  function branchLabel(v: unknown): string {
    if (Array.isArray(v)) return `Array(${v.length})`;
    if (isRecord(v)) return `Object(${Object.keys(v).length})`;
    return scalar(v);
  }
</script>

{#snippet node(name: string, item: unknown)}
  {#if Array.isArray(item) || isRecord(item)}
    <details open class="pl-3 border-l border-border/60">
      <summary class="cursor-pointer select-none text-fg-muted">
        <span class="font-mono text-accent">{name}</span>
        <span class="text-fg-subtle">: {branchLabel(item)}</span>
      </summary>
      <div class="mt-1 space-y-1">
        {#each entries(item) as [childName, child] (childName)}
          {@render node(childName, child)}
        {/each}
      </div>
    </details>
  {:else}
    <div class="pl-3 border-l border-border/60">
      <span class="font-mono text-accent">{name}</span>
      <span class="text-fg-subtle">: </span>
      <span class="font-mono text-fg">{scalar(item)}</span>
    </div>
  {/if}
{/snippet}

<div class="text-[11px] leading-relaxed">
  {@render node(label, value)}
</div>
