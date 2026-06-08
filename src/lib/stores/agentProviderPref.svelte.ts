/**
 * Remembers which Agent provider (ollama / Claude Code / Codex) you last used
 * per SSH host, so reopening the Agent tab lands on the same one.
 *
 * Frontend-only, persisted to localStorage as a `{ connectionId: provider }`
 * map — honest about being a UI preference, not server state. The component
 * reads the saved provider after detection and only honours it if that provider
 * is still available on the host (otherwise it falls back to the first one).
 */
import { browser } from "$app/environment";

export type AgentProvider = "ollama" | "claude" | "codex";

const PROVIDERS: AgentProvider[] = ["ollama", "claude", "codex"];
const STORAGE_KEY = "portbay.ssh.agentProvider";

function isProvider(v: unknown): v is AgentProvider {
  return typeof v === "string" && PROVIDERS.includes(v as AgentProvider);
}

function loadMap(): Record<string, AgentProvider> {
  if (!browser) return {};
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const out: Record<string, AgentProvider> = {};
    for (const [id, v] of Object.entries(parsed)) {
      if (isProvider(v)) out[id] = v;
    }
    return out;
  } catch {
    return {};
  }
}

function createAgentProviderPref() {
  let providers = $state<Record<string, AgentProvider>>(loadMap());

  function persist() {
    if (!browser) return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(providers));
    } catch {
      /* storage unavailable (private mode); keep the in-memory value */
    }
  }

  return {
    /** The remembered provider for a host, or null if none saved. */
    get(connectionId: string): AgentProvider | null {
      return providers[connectionId] ?? null;
    },
    /** Record the provider the user just switched to for this host. */
    set(connectionId: string, provider: AgentProvider) {
      if (providers[connectionId] === provider) return;
      providers = { ...providers, [connectionId]: provider };
      persist();
    },
  };
}

export const agentProviderPref = createAgentProviderPref();
