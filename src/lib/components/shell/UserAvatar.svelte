<!--
  UserAvatar — the signed-in account's visual identity in a circle.

  Resolution order, best to worst:
    1. A custom photo the user uploaded in their profile, when set (overrides
       everything — a deliberate choice beats an inherited one). [pending the
       profile-upload feature; the loader already returns whatever the backend
       resolves, so this slots in without a UI change here.]
    2. The user's real GitHub avatar (when the account carries a `github_id`),
       fetched + cached by the Rust `get_account_avatar` command and handed
       back as a `data:` URL — see `crate::avatar`.
    3. Initials (first + last initial) on a per-user deterministic gradient —
       the fallback whenever there is no photo at all.

  When nobody is signed in there is no identity to show, so the chip renders a
  neutral person glyph: it reads as "account / sign in" rather than a stray
  letter. The photo is fetched once per account via the memoized
  `loadUserAvatar` loader; until (and unless) it arrives, initials show, so
  there's no blank flash and offline accounts still get a stable identity.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { loadUserAvatar } from "$lib/userAvatar";

  interface Props {
    size?: number;
    class?: string;
  }
  let { size = 28, class: cls = "" }: Props = $props();

  const account = $derived(entitlements.account);

  /**
   * Up-to-two initials for the fallback. Prefers a real "First Last" name when
   * one is available (→ "FL"); otherwise derives from the login. For an email
   * login we take the local part before splitting, so `jane.doe@x.com` reads
   * "JD" rather than mixing in the domain. A single-token handle (`octocat`)
   * yields one letter.
   */
  function initialsFor(login: string): string {
    const base = login.includes("@") ? login.slice(0, login.indexOf("@")) : login;
    const segs = base.split(/[-_.\s]+/).filter(Boolean);
    if (segs.length >= 2) return (segs[0][0] + segs[1][0]).toUpperCase();
    const first = segs[0] ?? base;
    return first.slice(0, 2).toUpperCase() || "?";
  }

  /** A stable per-user gradient hashed from the login, so accounts read apart. */
  function gradientFor(login: string): string {
    let h = 0;
    for (let i = 0; i < login.length; i++) h = (h * 31 + login.charCodeAt(i)) >>> 0;
    const hue = h % 360;
    const hue2 = (hue + 40) % 360;
    return `linear-gradient(135deg, hsl(${hue} 68% 55%), hsl(${hue2} 60% 45%))`;
  }

  // Initials prefer the display name (→ proper first+last); gradient stays keyed
  // on the stable login so a user's colour doesn't shift when they rename.
  const initials = $derived(account ? initialsFor(account.display_name || account.login) : "");
  const gradient = $derived(account ? gradientFor(account.login) : "");

  // Resolved photo data URL (custom upload or GitHub), or null while loading /
  // when there's none — in which case initials show.
  let photo = $state<string | null>(null);

  $effect(() => {
    // Fetch when there's an avatar to show: a server-resolved `avatar_url`
    // (custom upload or GitHub) keys the cache so a changed `?v=` re-fetches;
    // otherwise the github_id guess. An email account with neither → initials.
    const key =
      account?.avatar_url ??
      (account?.github_id != null ? `gh:${account.github_id}` : null);
    photo = null;
    if (!key) return;
    let cancelled = false;
    loadUserAvatar(key).then((url) => {
      if (!cancelled) photo = url;
    });
    return () => {
      cancelled = true;
    };
  });
</script>

<span
  class="inline-flex items-center justify-center rounded-full shrink-0
         overflow-hidden font-semibold tracking-tight {cls}
         {account ? 'text-on-accent shadow-inner' : 'text-fg-subtle bg-surface-2'}"
  style:width="{size}px"
  style:height="{size}px"
  style:background={photo ? "transparent" : gradient || undefined}
  style:font-size="{Math.round(size * 0.4)}px"
>
  {#if photo}
    <img
      src={photo}
      alt={account?.login ?? "Account"}
      class="h-full w-full object-cover"
      draggable="false"
    />
  {:else if account}
    {initials}
  {:else}
    <Icon name="user" size={Math.round(size * 0.52)} />
  {/if}
</span>
