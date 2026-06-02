/**
 * deploySnippets — saved deploy command sequences ("macros"), per SSH
 * connection, so a Phase-3 deploy you ran once can be recalled and re-run
 * instead of retyped.
 *
 * Local convenience state (like terminal prefs / IDE layout), so it persists to
 * localStorage rather than the registry — it carries no secrets, just command
 * text the user already sees and explicitly runs.
 */
import { browser } from "$app/environment";

export interface DeploySnippet {
  id: string;
  name: string;
  /** Working directory for the run (may be empty). */
  cwd: string;
  steps: string[];
}

const STORAGE_KEY = "portbay.ssh.deploySnippets";
/** Cap per connection so the list stays a quick-pick, not a dumping ground. */
const MAX_PER_CONNECTION = 30;

type SnippetMap = Record<string, DeploySnippet[]>;

function freshId(): string {
  if (browser && typeof crypto !== "undefined" && crypto.randomUUID) return crypto.randomUUID();
  return `snip-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
}

function load(): SnippetMap {
  if (!browser) return {};
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as SnippetMap;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function createDeploySnippetsStore() {
  let map = $state<SnippetMap>(load());

  function persist() {
    if (!browser) return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
    } catch {
      /* storage unavailable; keep in-memory */
    }
  }

  return {
    /** Saved snippets for a connection (empty array when none). */
    list(connectionId: string): DeploySnippet[] {
      return map[connectionId] ?? [];
    },
    /** Save a new snippet (capped). Returns its id, or null if at the cap. */
    add(connectionId: string, name: string, cwd: string, steps: string[]): string | null {
      const current = map[connectionId] ?? [];
      if (current.length >= MAX_PER_CONNECTION) return null;
      const snippet: DeploySnippet = { id: freshId(), name, cwd, steps };
      map = { ...map, [connectionId]: [...current, snippet] };
      persist();
      return snippet.id;
    },
    remove(connectionId: string, id: string) {
      const current = map[connectionId] ?? [];
      map = { ...map, [connectionId]: current.filter((s) => s.id !== id) };
      persist();
    },
  };
}

export const deploySnippets = createDeploySnippetsStore();
