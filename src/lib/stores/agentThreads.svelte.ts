/**
 * Multi-thread persistence for the SSH Agent chat (Void's chatThreadService
 * analog). Each host keeps several conversations ("threads"), one active at a
 * time; the component mirrors the active thread's fields into its working
 * `$state` and snapshots them back here via `saveThreadStore`. Threads persist in
 * `localStorage` per connection so they survive reopening the tab.
 *
 * Only the durable conversation belongs to a thread (transcript, the CLI
 * `--resume`/thread id, the brain + model + mode + cwd, the agent's plan).
 * Transient UI (streaming buffers, attachments, sign-in, errors) is never
 * persisted — it resets on switch.
 */
import { browser } from "$app/environment";

import type { TodoItem } from "$lib/ssh/agent";
import type { AgentProvider } from "$lib/stores/agentProviderPref.svelte";

export type Role = "user" | "assistant";

/** One of the agent's own tool calls, mirrored read-only (Claude Code / Codex). */
export interface ToolActivity {
  name: string;
  summary: string;
  result?: string;
  isError?: boolean;
}

export interface Turn {
  role: Role;
  /** What the model receives (may carry `@path` attachment references). */
  content: string;
  /** What the bubble shows, when it differs from `content`. */
  display?: string;
  /** Attachment file names shown as chips under a user turn. */
  attachments?: string[];
  tools?: ToolActivity[];
  /** The agent's reasoning for this turn, when it surfaced any. */
  reasoning?: string;
}

/** Void-style chat mode → permission posture (see SshAgent `MODE_PERMISSION`). */
export type ChatMode = "normal" | "gather" | "agent";

/** One persisted conversation for a host. */
export interface AgentThread {
  id: string;
  /** Derived from the first user message; "New chat" until then. */
  title: string;
  provider: AgentProvider;
  /** ollama model name (unused by the CLI agents). */
  model: string;
  /** CLI `--model` override (null = the provider's own default). */
  cliModel: string | null;
  chatMode: ChatMode;
  cwd: string;
  /** Claude/Codex session id threaded across turns with `--resume`. */
  sessionId: string | null;
  turns: Turn[];
  /** The agent's own plan from the last turn (TodoWrite / todo_list). */
  todos: TodoItem[];
  /** Epoch ms of the last change; drives the history sort. */
  lastModified: number;
}

const storeKey = (connId: string) => `portbay.agent.threads.${connId}`;

/** A short, single-line title from a thread's first user message. */
export function threadTitle(thread: AgentThread): string {
  const firstUser = thread.turns.find((t) => t.role === "user");
  const raw = (firstUser?.display ?? firstUser?.content ?? "").trim();
  if (!raw) return "New chat";
  const oneLine = raw.replace(/\s+/g, " ");
  return oneLine.length > 42 ? `${oneLine.slice(0, 42)}…` : oneLine;
}

/** Load a connection's persisted threads, or `null` if none/unreadable. */
export function loadThreadStore(connId: string): { threads: AgentThread[]; activeId: string } | null {
  if (!browser) return null;
  try {
    const raw = localStorage.getItem(storeKey(connId));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { threads?: AgentThread[]; activeId?: string };
    if (!parsed || !Array.isArray(parsed.threads) || parsed.threads.length === 0) return null;
    return { threads: parsed.threads, activeId: parsed.activeId ?? parsed.threads[0].id };
  } catch {
    return null;
  }
}

/** Persist a connection's threads + which one is active. Best-effort. */
export function saveThreadStore(connId: string, threads: AgentThread[], activeId: string): void {
  if (!browser) return;
  try {
    localStorage.setItem(storeKey(connId), JSON.stringify({ threads, activeId }));
  } catch {
    /* quota / disabled storage — non-fatal, threads just won't persist */
  }
}
