<!--
  SshKbiPrompt — the single VS Code-style keyboard-interactive / 2FA dialog,
  driven by the `sshKbiPrompt` store and mounted once at the layout root.

  It descends from the top of the window (like VS Code's Quick Input) and
  presents one or more challenge fields issued by the server (password OTP,
  token, etc.). The user fills every field then clicks Continue (or presses
  Enter). Esc / backdrop / Cancel → dismiss().

  A muted countdown hints at the backend's 120s timeout; it turns red at ≤15s
  and disables Continue at 0s (purely advisory — the backend enforces the real
  timeout). The timer resets each time a new challenge opens.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import QuickSheet from "$lib/components/connections/QuickSheet.svelte";
  import { sshKbiPrompt } from "$lib/stores/sshKbiPrompt.svelte";

  // Per-field response values, one entry per prompt.
  let responses = $state<string[]>([]);

  // Element refs for focus management.
  let firstInputEl = $state<HTMLInputElement | null>(null);
  let lastFocused: HTMLElement | null = null;

  // Countdown state.
  let secondsLeft = $state(90);
  let countdownInterval: ReturnType<typeof setInterval> | null = null;

  // Reset responses, focus the first field, and start countdown each time the
  // prompt opens. Restore focus to the previous element when it closes.
  $effect(() => {
    if (sshKbiPrompt.isOpen) {
      responses = sshKbiPrompt.prompts.map(() => "");
      secondsLeft = 90;
      lastFocused = document.activeElement as HTMLElement | null;
      queueMicrotask(() => firstInputEl?.focus());

      countdownInterval = setInterval(() => {
        secondsLeft = Math.max(0, secondsLeft - 1);
        if (secondsLeft === 0 && countdownInterval !== null) {
          clearInterval(countdownInterval);
          countdownInterval = null;
        }
      }, 1000);
    } else {
      if (countdownInterval !== null) {
        clearInterval(countdownInterval);
        countdownInterval = null;
      }
      if (lastFocused) {
        lastFocused.focus();
        lastFocused = null;
      }
    }
  });

  // Continue is enabled only when every field has a value and time hasn't expired.
  const allFilled = $derived(
    responses.length > 0 && responses.every((r) => r.length > 0),
  );
  const canSubmit = $derived(allFilled && secondsLeft > 0);

  function submitKbi() {
    if (!canSubmit) return;
    void sshKbiPrompt.submit(responses);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      submitKbi();
    }
    // Esc is handled by QuickSheet → ondismiss → sshKbiPrompt.dismiss()
  }

  onMount(() => {
    void sshKbiPrompt.start();
  });

  onDestroy(() => {
    sshKbiPrompt.stop();
    if (countdownInterval !== null) {
      clearInterval(countdownInterval);
      countdownInterval = null;
    }
  });
</script>

<QuickSheet
  open={sshKbiPrompt.isOpen}
  heading="Two-step verification"
  hostLabel={sshKbiPrompt.host}
  icon="shield"
  iconClass="text-accent"
  ondismiss={() => void sshKbiPrompt.dismiss()}
>
  {#snippet headerExtra()}
    <span
      class="text-[11px] tabular-nums
             {secondsLeft <= 15 ? 'text-status-crashed' : 'text-fg-muted'}"
    >
      {secondsLeft}s
    </span>
  {/snippet}

  {#snippet body()}
    <div onkeydown={onKeydown} role="none" class="space-y-3">
      {#if sshKbiPrompt.name}
        <p class="text-[12px] font-semibold text-fg">{sshKbiPrompt.name}</p>
      {/if}
      {#if sshKbiPrompt.instructions}
        <p class="text-[11.5px] text-fg-subtle leading-relaxed whitespace-pre-line">
          {sshKbiPrompt.instructions}
        </p>
      {/if}

      {#each sshKbiPrompt.prompts as field, i}
        <label class="block">
          <span class="text-[11.5px] text-fg-subtle">{field.prompt}</span>
          {#if i === 0}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              bind:this={firstInputEl}
              bind:value={responses[i]}
              type={field.echo ? "text" : "password"}
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              autofocus
              class="mt-1.5 w-full h-9 rounded-md border border-border bg-surface-2 px-2.5
                     text-[12.5px] text-fg focus:outline-none focus:ring-2 focus:ring-accent/50"
            />
          {:else}
            <input
              bind:value={responses[i]}
              type={field.echo ? "text" : "password"}
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              class="mt-1.5 w-full h-9 rounded-md border border-border bg-surface-2 px-2.5
                     text-[12.5px] text-fg focus:outline-none focus:ring-2 focus:ring-accent/50"
            />
          {/if}
        </label>
      {/each}

      <!-- Footer buttons -->
      <div class="flex items-center justify-end gap-2 pt-0.5">
        <button
          type="button"
          onclick={() => void sshKbiPrompt.dismiss()}
          class="inline-flex items-center justify-center h-8 px-3 rounded-md text-[12px]
                 font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
        >
          Cancel
        </button>
        <button
          type="button"
          onclick={submitKbi}
          disabled={!canSubmit}
          class="inline-flex items-center justify-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
                 font-medium bg-accent text-on-accent hover:brightness-110 disabled:opacity-50"
        >
          <Icon name="arrow-right" size={12} />
          Continue
        </button>
      </div>
    </div>
  {/snippet}
</QuickSheet>
