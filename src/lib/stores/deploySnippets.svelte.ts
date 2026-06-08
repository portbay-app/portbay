/**
 * deploySnippets — saved deploy command sequences ("macros"), per SSH
 * connection, so a Phase-3 deploy you ran once can be recalled and re-run
 * instead of retyped.
 *
 * Persistence: the Rust backend file `<data_dir>/PortBay/ssh-deploy-snippets.json`
 * (via `ssh_deploy_snippets_get/set`). Snippets used to live only in
 * localStorage, but WKWebView keys its storage by bundle identity — running
 * the dev binary unbundled vs. wrapped in a signed .app lands in different
 * containers and saved snippets silently "vanished" across relaunches. The
 * data-dir file survives all of that; localStorage stays as a mirror so the
 * hosted simulator (no Tauri backend) keeps working, and any pre-existing
 * localStorage snippets are merged into the backend file on first load.
 */
import { browser } from "$app/environment";

import { invokeQuiet } from "$lib/ipc";

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

function loadLocal(): SnippetMap {
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

/** Union of two maps, per connection, deduped by snippet id (base wins). */
function merge(base: SnippetMap, extra: SnippetMap): SnippetMap {
  const out: SnippetMap = { ...base };
  for (const [conn, snippets] of Object.entries(extra)) {
    const have = new Set((out[conn] ?? []).map((s) => s.id));
    const fresh = snippets.filter((s) => !have.has(s.id));
    if (fresh.length) out[conn] = [...(out[conn] ?? []), ...fresh];
  }
  return out;
}

function createDeploySnippetsStore() {
  // Seed synchronously from localStorage so the pane isn't empty for the
  // first frame; the backend file is authoritative and hydrates over it.
  let map = $state<SnippetMap>(loadLocal());
  // Until backend hydration settles, writes go to localStorage only — a
  // premature `set` could clobber the file with the (possibly empty) seed.
  let backendReady = false;

  if (browser) void hydrate();

  async function hydrate() {
    let fromBackend: SnippetMap;
    try {
      const raw = await invokeQuiet<SnippetMap | null>("ssh_deploy_snippets_get");
      if (!raw || typeof raw !== "object") return; // no real backend — stay on localStorage
      fromBackend = raw;
    } catch {
      // No backend (hosted simulator) — stay on the localStorage mirror.
      return;
    }
    const local = map;
    map = merge(fromBackend, local);
    backendReady = true;
    // One-time migration: anything that existed only in localStorage is now
    // part of the merged map — push it into the backend file.
    const localOnly = Object.entries(local).some(([conn, snippets]) => {
      const have = new Set((fromBackend[conn] ?? []).map((s) => s.id));
      return snippets.some((s) => !have.has(s.id));
    });
    if (localOnly) persist();
  }

  function persist() {
    if (!browser) return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
    } catch {
      /* storage unavailable; the backend file still has it */
    }
    if (backendReady) {
      invokeQuiet("ssh_deploy_snippets_set", { snippets: map }).catch(() => {
        /* write failed; localStorage mirror still has it for this identity */
      });
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
