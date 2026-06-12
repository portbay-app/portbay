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

/** Result of `ollama_embed` — one vector per input, for the Embeddings playground. */
export interface OllamaEmbedResult {
  model: string;
  embeddings: number[][];
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

/**
 * One speech-to-text catalog entry from the live PortBay Model Catalog (signed
 * manifest + cache + bundled fallback — see `commands/model_catalog.rs`).
 */
export interface SttCatalogModel {
  id: string;
  /** "whisper" | "parakeet" | "qwen3" | "cohere" | "nemotron". */
  engine: string;
  displayName: string;
  repoModel: string;
  /** Parakeet generation ("v2" | "v3"); absent for Whisper. */
  parakeetVersion?: string;
  /** Display-only approximation; the installed size is measured on disk. */
  approxSizeBytes: number;
  languages: string;
  speedNote: string;
  recommended: boolean;
  /** Whether capture emits live partial transcripts (batch models don't). */
  streaming: boolean;
  /** Model-weights license label (e.g. "MIT", "CC-BY-4.0"). */
  license?: string;
  /** Authoritative license/model card URL. */
  licenseUrl?: string;
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
  /** Catalog served from cache/bundled after a failed live refresh. */
  catalogStale: boolean;
  /** Provenance of `catalog`: "live" | "cache" | "bundled". */
  catalogSource: string;
  /** Microphone TCC status for PortBay: "authorized" | "denied" |
   * "restricted" | "not_determined" | "unknown". */
  micPermission: string;
}

/** Channel events streamed by `stt_download_model`. */
export type SttDownloadEvent =
  | { kind: "progress"; fraction: number; phase: string }
  | { kind: "done"; success: boolean; cancelled: boolean; error: string | null };

/** One selectable text-to-speech voice. */
export interface TtsVoice {
  id: string;
  label: string;
}

/** One text-to-speech model from the PortBay Model Catalog (Kokoro today). */
export interface TtsCatalogModel {
  id: string;
  engine: string;
  displayName: string;
  repoModel: string;
  approxSizeBytes: number;
  languages: string;
  speedNote: string;
  recommended: boolean;
  voices: TtsVoice[];
  defaultVoice?: string;
  /** Model-weights license label. */
  license?: string;
  /** Authoritative license/model card URL. */
  licenseUrl?: string;
}

/** Everything the AI page's Text-to-Speech playground renders, one call. */
export interface TtsOverview {
  status: SttStatus;
  catalog: TtsCatalogModel[];
  installed: SttInstalledModel[];
  modelsDir: string;
  catalogStale: boolean;
  /** Provenance of the catalog list: "live" | "cache" | "bundled". */
  catalogSource: string;
}

/** One on-device image-generation model from the PortBay Model Catalog
 *  (FLUX / SD3 today, via the portbay-imagegen DiffusionKit sidecar). */
export interface ImageCatalogModel {
  id: string;
  engine: string; // "flux" | "sd" | "sdxl" | "stable-diffusion"
  displayName: string;
  repoModel: string;
  approxSizeBytes: number;
  /** Default diffusion steps for this model (FLUX-schnell ≈ 4, SD ≈ 25). */
  defaultSteps: number;
  /** Native/recommended square resolution (e.g. 1024). */
  defaultSize: number;
  speedNote: string;
  recommended: boolean;
  /** Optional override for the HF glob the sidecar fetches (community layouts
   *  like SD-Turbo's `original/compiled/`); omitted = engine default. */
  compiledGlob?: string;
  /** Model-weights license label. */
  license?: string;
  /** Authoritative license/model card URL. */
  licenseUrl?: string;
}

/** Apple Image Playground availability (the `imageplayground_check` command).
 *  `reason` ∈ requires_macos_15_4 | apple_intelligence_unavailable |
 *  unsupported_device | sidecar_missing | sidecar_failed | unsupported. */
export interface ImagePlaygroundStatus {
  available: boolean;
  reason?: string | null;
}

export interface ImagegenInstalledModel {
  id: string;
  engine: string;
  sizeBytes: number;
}

/** Everything the Image-generation category + playground render, one call. */
export interface ImagegenOverview {
  status: SttStatus;
  catalog: ImageCatalogModel[];
  installed: ImagegenInstalledModel[];
  modelsDir: string;
  catalogStale: boolean;
  /** "live" (verified manifest), "cache", or "bundled". */
  catalogSource: string;
}

/** Channel events streamed by `imagegen_generate` — per-step diffusion
 *  progress, then a terminal frame carrying the base64 PNG. */
export type ImagegenGenerateEvent =
  | { kind: "progress"; fraction: number; step: number; totalSteps: number }
  | { kind: "done"; imageBase64: string }
  | { kind: "error"; message: string };
