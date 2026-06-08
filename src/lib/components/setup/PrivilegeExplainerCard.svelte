<!--
  PrivilegeExplainerCard — surfaces the two macOS privilege prompts that
  PortBay will eventually trigger *before* they appear, so users are never
  surprised by an unexpected admin-password dialog.

  Why here (onboarding) rather than at the moment of the prompt?
  ─────────────────────────────────────────────────────────────────────────
  When an operating-system dialog appears without context, users often
  dismiss it reflexively — denying the access and then wondering why the
  feature doesn't work. Showing this card at first launch, while the user
  is already in a "setting up" mindset, converts a potential shock into an
  expected step.

  Two prompts are explained:

  1. **Privileged helper (admin password)** — PortBay installs a tiny macOS
     LaunchDaemon that writes /etc/hosts entries without repeating the prompt
     on every project change. macOS asks once for your admin password. The
     daemon then appears under System Settings › General › Login Items ›
     "Allow in the Background". This is the `install_privileged_helper` IPC
     flow, gated by `MacPermissionDialog` before it fires.

  2. **mkcert CA trust (Keychain prompt)** — For local HTTPS, PortBay runs
     `mkcert -install` the first time it issues a certificate. macOS asks
     once to add PortBay's local CA to the system Keychain so browsers trust
     the certs silently. No admin password needed — standard Keychain auth.

  The card is informational only. No action buttons. It disappears once both
  operations are already complete (helper installed + CA trusted), so
  returning users who re-trigger onboarding don't see stale advice.

  Props
  ─────
  helperInstalled  — from a `dns_preflight` call on the parent; undefined
                     while loading. Hides the DNS row once the helper is up.
  caInstalled      — from `sidecar_status`; true once mkcertCa.status === "running".
                     Hides the CA row once the CA is trusted.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";

  interface Props {
    /** Whether PortBay's privileged hosts/DNS helper is already installed.
        Pass `undefined` while the preflight result is loading — shows a
        subtle skeleton so the card doesn't flash in and out. */
    helperInstalled?: boolean;
    /** Whether the mkcert local CA is already trusted in the system Keychain.
        Pass `undefined` while loading. */
    caInstalled?: boolean;
  }

  let { helperInstalled = false, caInstalled = false }: Props = $props();

  // The card is only useful if at least one prompt hasn't been answered yet.
  // When both are done, suppress the card entirely rather than showing an
  // empty "nothing to do" state — the user already went through setup.
  const shouldShow = $derived(
    helperInstalled === undefined ||
      caInstalled === undefined ||
      !helperInstalled ||
      !caInstalled,
  );
</script>

{#if shouldShow}
  <section
    class="mt-4 p-5 rounded-xl border border-border bg-surface"
    aria-label="Upcoming permission prompts"
  >
    <div class="flex items-center gap-2.5 mb-4">
      <div
        class="w-7 h-7 shrink-0 rounded-md bg-accent/10 text-accent
               flex items-center justify-center"
      >
        <Icon name="shield" size={13} />
      </div>
      <div>
        <div class="text-sm font-medium">
          What PortBay will ask for
        </div>
        <p class="text-[11px] text-fg-subtle leading-snug">
          Two one-time macOS prompts — here's why each one exists.
        </p>
      </div>
    </div>

    <div class="space-y-3">
      {#if helperInstalled === undefined || !helperInstalled}
        <!-- Row 1: DNS helper / admin password -->
        <div
          class="flex items-start gap-3 p-3 rounded-lg bg-surface-2/50
                 border border-border/60"
        >
          <div
            class="shrink-0 w-7 h-7 rounded-md bg-surface border border-border
                   flex items-center justify-center mt-0.5"
          >
            <!-- Lock icon signals a privileged/OS-level operation. -->
            <Icon name="lock" size={12} class="text-fg-muted" />
          </div>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2 mb-0.5">
              <span class="text-[12.5px] font-medium text-fg">
                Admin password — one time
              </span>
              {#if helperInstalled === undefined}
                <!-- Still loading the preflight; show a subtle placeholder. -->
                <span
                  class="h-2 w-14 rounded bg-border/60 animate-pulse"
                  aria-hidden="true"
                ></span>
              {:else}
                <!-- Not yet installed. -->
                <span
                  class="px-1.5 py-px rounded text-[10px] font-medium
                         bg-fg-subtle/10 text-fg-subtle"
                >
                  Pending
                </span>
              {/if}
            </div>
            <p class="text-[11.5px] text-fg-muted leading-relaxed">
              PortBay installs a small privileged helper so your project
              hostnames (like <code class="font-mono">myapp.test</code>)
              resolve without repeating the prompt on every change. macOS
              shows the prompt once; afterwards the helper appears in
              <span class="font-medium text-fg">System Settings › Login Items</span>.
            </p>
          </div>
        </div>
      {/if}

      {#if caInstalled === undefined || !caInstalled}
        <!-- Row 2: mkcert CA / Keychain prompt -->
        <div
          class="flex items-start gap-3 p-3 rounded-lg bg-surface-2/50
                 border border-border/60"
        >
          <div
            class="shrink-0 w-7 h-7 rounded-md bg-surface border border-border
                   flex items-center justify-center mt-0.5"
          >
            <!-- Shield-check pairs with "certificate trust" semantics. -->
            <Icon name="shield-check" size={12} class="text-fg-muted" />
          </div>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2 mb-0.5">
              <span class="text-[12.5px] font-medium text-fg">
                Keychain access — first HTTPS project
              </span>
              {#if caInstalled === undefined}
                <span
                  class="h-2 w-14 rounded bg-border/60 animate-pulse"
                  aria-hidden="true"
                ></span>
              {:else}
                <span
                  class="px-1.5 py-px rounded text-[10px] font-medium
                         bg-fg-subtle/10 text-fg-subtle"
                >
                  Pending
                </span>
              {/if}
            </div>
            <p class="text-[11.5px] text-fg-muted leading-relaxed">
              When you enable HTTPS on your first project, PortBay asks macOS
              to trust its local certificate authority. Browsers then show
              the padlock silently — no more security warnings on
              <code class="font-mono">.test</code> sites. The prompt appears
              only once for the lifetime of this machine.
            </p>
          </div>
        </div>
      {/if}
    </div>

    <p class="mt-3 text-[10.5px] text-fg-subtle leading-relaxed">
      Neither prompt shares any data outside your Mac. You can review
      PortBay's entries in <span class="font-medium text-fg">Keychain Access</span>
      and under <span class="font-medium text-fg">System Settings › Login Items</span>
      at any time.
    </p>
  </section>
{/if}
