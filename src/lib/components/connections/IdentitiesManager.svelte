<!--
  IdentitiesManager — manage reusable SSH identities (shared user + key/agent/
  password method). A flat list with an inline add/edit form; delete is blocked
  while a host still borrows an identity. Reached from the hosts dashboard.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { sshIdentities } from "$lib/stores/sshIdentities.svelte";
  import type { SaveSshIdentityInput, SshIdentityView } from "$lib/types/sshIdentities";
  import type { SshAuthKind } from "$lib/types/sshTunnels";

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  type Draft = {
    id: string | null;
    name: string;
    sshUser: string;
    authKind: SshAuthKind;
    keyPath: string;
  };

  function blankDraft(): Draft {
    return { id: null, name: "", sshUser: "", authKind: "key", keyPath: "" };
  }

  let draft = $state<Draft | null>(null);

  onMount(() => {
    void sshIdentities.refresh();
  });

  function startCreate() {
    draft = blankDraft();
  }

  function startEdit(identity: SshIdentityView) {
    draft = {
      id: identity.id,
      name: identity.name,
      sshUser: identity.sshUser,
      authKind: identity.authKind,
      keyPath: identity.keyPath ?? "",
    };
  }

  async function submit() {
    if (!draft) return;
    const input: SaveSshIdentityInput = {
      id: draft.id,
      name: draft.name.trim(),
      sshUser: draft.sshUser.trim(),
      authKind: draft.authKind,
      keyPath: draft.keyPath.trim() || null,
    };
    const saved = await sshIdentities.save(input);
    if (saved) draft = null;
  }
</script>

<section class="h-full min-w-0 overflow-y-auto">
  <header class="px-8 pt-6 pb-4 border-b border-border/60">
    <button
      type="button"
      onclick={onclose}
      class="inline-flex items-center gap-1.5 text-[12px] text-fg-muted hover:text-fg transition-colors"
    >
      <Icon name="chevron-left" size={14} />
      Back to hosts
    </button>
    <div class="mt-3 flex items-center gap-2.5">
      <Icon name="key" size={18} class="text-accent" />
      <h1 class="text-[17px] font-semibold tracking-tight text-fg">SSH Identities</h1>
      {#if !draft}
        <button
          type="button"
          onclick={startCreate}
          class="ml-auto inline-flex items-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
                 font-medium text-on-accent bg-accent shadow-sm hover:brightness-110 transition"
        >
          <Icon name="plus" size={13} />
          New identity
        </button>
      {/if}
    </div>
    <p class="mt-1.5 text-[12.5px] text-fg-muted leading-relaxed max-w-2xl">
      Reuse one login (user + key/agent/password) across many hosts. Passwords stay in your OS keychain.
    </p>
  </header>

  <div class="px-8 py-6 space-y-4">
    {#if draft}
      <form
        class="rounded-lg border border-border/70 bg-surface px-5 py-5 space-y-3"
        onsubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <h2 class="text-[13px] font-semibold text-fg">{draft.id ? "Edit identity" : "New identity"}</h2>
        <label class="block text-[11px] font-medium text-fg-subtle">
          Name
          <input bind:value={draft.name} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Deploy user" />
        </label>
        <label class="block text-[11px] font-medium text-fg-subtle">
          User
          <input bind:value={draft.sshUser} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="deploy, ubuntu, ec2-user" />
        </label>
        <label class="block text-[11px] font-medium text-fg-subtle">
          Auth
          <select bind:value={draft.authKind} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg">
            <option value="key">Key</option>
            <option value="agent">SSH agent</option>
            <option value="password">Password (per host)</option>
          </select>
        </label>
        {#if draft.authKind === "key"}
          <label class="block text-[11px] font-medium text-fg-subtle">
            Key path
            <input bind:value={draft.keyPath} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="~/.ssh/id_ed25519 (optional; agent fallback)" />
          </label>
        {/if}
        <div class="flex items-center gap-2 pt-1">
          <button
            type="submit"
            disabled={sshIdentities.isBusy(draft.id ?? "__new")}
            class="flex-1 inline-flex items-center justify-center gap-1.5 h-9 rounded-md text-[12px]
                   font-medium bg-accent text-on-accent hover:brightness-110 disabled:opacity-50"
          >
            <Icon name={draft.id ? "check" : "plus"} size={12} />
            {draft.id ? "Save changes" : "Add identity"}
          </button>
          <button
            type="button"
            onclick={() => (draft = null)}
            class="inline-flex items-center justify-center gap-1.5 h-9 px-4 rounded-md text-[12px]
                   font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
          >
            Cancel
          </button>
        </div>
      </form>
    {/if}

    {#if sshIdentities.count === 0 && !draft}
      <div class="rounded-xl border border-dashed border-border px-6 py-12 text-center">
        <span class="inline-grid place-items-center w-12 h-12 rounded-xl bg-surface-2 text-fg-subtle mx-auto">
          <Icon name="key" size={24} />
        </span>
        <p class="mt-3 text-[13px] font-medium text-fg">No identities yet</p>
        <p class="mt-1.5 text-[12px] text-fg-subtle leading-relaxed max-w-md mx-auto">
          Create one to share a login across hosts instead of re-entering the user and key each time.
        </p>
      </div>
    {:else}
      <div class="space-y-2">
        {#each sshIdentities.value as identity (identity.id)}
          <article class="rounded-2xl border border-border/70 bg-surface px-5 py-3.5">
            <div class="flex items-center gap-3">
              <div class="min-w-0 flex-1">
                <p class="truncate text-[13px] font-semibold text-fg">{identity.name}</p>
                <p class="mt-0.5 truncate font-mono text-[11px] text-fg-subtle">
                  {identity.sshUser || "—"} · {identity.authKind}{identity.keyPath ? ` · ${identity.keyPath}` : ""}
                </p>
              </div>
              {#if identity.connectionCount > 0}
                <span class="shrink-0 rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-muted">
                  {identity.connectionCount} host{identity.connectionCount === 1 ? "" : "s"}
                </span>
              {/if}
              <button
                type="button"
                onclick={() => startEdit(identity)}
                class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                       font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
              >
                <Icon name="pencil" size={12} />
                Edit
              </button>
              <button
                type="button"
                onclick={() => sshIdentities.remove(identity.id)}
                disabled={identity.inUse || sshIdentities.isBusy(identity.id)}
                title={identity.inUse ? "Reassign the hosts using this identity first" : "Delete identity"}
                class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                       font-medium text-status-crashed hover:bg-status-crashed/10 disabled:opacity-50"
              >
                <Icon name="trash-2" size={12} />
                Delete
              </button>
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </div>
</section>
