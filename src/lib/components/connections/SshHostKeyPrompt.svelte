<!--
  SshHostKeyPrompt — the single VS Code-style host-key dialog, driven by the
  `sshHostKeyPrompt` store and mounted once at the layout root.

  It descends from the top of the window (like VS Code's Quick Input) and
  presents the host key fingerprint for user verification. Two interaction
  modes:

  "new"     — first-contact host; user can Trust Once, Trust & Save, or Cancel.
  "changed" — key mismatch; shown as destructive. User must type the hostname
              to confirm before "Replace & Connect" is enabled.

  Esc / backdrop / Cancel → dismiss(). Enter on "new" → trustSave(). Enter on
  "changed" → trustSave() only when the confirmation field matches the host.
  Focus opens on the confirm field (changed state) or the Trust & Save button
  (new state) and is restored on close.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import QuickSheet from "$lib/components/connections/QuickSheet.svelte";
  import { sshHostKeyPrompt } from "$lib/stores/sshHostKeyPrompt.svelte";

  let confirmInput = $state("");
  let confirmInputEl = $state<HTMLInputElement | null>(null);
  let trustSaveBtnEl = $state<HTMLButtonElement | null>(null);
  let lastFocused: HTMLElement | null = null;

  // Reset confirm field and move focus each time the prompt opens or closes.
  $effect(() => {
    if (sshHostKeyPrompt.isOpen) {
      confirmInput = "";
      lastFocused = document.activeElement as HTMLElement | null;
      queueMicrotask(() => {
        if (sshHostKeyPrompt.state === "changed") {
          confirmInputEl?.focus();
        } else {
          trustSaveBtnEl?.focus();
        }
      });
    } else if (lastFocused) {
      lastFocused.focus();
      lastFocused = null;
    }
  });

  const confirmMatches = $derived(
    confirmInput.trim().toLowerCase() === sshHostKeyPrompt.host.toLowerCase(),
  );

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      if (sshHostKeyPrompt.state === "new") {
        sshHostKeyPrompt.trustSave();
      } else if (sshHostKeyPrompt.state === "changed" && confirmMatches) {
        sshHostKeyPrompt.trustSave();
      }
    }
    // Esc is handled by QuickSheet → ondismiss → sshHostKeyPrompt.dismiss()
  }

  onMount(() => {
    void sshHostKeyPrompt.start();
  });

  onDestroy(() => {
    sshHostKeyPrompt.stop();
  });

  const isNew = $derived(sshHostKeyPrompt.state === "new");
  const sheetHeading = $derived(isNew ? "Unknown host key" : "Host key changed");
  const sheetIcon = $derived(isNew ? "key" : "alert-triangle") as "key" | "alert-triangle";
  const sheetIconClass = $derived(isNew ? "text-accent" : "text-status-crashed");
  const sheetHostLabel = $derived(
    `${sshHostKeyPrompt.host}:${sshHostKeyPrompt.port}`,
  );
</script>

<QuickSheet
  open={sshHostKeyPrompt.isOpen}
  heading={sheetHeading}
  hostLabel={sheetHostLabel}
  icon={sheetIcon}
  iconClass={sheetIconClass}
  ondismiss={() => void sshHostKeyPrompt.dismiss()}
>
  {#snippet body()}
    <div onkeydown={onKeydown} role="none">
      {#if isNew}
        <!-- ── "new" state ─────────────────────────────────────────────── -->
        <p class="text-[11.5px] text-fg-subtle leading-relaxed">
          First time connecting to this host — verify the key fingerprint before trusting it.
        </p>

        <dl class="mt-3 space-y-1.5">
          <div class="flex gap-2 text-[11.5px]">
            <dt class="w-20 shrink-0 text-fg-subtle">Key type</dt>
            <dd class="font-mono text-fg">{sshHostKeyPrompt.keyType}</dd>
          </div>
          <div class="flex gap-2 text-[11.5px]">
            <dt class="w-20 shrink-0 text-fg-subtle">Fingerprint</dt>
            <dd class="min-w-0 font-mono text-fg select-all break-all">
              {sshHostKeyPrompt.fingerprint}
            </dd>
          </div>
        </dl>

        <div class="mt-3 flex items-center justify-end gap-2">
          <button
            type="button"
            onclick={() => void sshHostKeyPrompt.dismiss()}
            class="inline-flex items-center justify-center h-8 px-3 rounded-md text-[12px]
                   font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
          >
            Cancel
          </button>
          <button
            type="button"
            onclick={() => sshHostKeyPrompt.trustOnce()}
            class="inline-flex items-center justify-center h-8 px-3 rounded-md text-[12px]
                   font-medium border border-border text-fg hover:bg-surface-2"
          >
            Trust Once
          </button>
          <button
            bind:this={trustSaveBtnEl}
            type="button"
            onclick={() => sshHostKeyPrompt.trustSave()}
            class="inline-flex items-center justify-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
                   font-medium bg-accent text-on-accent hover:brightness-110"
          >
            <Icon name="shield-check" size={12} />
            Trust &amp; Save
          </button>
        </div>
      {:else}
        <!-- ── "changed" state ────────────────────────────────────────── -->
        <div class="rounded-md border border-status-crashed/40 bg-status-crashed/10 px-3 py-2.5 text-[11.5px] text-status-crashed leading-relaxed">
          The key for this host no longer matches the one you previously trusted. This can mean
          the server was rebuilt — or that the connection is being intercepted.
        </div>

        <dl class="mt-3 space-y-1.5">
          <div class="flex gap-2 text-[11.5px]">
            <dt class="w-32 shrink-0 text-fg-subtle">New fingerprint</dt>
            <dd class="min-w-0 font-mono text-status-crashed select-all break-all">
              {sshHostKeyPrompt.fingerprint}
            </dd>
          </div>
          {#if sshHostKeyPrompt.expectedFingerprint}
            <div class="flex gap-2 text-[11.5px]">
              <dt class="w-32 shrink-0 text-fg-subtle">Previously trusted</dt>
              <dd class="min-w-0 font-mono text-fg-muted line-through break-all">
                {sshHostKeyPrompt.expectedFingerprint}
              </dd>
            </div>
          {/if}
        </dl>

        <label class="mt-3 block">
          <span class="text-[11.5px] text-fg-subtle">
            Type the hostname <span class="font-mono text-fg">{sshHostKeyPrompt.host}</span> to confirm
          </span>
          <input
            bind:this={confirmInputEl}
            bind:value={confirmInput}
            type="text"
            autocomplete="off"
            autocapitalize="off"
            spellcheck="false"
            placeholder={sshHostKeyPrompt.host}
            class="mt-1.5 w-full h-9 rounded-md border border-border bg-surface-2 px-2.5 text-[12.5px] text-fg
                   focus:outline-none focus:ring-2 focus:ring-status-crashed/50"
          />
        </label>

        <div class="mt-3 flex items-center justify-end gap-2">
          <button
            type="button"
            onclick={() => void sshHostKeyPrompt.dismiss()}
            class="inline-flex items-center justify-center h-8 px-3 rounded-md text-[12px]
                   font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
          >
            Cancel
          </button>
          <button
            type="button"
            onclick={() => sshHostKeyPrompt.trustSave()}
            disabled={!confirmMatches}
            class="inline-flex items-center justify-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
                   font-medium bg-status-crashed text-white hover:brightness-110 disabled:opacity-50"
          >
            <Icon name="shield-off" size={12} />
            Replace &amp; Connect
          </button>
        </div>
      {/if}
    </div>
  {/snippet}
</QuickSheet>
