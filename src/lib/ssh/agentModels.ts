/**
 * Agent-model metadata + catalog resolver for the SSH workspace agent chat.
 *
 * Static per-agent model lists (each `id` is the exact value the agent's CLI
 * accepts for its `--model`/`-m` flag) plus a resolver that prefers a LIVE host
 * catalog (via the `agent_model_catalog` command, where the CLI keeps a
 * machine-readable one) and falls back to the static table. The resolver never
 * rejects — any IPC failure falls back to the static list, so it is safe in the
 * community build where the live-catalog command may be absent.
 */
import { invokeQuiet } from "$lib/ipc";

export interface AgentModel {
  id: string;
  name: string;
  description?: string;
  /** Small pill next to the name (e.g. "Default"). */
  badge?: string;
  /** Ollama catalog only: whether the model can make native tool calls. */
  toolCapable?: boolean;
}

/**
 * STATIC FALLBACK per-agent model picker options. Each `id` is the EXACT value
 * the agent's CLI accepts for its `--model`/`-m` flag. Where a CLI offers a
 * version-agnostic alias (Claude's `sonnet`/`opus`/`haiku`, Aider's `sonnet`),
 * that alias is used so the pick always resolves to the latest model the tool
 * ships and never goes stale. Prefer `agentModelCatalog()` over reading this
 * directly: where a CLI keeps a machine-readable catalog on the host the
 * backend serves the LIVE list and this table is only the fallback.
 */
export const AGENT_MODELS: Record<string, AgentModel[]> = {
  // `claude --model` aliases — each resolves to the latest of that tier.
  claude: [
    { id: "sonnet", name: "Sonnet", description: "Balanced" },
    { id: "opus", name: "Opus", description: "Most capable" },
    { id: "haiku", name: "Haiku", description: "Fastest" },
  ],
  // `codex --model` slugs (fallback only — the live list comes from the host's
  // `models_cache.json` via `agent_model_catalog`).
  codex: [
    { id: "gpt-5.5", name: "GPT-5.5", description: "Flagship (default)" },
    { id: "gpt-5.4", name: "GPT-5.4", description: "Previous flagship" },
    { id: "gpt-5.4-mini", name: "GPT-5.4-Mini", description: "Faster" },
  ],
  // `gemini -m` model ids (geminicli.com/docs).
  gemini: [
    { id: "gemini-3-pro", name: "Gemini 3 Pro", description: "Most capable" },
    { id: "gemini-3.1-pro-preview", name: "Gemini 3.1 Pro", description: "Newest", badge: "Preview" },
    { id: "gemini-2.5-flash", name: "Gemini 2.5 Flash", description: "Fastest" },
  ],
  // `aider --model` — `sonnet`/`opus` are Aider's version-agnostic Anthropic
  // shortcuts; `gpt-5.5` forwards to OpenAI's current flagship via litellm.
  aider: [
    { id: "sonnet", name: "Sonnet", description: "Anthropic Sonnet" },
    { id: "opus", name: "Opus", description: "Anthropic Opus" },
    { id: "gpt-5.5", name: "GPT-5.5", description: "OpenAI GPT-5.5" },
  ],
  // `qwen -m` model ids.
  qwen: [
    { id: "qwen3-coder-plus", name: "Qwen3 Coder Plus", description: "Most capable" },
    { id: "qwen3-coder-flash", name: "Qwen3 Coder Flash", description: "Faster" },
  ],
};

/** One in-flight/settled fetch per agent per session — the catalog file only
 * changes when the CLI refreshes it, so re-invoking per picker open is waste. */
const cache = new Map<string, Promise<AgentModel[]>>();

/** The pickable models for `agent`: live host catalog when one exists, else the
 * static `AGENT_MODELS` entry (empty for agents with no wired flag). Never
 * rejects — any IPC failure falls back to the static list. */
export function agentModelCatalog(agent: string): Promise<AgentModel[]> {
  let hit = cache.get(agent);
  if (!hit) {
    hit = invokeQuiet<AgentModel[] | null>("agent_model_catalog", { agent })
      .then((live) => (live && live.length ? live : (AGENT_MODELS[agent] ?? [])))
      .catch(() => AGENT_MODELS[agent] ?? [])
      .then((models) => {
        // An empty settle is a transient condition, not a catalog. Memoizing it
        // would leave the picker empty for the session, so drop the entry and
        // let the next picker open retry.
        if (models.length === 0) cache.delete(agent);
        return models;
      });
    cache.set(agent, hit);
  }
  return hit;
}

/** Test hook — drop memoized fetches. */
export function resetModelCatalogCache(): void {
  cache.clear();
}
