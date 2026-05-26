<!--
  HostnameField — Cloudflare-style split hostname editor: an editable subdomain
  prefix plus the (selectable) domain suffix, composed into the full hostname.
  The user edits only the part before the dot; the suffix is fixed/selected.

  Binds `value` to the full composed hostname (e.g. "api.portbay.test") so call
  sites keep a single hostname string. Matches the /domains page UX so project
  editing and domain editing read identically.
-->
<script lang="ts">
  import { untrack } from "svelte";

  interface Props {
    /** Bindable full hostname, e.g. "api.portbay.test". */
    value: string;
    /** Active system DNS suffix (e.g. "portbay.test"). */
    systemSuffix: string;
    id?: string;
    disabled?: boolean;
    /** Notified whenever the prefix validity changes (for save guards). */
    onValidChange?: (valid: boolean) => void;
    /** Fired when the user edits the prefix/suffix (not on external value sync). */
    onInput?: () => void;
  }
  let {
    value = $bindable(),
    systemSuffix,
    id = "hostname-field",
    disabled = false,
    onValidChange,
    onInput,
  }: Props = $props();

  // Mirror the label rules in src-tauri/src/domain.rs: each dot-separated label
  // is 1–63 chars of [a-z0-9-] with no leading/trailing hyphen.
  const LABEL_RE = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/;
  function isValidSubPrefix(prefix: string): boolean {
    const p = prefix.trim();
    if (p === "") return true; // empty prefix = the suffix's root domain
    return p
      .split(".")
      .every((l) => l.length >= 1 && l.length <= 63 && LABEL_RE.test(l));
  }

  // Split a stored hostname into { subPrefix, suffix }. Prefer the active system
  // suffix; otherwise fall back to "first label is the prefix, the rest is the
  // suffix" so a hostname on a different/legacy suffix still round-trips.
  function splitHostname(
    hostname: string,
    sysSuffix: string,
  ): { subPrefix: string; suffix: string } {
    const host = (hostname ?? "").trim().toLowerCase();
    if (host === sysSuffix) return { subPrefix: "", suffix: sysSuffix };
    if (host.endsWith("." + sysSuffix)) {
      return {
        subPrefix: host.slice(0, host.length - sysSuffix.length - 1),
        suffix: sysSuffix,
      };
    }
    const dot = host.indexOf(".");
    if (dot > 0) {
      return { subPrefix: host.slice(0, dot), suffix: host.slice(dot + 1) };
    }
    return { subPrefix: host, suffix: sysSuffix };
  }

  // Capture the initial split without binding it reactively to the props; the
  // $effect below re-syncs whenever `value` changes from outside.
  const initial = untrack(() => splitHostname(value, systemSuffix));
  let subPrefix = $state(initial.subPrefix);
  let suffix = $state(initial.suffix);
  // Guard so our own compose → `value` write doesn't re-trigger the re-split.
  let lastComposed = untrack(() => value);

  // Re-split whenever `value` changes from the outside (project loads, reset,
  // raw-config edit). Skipped for our own writes via the lastComposed guard.
  $effect(() => {
    if (value !== lastComposed) {
      const s = splitHostname(value, systemSuffix);
      subPrefix = s.subPrefix;
      suffix = s.suffix;
      lastComposed = value;
    }
  });

  const valid = $derived(isValidSubPrefix(subPrefix));
  $effect(() => onValidChange?.(valid));

  const suffixOptions = $derived.by<string[]>(() => {
    const opts = [systemSuffix];
    if (suffix && !opts.includes(suffix)) opts.push(suffix);
    return opts;
  });

  const composed = $derived(
    subPrefix.trim()
      ? `${subPrefix.trim().toLowerCase()}.${suffix}`
      : suffix,
  );

  // Push the composed hostname back to the bound value.
  function commit() {
    lastComposed = composed;
    value = composed;
    onInput?.();
  }
</script>

<div
  class="flex items-stretch rounded-lg bg-bg border transition-shadow
         focus-within:ring-2 focus-within:ring-accent/40
         {valid ? 'border-border' : 'border-status-crashed/70'}
         {disabled ? 'opacity-60' : ''}"
>
  <input
    {id}
    {disabled}
    value={subPrefix}
    oninput={(e) => {
      subPrefix = e.currentTarget.value.toLowerCase().replace(/\s+/g, "");
      commit();
    }}
    placeholder="subdomain"
    spellcheck="false"
    autocapitalize="off"
    autocomplete="off"
    class="min-w-0 flex-1 h-9 px-3 rounded-l-lg bg-transparent font-mono
           text-[13px] text-fg placeholder:text-fg-subtle focus:outline-none"
  />
  {#if suffixOptions.length > 1}
    <select
      bind:value={suffix}
      onchange={commit}
      {disabled}
      aria-label="Domain suffix"
      class="h-9 shrink-0 pl-2 pr-7 rounded-r-lg border-l border-border
             bg-surface-2/60 font-mono text-[13px] text-fg-muted focus:outline-none"
    >
      {#each suffixOptions as s (s)}
        <option value={s}>.{s}</option>
      {/each}
    </select>
  {:else}
    <span
      class="inline-flex items-center h-9 shrink-0 px-3 rounded-r-lg border-l
             border-border bg-surface-2/50 font-mono text-[13px] text-fg-muted
             select-none"
    >
      .{suffix}
    </span>
  {/if}
</div>
{#if !valid}
  <p class="mt-1 text-[11px] text-status-crashed leading-relaxed">
    Use lowercase letters, digits, and hyphens (e.g.
    <code class="font-mono">cloud</code> or
    <code class="font-mono">api.staging</code>). Leave empty for the root domain.
  </p>
{:else}
  <p class="mt-1 text-[11px] text-fg-subtle leading-relaxed">
    Resolves at <code class="font-mono">{composed}</code>.
  </p>
{/if}
