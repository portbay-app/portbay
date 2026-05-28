/**
 * Memoized loader for a project's detected avatar icon.
 *
 * Asks the Rust `project_icon` command for a project's real favicon / web-clip
 * / app icon (returned as a `data:` URL) and caches the in-flight promise per
 * project id, so the same icon is fetched at most once regardless of how many
 * avatars render it. A failed lookup resolves to `null` — callers then fall
 * back to the project's stack glyph rather than surfacing an error.
 */
import { invokeQuiet } from "$lib/ipc";

const cache = new Map<string, Promise<string | null>>();

export function loadProjectIcon(id: string): Promise<string | null> {
  const hit = cache.get(id);
  if (hit) return hit;

  const promise = invokeQuiet<string | null>("project_icon", { id }).catch(
    () => null,
  );
  cache.set(id, promise);
  return promise;
}

/** Drop a cached icon promise so the next `loadProjectIcon` re-fetches. */
export function invalidateProjectIcon(id: string): void {
  cache.delete(id);
}
