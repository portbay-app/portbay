export type OllamaRunState =
  | "stopped"
  | "starting"
  | "running_managed"
  | "running_external"
  | "unreachable_managed";

export interface AiPrefs {
  endpoint: string;
  modelsDir: string;
  binaryPath: string;
  keepAlive: string;
  flashAttention: boolean;
  origins: string;
  numParallel: number | null;
  debug: boolean;
  modelDownloadThreads: number | null;
  noHistory: boolean;
  noPrune: boolean;
  scheduleSpread: boolean;
  multiUserCache: boolean;
  kvCacheType: string;
  gpuOverhead: number | null;
  loadTimeout: string;
  maxLoadedModels: number | null;
  maxQueue: number | null;
  llmLibrary: string;
  httpProxy: string;
  httpsProxy: string;
  noProxy: string;
}

export interface OllamaStatus {
  state: OllamaRunState;
  endpoint: string;
  version: string | null;
  pid: number | null;
  external: boolean;
  detail: string | null;
  portConflict: string | null;
}

export interface OllamaBinaryStatus {
  path: string | null;
  version: string | null;
  detected: boolean;
  installHint: string;
}

export interface DiskUsage {
  path: string;
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  volume: string | null;
}

export interface OllamaModel {
  name: string;
  size: number;
  modifiedAt: string | null;
  family: string | null;
  parameterSize: string | null;
  quantizationLevel: string | null;
  /** Manifest digest (sha256 hex) from `/api/tags`; compared against the
   * library tag's 12-char digest prefix to detect available updates. */
  digest?: string | null;
}

export interface OllamaLoadedModel {
  name: string;
  size: number;
  sizeVram: number;
  expiresAt: string | null;
  processor: string | null;
}

export interface StarterModel {
  name: string;
  label: string;
  fit: string;
  sizeHint: string;
}

export interface OllamaOverview {
  config: AiPrefs;
  status: OllamaStatus;
  binary: OllamaBinaryStatus;
  installedModels: OllamaModel[];
  loadedModels: OllamaLoadedModel[];
  modelsDisk: DiskUsage;
  logPath: string;
  starterModels: StarterModel[];
  activePull: ActivePull | null;
}

/** The in-flight (or last terminal, until dismissed) model pull, carried on
 * the overview so the page re-attaches after navigating away and back. */
export interface ActivePull {
  pullId: string;
  model: string;
  event: PullEvent;
}

export interface PullEvent {
  status: string;
  digest: string | null;
  total: number | null;
  completed: number | null;
  error: string | null;
  done: boolean;
}

export interface SmokeTestResult {
  response: string;
  model: string;
  totalDurationMs: number | null;
}

/** Streamed events from `ollama_test_stream` — the live Test prompt run.
 * Tagged by `kind`, mirroring the install/pull channels. */
export type GenerateEvent =
  | { kind: "token"; text: string }
  | { kind: "thinking"; text: string }
  | {
      kind: "done";
      model: string;
      totalDurationMs: number | null;
      loadDurationMs: number | null;
      evalCount: number | null;
      evalDurationMs: number | null;
      promptEvalCount: number | null;
      promptEvalDurationMs: number | null;
    }
  | { kind: "error"; message: string }
  | { kind: "stopped" };

/** Managed-binary manifest check (`ollama_update_check`). */
export interface OllamaUpdateCheck {
  installedVersion: string | null;
  latestVersion: string | null;
  updateAvailable: boolean;
}

/** One model from the live ollama.com/library catalog. */
export interface LibraryModel {
  name: string;
  description: string;
  capabilities: string[];
  sizes: string[];
  pullCount: string | null;
  updated: string | null;
  /** ollama.com "cloud" badge — inference runs remotely, nothing to download. */
  cloud?: boolean;
}

export interface LibraryCatalog {
  fetchedAt: string;
  models: LibraryModel[];
  /** True when served from disk cache after a failed live refresh. */
  stale: boolean;
}

/** One pullable tag from a model's ollama.com tags page. */
export interface LibraryTag {
  name: string;
  size: string | null;
  context: string | null;
  input: string | null;
  latest: boolean;
  /** 12-char manifest digest prefix from the ollama.com tags page — the
   * local `/api/tags` digest starts with this when the install is current. */
  digest?: string | null;
}

export interface LibraryTagsResult {
  model: string;
  fetchedAt: string;
  tags: LibraryTag[];
  stale: boolean;
}

// ---- Local speech-to-text (portbay-stt sidecar) ----

/** Sidecar availability, probed via `stt_status` / inside `stt_overview`.
 * `reason`: requires_macos_14 | sidecar_missing | sidecar_failed | unsupported. */
export interface SttStatus {
  available: boolean;
  reason?: string;
  /** Engine libraries linked into the sidecar ("whisper", "parakeet"). */
  engines?: string[];
}

/** One curated speech-to-text catalog entry (static, ships with the sidecar). */
export interface SttCatalogModel {
  id: string;
  engine: "whisper" | "parakeet";
  displayName: string;
  repoModel: string;
  /** Display-only approximation; the installed size is measured on disk. */
  approxSizeBytes: number;
  languages: string;
  speedNote: string;
  recommended: boolean;
  /** Whether capture emits live partial transcripts (batch models don't). */
  streaming: boolean;
}

/** One installed (fully downloaded) STT model. */
export interface SttInstalledModel {
  id: string;
  engine: string;
  sizeBytes: number;
}

/** Everything the AI page's "Speech to text" section renders, one call. */
export interface SttOverview {
  status: SttStatus;
  catalog: SttCatalogModel[];
  installed: SttInstalledModel[];
  modelsDir: string;
  disk: DiskUsage;
}

/** Channel events streamed by `stt_download_model`. */
export type SttDownloadEvent =
  | { kind: "progress"; fraction: number; phase: string }
  | { kind: "done"; success: boolean; cancelled: boolean; error: string | null };
