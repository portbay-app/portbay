/**
 * Frontend bridge to the server-side AI agent.
 *
 * The model runs on the remote host; we open/cache a session (through
 * connectWithPrompt, so the credential prompt is shared) and relay chat turns +
 * approved commands to it. The agent loop, system prompt, and approval gate live
 * in the SshAgent component — the backend never runs an unapproved command.
 */
import { Channel } from "@tauri-apps/api/core";

import { invokeQuiet } from "$lib/ipc";
import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
import type { ExecResult } from "$lib/types/sshTunnels";

/** What model tooling the host offers (mirrors Rust `AgentInfo`). */
export interface AgentInfo {
  hasCurl: boolean;
  hasWget: boolean;
  hasOllama: boolean;
  hasLlm: boolean;
  ollamaModels: string[];
  port: number;
}

/** A streamed event from one chat turn (mirrors Rust `AgentEvent`). */
export type AgentEvent =
  | { type: "token"; text: string }
  | { type: "done"; content: string }
  | { type: "error"; message: string };

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

/** Connect (prompting once if needed), cache the session, and probe the host. */
export async function openAgent(connectionId: string, label: string): Promise<AgentInfo> {
  return connectWithPrompt(connectionId, label, (cred) =>
    invokeQuiet<AgentInfo>("ssh_agent_open", {
      connectionId,
      password: cred?.kind === "password" ? cred.secret : undefined,
      passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
    }),
  );
}

/** Relay one chat turn; `onEvent` streams tokens, then a done/error. Resolves
    when the turn finishes. */
export async function agentChat(
  connectionId: string,
  model: string,
  messages: ChatMessage[],
  port: number,
  onEvent: (event: AgentEvent) => void,
): Promise<void> {
  const channel = new Channel<AgentEvent>();
  channel.onmessage = onEvent;
  await invokeQuiet("ssh_agent_chat", {
    connectionId,
    model,
    messages,
    port,
    onEvent: channel,
  });
}

/** Run one user-approved command on the agent's cached session. */
export function agentRun(connectionId: string, command: string): Promise<ExecResult> {
  return invokeQuiet<ExecResult>("ssh_agent_run", { connectionId, command });
}

/** Drop the agent's cached session. Best-effort. */
export function agentClose(connectionId: string): void {
  void invokeQuiet("ssh_agent_close", { connectionId }).catch(() => {});
}
