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
  /** `claude` (Claude Code) is on the host's PATH. */
  hasClaude: boolean;
  /** `codex` is on the host's PATH. */
  hasCodex: boolean;
  ollamaModels: string[];
  port: number;
}

/** One entry in an agent-authored task list (Claude `TodoWrite` / Codex
    `todo_list`). Mirrors Rust `TodoItem`. */
export type TodoItem = {
  text: string;
  status: "pending" | "in_progress" | "completed";
};

/** A streamed event from one chat turn (mirrors Rust `AgentEvent`). The CLI
    agents (Claude Code / Codex) additionally emit `session`/`toolUse`/`toolResult`
    and `todos` (the agent's own plan). */
export type AgentEvent =
  | { type: "token"; text: string }
  | { type: "reasoning"; text: string }
  | { type: "session"; id: string }
  | { type: "toolUse"; name: string; summary: string }
  | { type: "toolResult"; summary: string; isError: boolean }
  | { type: "todos"; items: TodoItem[] }
  | { type: "done"; content: string }
  /** `auth` is true when the failure looks like the host CLI isn't signed in,
      so the UI can offer in-app sign-in instead of a bare error. */
  | { type: "error"; message: string; auth?: boolean };

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

/** Drive the host's official agent CLI (Claude Code / Codex) for one turn,
    streaming its own events. `resumeId` continues a Claude conversation across
    turns; `permissionMode` is Claude's official `--permission-mode`. */
export async function agentCliChat(
  connectionId: string,
  provider: "claude" | "codex",
  prompt: string,
  permissionMode: string | null,
  resumeId: string | null,
  model: string | null,
  cwd: string | null,
  onEvent: (event: AgentEvent) => void,
): Promise<void> {
  const channel = new Channel<AgentEvent>();
  channel.onmessage = onEvent;
  await invokeQuiet("ssh_agent_cli_chat", {
    connectionId,
    provider,
    prompt,
    permissionMode: permissionMode ?? undefined,
    resumeId: resumeId ?? undefined,
    model: model ?? undefined,
    cwd: cwd ?? undefined,
    onEvent: channel,
  });
}

/** Run one user-approved command on the agent's cached session. `cwd` runs it in
    that working directory (so a proposed `cp`/`mv` lands in the project). */
export function agentRun(
  connectionId: string,
  command: string,
  cwd?: string | null,
): Promise<ExecResult> {
  return invokeQuiet<ExecResult>("ssh_agent_run", {
    connectionId,
    command,
    cwd: cwd ?? undefined,
  });
}

/** Upload a chat attachment's bytes (base64) to the host and resolve to its
    absolute remote path. For clipboard-pasted images, which have no local path. */
export function agentUploadBytes(
  connectionId: string,
  turnId: string,
  name: string,
  dataBase64: string,
): Promise<string> {
  return invokeQuiet<string>("ssh_agent_upload_bytes", {
    connectionId,
    turnId,
    name,
    dataBase64,
  });
}

/** Upload a local file (picker / OS drag-drop) to the host and resolve to its
    absolute remote path. The backend reads the local file, so no fs plugin. */
export function agentUploadPath(
  connectionId: string,
  turnId: string,
  name: string,
  localPath: string,
): Promise<string> {
  return invokeQuiet<string>("ssh_agent_upload_path", {
    connectionId,
    turnId,
    name,
    localPath,
  });
}

/** Best-effort deletion of one turn's remote staged attachments after the agent
    has consumed them. Keeps pasted screenshots from lingering on the host. */
export function agentCleanupAttachments(connectionId: string, turnId: string): Promise<void> {
  return invokeQuiet("ssh_agent_cleanup_attachments", { connectionId, turnId });
}

/** Start an ephemeral local→remote port forward over the agent's session (e.g.
    so codex login's localhost:1455 callback on the host is reachable locally). */
export function agentForwardStart(
  connectionId: string,
  localPort: number,
  remotePort: number,
): Promise<void> {
  return invokeQuiet("ssh_agent_forward_start", { connectionId, localPort, remotePort });
}

/** Stop the sign-in port forward for a connection. Best-effort. */
export function agentForwardStop(connectionId: string): void {
  void invokeQuiet("ssh_agent_forward_stop", { connectionId }).catch(() => {});
}

/** Stop the in-flight chat turn for a connection (Stop button / Escape). The
    backend closes the streaming channel so the remote model/CLI exits, and the
    running turn resolves with whatever streamed so far. Best-effort. */
export function agentAbort(connectionId: string): void {
  void invokeQuiet("ssh_agent_abort", { connectionId }).catch(() => {});
}

/** Drop the agent's cached session. Best-effort. */
export function agentClose(connectionId: string): void {
  void invokeQuiet("ssh_agent_close", { connectionId }).catch(() => {});
}
