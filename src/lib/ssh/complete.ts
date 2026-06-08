/**
 * SSH transport for inline completion: discover a usable ollama model on the
 * host, and fetch one FIM completion over the cached agent session
 * (`ssh_ollama_complete` → host `/api/generate`). Everything here is best-effort
 * and silent — any failure resolves to null so the surface simply shows no
 * ghost, never a toast or a credential prompt mid-typing.
 */
import { invokeQuiet } from "$lib/ipc";
import { openAgent } from "$lib/ssh/agent";

export interface CompletionModel {
  model: string;
  port: number;
  /** True when the model name looks purpose-built for code/FIM (vs a general
      chat model pressed into service for the terminal line hint). */
  isCodeModel: boolean;
}

// Names that signal a FIM/code-completion model on the host.
const CODE_MODEL =
  /(coder|starcoder|deepseek.*coder|codellama|code-?llama|codegemma|stable-?code|qwen.*coder|granite.*code|codeqwen|codestral)/i;

/**
 * Find a completion model on the host. Prefers a code/FIM model; for the
 * terminal next-command hint a general model is acceptable (`allowGeneral`),
 * since completing a shell line doesn't need true FIM. Returns null when the
 * host has no ollama models (the feature then stays dormant — Claude/Codex CLI
 * hosts can't do low-latency FIM).
 */
export async function detectCompletionModel(
  connectionId: string,
  label: string,
  allowGeneral = false,
): Promise<CompletionModel | null> {
  try {
    const info = await openAgent(connectionId, label);
    const models = info.ollamaModels ?? [];
    if (models.length === 0) return null;
    const code = models.find((m) => CODE_MODEL.test(m));
    if (code) return { model: code, port: info.port, isCodeModel: true };
    if (allowGeneral) return { model: models[0], port: info.port, isCodeModel: false };
    return null;
  } catch {
    return null;
  }
}

/**
 * One raw FIM completion. `prefix`/`suffix` straddle the cursor; the host model
 * fills the middle. `signal` aborts the wait (we can't kill the remote curl
 * mid-flight, but the short server-side `--max-time` bounds it and we drop the
 * result). Returns null on any error.
 */
export async function fetchCompletion(
  connectionId: string,
  cm: CompletionModel,
  prefix: string,
  suffix: string,
  signal: AbortSignal,
  numPredict = 64,
): Promise<string | null> {
  if (signal.aborted) return null;
  try {
    const text = await invokeQuiet<string>("ssh_ollama_complete", {
      connectionId,
      model: cm.model,
      prefix,
      suffix,
      port: cm.port,
      numPredict,
    });
    if (signal.aborted) return null;
    return text && text.length > 0 ? text : null;
  } catch {
    return null;
  }
}
