/**
 * Memoized loader for the signed-in account's avatar.
 *
 * Asks the Rust `get_account_avatar` command for the user's GitHub avatar
 * (fetched + cached backend-side, returned as a `data:` URL). The in-flight
 * promise is cached per account key so the same image is resolved at most once
 * regardless of how many avatars render it. A miss — signed out, an email-auth
 * account, or a failed fetch with no cache — resolves to `null`, and callers
 * then fall back to the user's initials rather than surfacing an error.
 */
import { invokeQuiet } from "$lib/ipc";

const cache = new Map<string, Promise<string | null>>();

/**
 * Resolve the avatar for the given account key (the `github_id`, stringified).
 * Keyed so switching accounts re-fetches rather than reusing the prior face.
 */
export function loadUserAvatar(key: string): Promise<string | null> {
  const hit = cache.get(key);
  if (hit) return hit;

  const promise = invokeQuiet<string | null>("get_account_avatar").catch(
    () => null,
  );
  cache.set(key, promise);
  return promise;
}

/** Drop all cached avatar promises (e.g. on sign-out) so the next load re-fetches. */
export function invalidateUserAvatar(): void {
  cache.clear();
}
