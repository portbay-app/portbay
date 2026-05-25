<!--
  ToastHost — mounted once at the root layout. Renders every active toast
  from `errorBus` as a fixed-position stack in the bottom-right.

  Action button handling:
    - `url` set → opens via `@tauri-apps/plugin-opener` (matches the Rust
      side's `open_project` capability). External-only — we do not trust
      arbitrary user URLs here, but errorBus entries originate from our
      own Rust code, so this is safe.
    - `command` set → calls `safeInvoke(command, args)`. Errors from the
      follow-up call land back in errorBus through normal safeInvoke flow.
    - Neither set → no-op; the button just dismisses.
-->
<script lang="ts">
  import { openUrl } from "$lib/security/openUrl";

  import { errorBus } from "$lib/stores/errors.svelte";
  import { safeInvoke } from "$lib/ipc";
  import type { ErrorAction } from "$lib/types/error";
  import ErrorEnvelope from "./ErrorEnvelope.svelte";

  async function onAction(action: ErrorAction): Promise<boolean> {
    try {
      if (action.url) {
        await openUrl(action.url);
        return true; // dismiss after open
      }
      if (action.command) {
        await safeInvoke(action.command, action.args);
        return true; // dismiss after success
      }
    } catch {
      // safeInvoke already pushed the follow-up error toast.
      // Keep the original toast so the user can retry or read context.
      return false;
    }
    return true;
  }
</script>

<div
  class="pointer-events-none fixed bottom-4 right-4 z-50 flex flex-col gap-2"
  aria-live="polite"
>
  {#each errorBus.value as toast (toast.id)}
    <div class="pointer-events-auto">
      <ErrorEnvelope
        envelope={toast.envelope}
        tone="toast"
        onDismiss={() => errorBus.dismiss(toast.id)}
        {onAction}
      />
    </div>
  {/each}
</div>
