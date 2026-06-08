<script lang="ts">
  import { onMount } from "svelte";
  import { Channel } from "@tauri-apps/api/core";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import ModelMark from "$lib/components/atoms/ModelMark.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import SmartDictationPanel from "$lib/components/ai/SmartDictationPanel.svelte";
  import { invokeQuiet, safeInvoke } from "$lib/ipc";
  import { openUrl } from "$lib/security/openUrl";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import type {
    AiPrefs,
    GenerateEvent,
    LibraryCatalog,
    LibraryModel,
    LibraryTag,
    LibraryTagsResult,
    OllamaLoadedModel,
    OllamaModel,
    OllamaOverview,
    OllamaUpdateCheck,
    PullEvent,
    SttDownloadEvent,
    SttOverview,
  } from "$lib/types/ai";

  type AiView = "home" | "models" | "test" | "dictation" | "config" | "logs";
  // Backs the "AI sections" count badge — one source so the number can't go
  // stale when a view is added or removed (keep aligned with the nav buttons).
  const AI_VIEWS = ["home", "models", "test", "dictation", "config", "logs"] as const satisfies readonly AiView[];
  type ModelVariant = {
    name: string;
    /** Bare library model the variant belongs to (key for tags lookups). */
    model: string;
    sizeHint: string;
    fit: string;
    workload: string;
    /** Relative freshness from ollama.com, e.g. "5 months ago". */
    updated: string | null;
    recommended?: boolean;
  };
  type VariantSort = "popular" | "updated" | "size-asc" | "size-desc";
  type ModelFamily = {
    id: string;
    label: string;
    vendor: string;
    summary: string;
    badge: string;
    models: LibraryModel[];
    variants: ModelVariant[];
  };
  type FamilyDef = Omit<ModelFamily, "models" | "variants"> & {
    match: (model: LibraryModel) => boolean;
  };

  // Family presentation + membership rules over the LIVE ollama.com library
  // (fetched by the `ollama_library` command, disk-cached for offline). The
  // variant lists themselves are no longer hardcoded — a model Ollama
  // publishes tomorrow shows up under its family without an app update.
  // Match order matters: first hit wins (qwen3 before the qwen catch-all).
  const FAMILY_DEFS: FamilyDef[] = [
    {
      id: "qwen3",
      label: "Qwen 3",
      vendor: "Alibaba Cloud",
      summary: "Newer Qwen line for reasoning, multilingual work, embeddings, and coding.",
      badge: "new generation",
      match: (m) => m.name.startsWith("qwen3"),
    },
    {
      id: "qwen25",
      label: "Qwen 2.5",
      vendor: "Alibaba Cloud",
      summary: "Reliable local default for dictation, coding, and general workspace prompts.",
      badge: "balanced",
      match: (m) => m.name.startsWith("qwen") || m.name.startsWith("qwq") || m.name.startsWith("codeqwen"),
    },
    {
      id: "llama",
      label: "Llama",
      vendor: "Meta",
      summary: "Broadly supported general-purpose models with strong ecosystem compatibility.",
      badge: "general",
      match: (m) => m.name.startsWith("llama") || m.name.startsWith("codellama") || m.name.startsWith("tinyllama"),
    },
    {
      id: "deepseek",
      label: "DeepSeek",
      vendor: "DeepSeek",
      summary: "Reasoning and coding models for harder technical prompts.",
      badge: "reasoning",
      match: (m) => m.name.startsWith("deepseek"),
    },
    {
      // Moonshot's K2 line ships on Ollama as cloud-only tags: the pull is a
      // tiny stub, inference runs on Ollama's cloud (ollama signin) — so
      // prompts DO leave the machine, unlike everything else in this catalog.
      id: "kimi",
      label: "Kimi K2",
      vendor: "Moonshot AI",
      summary:
        "Trillion-parameter agentic models, served through Ollama's cloud — pulled like a local model, but inference runs remotely and requires an ollama.com sign-in.",
      badge: "cloud",
      match: (m) => m.name.startsWith("kimi"),
    },
    {
      id: "gemma",
      label: "Gemma",
      vendor: "Google",
      summary: "Compact, multilingual models with image-capable variants.",
      badge: "multimodal",
      match: (m) => m.name.includes("gemma"),
    },
    {
      id: "mistral",
      label: "Mistral",
      vendor: "Mistral AI",
      summary: "Efficient general, edge, and coding models used widely in local stacks.",
      badge: "efficient",
      match: (m) => /(mistral|mixtral|ministral|devstral|magistral|codestral|mathstral)/.test(m.name),
    },
    {
      id: "phi",
      label: "Phi",
      vendor: "Microsoft",
      summary: "Small, capable models for fast local tasks on modest hardware.",
      badge: "small",
      match: (m) => m.name.startsWith("phi"),
    },
    {
      id: "exaone",
      label: "EXAONE",
      vendor: "LG AI Research",
      summary: "Reasoning-oriented bilingual models for math, code, and Korean/English work.",
      badge: "reasoning",
      match: (m) => m.name.startsWith("exaone"),
    },
    {
      id: "embeddings",
      label: "Embeddings",
      vendor: "Search/RAG",
      summary: "Small models for semantic search, retrieval, and local knowledge indexes.",
      badge: "retrieval",
      match: (m) => m.capabilities.includes("embedding") || m.name.includes("embed"),
    },
    {
      id: "vision",
      label: "Vision",
      vendor: "Multimodal",
      summary: "Image-aware local models for screenshots, documents, and visual inspection.",
      badge: "image",
      match: (m) => m.capabilities.includes("vision") || m.name.includes("llava") || m.name.includes("moondream"),
    },
  ];

  // Live library models no FAMILY_DEFS rule claims (new vendors, specialty
  // models) — still browsable instead of silently dropped.
  const OTHER_FAMILY: Omit<ModelFamily, "models" | "variants"> = {
    id: "other",
    label: "More models",
    vendor: "ollama.com",
    summary: "Everything else in the live Ollama library, in popularity order.",
    badge: "library",
  };

  // Hand-written PortBay guidance layered over the live data, keyed by exact
  // variant name first, then bare model name. Models without an entry fall
  // back to ollama.com's own description.
  const CURATED: Record<string, { fit?: string; recommended?: boolean }> = {
    "qwen2.5:3b": { fit: "Fast rewrite and chat on constrained laptops." },
    "qwen2.5:7b": { fit: "Best PortBay starter for dictation and daily local AI.", recommended: true },
    "qwen2.5:14b": { fit: "Better quality when memory headroom is available." },
    "qwen2.5-coder:7b": { fit: "Repo Q&A, shell help, and developer prompts." },
    "qwen2.5-coder:14b": { fit: "More capable coding model for larger machines." },
    "qwen3:4b": { fit: "Current Qwen model for lightweight chat and summaries." },
    "qwen3:8b": { fit: "Good local reasoning choice with manageable size.", recommended: true },
    "qwen3:14b": { fit: "Higher quality for larger memory machines." },
    "qwen3-coder:30b": { fit: "Repository-scale coding and long-horizon tasks." },
    "qwen3-embedding": { fit: "Semantic search and RAG indexes." },
    "llama3.2:1b": { fit: "Fast summarization and basic automation." },
    "llama3.2:3b": { fit: "Good fit for smaller local machines.", recommended: true },
    "llama3.1:8b": { fit: "General chat, notes, and task assistance." },
    "llama3.3:70b": { fit: "High quality on workstations with substantial memory." },
    "deepseek-r1:1.5b": { fit: "Quick chain-of-thought style checks." },
    "deepseek-r1:7b": { fit: "Local reasoning on common developer hardware." },
    "deepseek-r1:8b": { fit: "Updated distilled reasoning default.", recommended: true },
    "deepseek-r1:14b": { fit: "Better reasoning when memory allows." },
    "deepseek-coder-v2:16b": { fit: "Coding specialist for refactors and explanations." },
    "kimi-k2.6": { recommended: true },
    "gemma3:1b": { fit: "Very small local text tasks." },
    "gemma3:4b": { fit: "Fast multilingual chat with vision support.", recommended: true },
    "gemma3:12b": { fit: "Higher quality text and image analysis." },
    "gemma3:27b": { fit: "Workstation-grade quality." },
    "mistral:7b": { fit: "Classic fast local assistant." },
    devstral: { fit: "Software engineering agent workflows.", recommended: true },
    "phi3.5": { fit: "Lightweight chat and rewriting." },
    "phi4-mini": { fit: "Small reasoning and automation model.", recommended: true },
    phi4: { fit: "Higher-quality local reasoning." },
    "phi4-reasoning": { fit: "Stronger step-by-step reasoning for harder problems." },
    "exaone-deep:2.4b": { fit: "Fast reasoning and Korean/English prompts." },
    "exaone-deep:7.8b": { fit: "Practical reasoning default.", recommended: true },
    "exaone-deep:32b": { fit: "Higher-quality math and coding." },
    "nomic-embed-text": { fit: "Popular local embedding baseline.", recommended: true },
    "mxbai-embed-large": { fit: "Higher-quality semantic search." },
    "llava:7b": { fit: "General image understanding." },
    "llava:13b": { fit: "Stronger visual analysis." },
    "granite3.2-vision": { fit: "Visual document understanding.", recommended: true },
  };

  // Bundled snapshot used only when the live fetch fails AND no disk cache
  // exists yet (first launch offline). Deliberately compact.
  const FALLBACK_LIBRARY: LibraryModel[] = [
    { name: "qwen2.5", description: "Qwen 2.5 general models.", capabilities: ["tools"], sizes: ["3b", "7b", "14b"], pullCount: null, updated: null },
    { name: "qwen2.5-coder", description: "Code-specialised Qwen 2.5.", capabilities: ["tools"], sizes: ["7b", "14b"], pullCount: null, updated: null },
    { name: "qwen3", description: "Latest-generation Qwen models.", capabilities: ["tools", "thinking"], sizes: ["4b", "8b", "14b"], pullCount: null, updated: null },
    { name: "qwen3-coder", description: "Agentic Qwen 3 coding models.", capabilities: ["tools"], sizes: ["30b"], pullCount: null, updated: null },
    { name: "qwen3-embedding", description: "Qwen 3 embedding model.", capabilities: ["embedding"], sizes: [], pullCount: null, updated: null },
    { name: "llama3.2", description: "Small Llama 3.2 models from Meta.", capabilities: ["tools"], sizes: ["1b", "3b"], pullCount: null, updated: null },
    { name: "llama3.1", description: "Llama 3.1 from Meta.", capabilities: ["tools"], sizes: ["8b"], pullCount: null, updated: null },
    { name: "llama3.3", description: "Llama 3.3 70B from Meta.", capabilities: ["tools"], sizes: ["70b"], pullCount: null, updated: null },
    { name: "llama3.2-vision", description: "Image-aware Llama 3.2.", capabilities: ["vision"], sizes: ["11b"], pullCount: null, updated: null },
    { name: "deepseek-r1", description: "Open reasoning models from DeepSeek.", capabilities: ["thinking"], sizes: ["1.5b", "7b", "8b", "14b"], pullCount: null, updated: null },
    { name: "deepseek-coder-v2", description: "DeepSeek coding specialist.", capabilities: [], sizes: ["16b"], pullCount: null, updated: null },
    { name: "kimi-k2.6", description: "Latest multimodal agentic Kimi model. Runs on Ollama's cloud.", capabilities: ["tools"], sizes: ["cloud"], pullCount: null, updated: null },
    { name: "kimi-k2-thinking", description: "Moonshot's strongest open thinking model. Runs on Ollama's cloud.", capabilities: ["thinking"], sizes: ["cloud"], pullCount: null, updated: null },
    { name: "gemma3", description: "Compact multimodal models from Google.", capabilities: ["vision"], sizes: ["1b", "4b", "12b", "27b"], pullCount: null, updated: null },
    { name: "mistral", description: "Classic efficient 7B assistant.", capabilities: ["tools"], sizes: ["7b"], pullCount: null, updated: null },
    { name: "ministral-3", description: "Mistral edge models.", capabilities: ["tools"], sizes: ["3b", "8b"], pullCount: null, updated: null },
    { name: "devstral", description: "Mistral software-engineering agent model.", capabilities: ["tools"], sizes: ["24b"], pullCount: null, updated: null },
    { name: "phi3.5", description: "Lightweight Microsoft Phi 3.5.", capabilities: [], sizes: [], pullCount: null, updated: null },
    { name: "phi4-mini", description: "Small Phi 4 reasoning and automation model.", capabilities: ["tools"], sizes: ["3.8b"], pullCount: null, updated: null },
    { name: "phi4", description: "Microsoft Phi 4.", capabilities: [], sizes: ["14b"], pullCount: null, updated: null },
    { name: "phi4-reasoning", description: "Phi 4 tuned for step-by-step reasoning.", capabilities: ["thinking"], sizes: ["14b"], pullCount: null, updated: null },
    { name: "exaone-deep", description: "LG reasoning models for math and code.", capabilities: ["thinking"], sizes: ["2.4b", "7.8b", "32b"], pullCount: null, updated: null },
    { name: "exaone3.5", description: "Instruction-tuned Korean/English assistant.", capabilities: [], sizes: ["7.8b"], pullCount: null, updated: null },
    { name: "nomic-embed-text", description: "Popular local embedding baseline.", capabilities: ["embedding"], sizes: [], pullCount: null, updated: null },
    { name: "mxbai-embed-large", description: "Higher-quality semantic search embeddings.", capabilities: ["embedding"], sizes: [], pullCount: null, updated: null },
    { name: "snowflake-arctic-embed", description: "Retrieval and RAG embeddings.", capabilities: ["embedding"], sizes: [], pullCount: null, updated: null },
    { name: "llava", description: "General image understanding.", capabilities: ["vision"], sizes: ["7b", "13b"], pullCount: null, updated: null },
    { name: "bakllava", description: "Mistral-based multimodal assistant.", capabilities: ["vision"], sizes: [], pullCount: null, updated: null },
    { name: "granite3.2-vision", description: "Visual document understanding.", capabilities: ["vision"], sizes: ["2b"], pullCount: null, updated: null },
  ];

  function buildVariants(models: LibraryModel[]): ModelVariant[] {
    return models.flatMap((model) => {
      // Cloud-only models (kimi-k2, …) carry no size badges — their one real
      // tag is `<name>:cloud`, so surface that instead of a bare name.
      const names =
        model.sizes.length > 0
          ? model.sizes.map((size) => `${model.name}:${size}`)
          : [model.cloud ? `${model.name}:cloud` : model.name];
      return names.map((name) => {
        const exact = CURATED[name];
        const byModel = CURATED[model.name];
        const sizeHint = name.includes(":") ? name.slice(name.indexOf(":") + 1) : "latest";
        return {
          name,
          model: model.name,
          sizeHint: sizeHint.includes("cloud") ? "Cloud — no download" : sizeHint,
          workload: model.capabilities.length > 0 ? model.capabilities.join(" · ") : "general",
          fit: exact?.fit ?? byModel?.fit ?? model.description,
          updated: model.updated,
          // A model-level recommendation only sticks when there is exactly one
          // variant to put it on; otherwise it needs an exact variant key.
          recommended: exact?.recommended ?? (names.length === 1 ? byModel?.recommended : undefined),
        };
      });
    });
  }

  /** "5 months ago" → ~150 (days). Unknown freshness sorts last. */
  function updatedDays(updated: string | null): number {
    if (!updated) return Number.POSITIVE_INFINITY;
    const text = updated.toLowerCase();
    if (text.includes("yesterday")) return 1;
    if (text.includes("hour") || text.includes("minute") || text.includes("just")) return 0;
    const n = Number.parseFloat(text) || 1; // "a month ago" → 1
    if (text.includes("day")) return n;
    if (text.includes("week")) return n * 7;
    if (text.includes("month")) return n * 30;
    if (text.includes("year")) return n * 365;
    return Number.POSITIVE_INFINITY;
  }

  /** "11GB" / "770MB" → GB. NaN when not a byte size. */
  function parseGb(size: string | null | undefined): number {
    if (!size) return Number.NaN;
    const match = /^(\d+(?:\.\d+)?)(MB|GB|TB)$/i.exec(size.trim());
    if (!match) return Number.NaN;
    const n = Number.parseFloat(match[1]);
    const unit = match[2].toUpperCase();
    return unit === "TB" ? n * 1000 : unit === "MB" ? n / 1000 : n;
  }

  /** "14b" / "3.8b" → billions of parameters. NaN for cloud/unknown. */
  function paramBillions(sizeHint: string): number {
    const match = /^(\d+(?:\.\d+)?)([bmt])?\b/i.exec(sizeHint.trim());
    if (!match) return Number.NaN;
    const n = Number.parseFloat(match[1]);
    const unit = (match[2] ?? "b").toLowerCase();
    return unit === "t" ? n * 1000 : unit === "m" ? n / 1000 : n;
  }

  let overview = $state<OllamaOverview | null>(null);
  let config = $state<AiPrefs | null>(null);
  let loading = $state<boolean>(true);
  // True once an initial load failed with no overview to fall back on, so the
  // body can render a retry affordance instead of a blank page.
  let loadError = $state<boolean>(false);
  let busy = $state<string | null>(null);
  let selectedModel = $state<string>("");
  // Empty by default so the input shows its placeholder ("Custom model, e.g.
  // qwen3:8b") instead of a pre-filled model name. Filled when the user types
  // or clicks a catalog variant; the Download button stays disabled until then.
  let pullName = $state<string>("");
  let pullId = $state<string>("");
  let pullEvent = $state<PullEvent | null>(null);
  let pulling = $state<boolean>(false);
  /** Model name of the current/last pull — what Resume re-pulls. */
  let lastPullModel = $state<string>("");
  /** Set when the user asked to pull an already-installed model — offers the
   * update path instead of silently dead-ending. */
  let pullPrompt = $state<string | null>(null);
  /** Inline details panel under an installed model's row: which model is
   * expanded and the parsed `ollama_show_model` (`/api/show`) payload. */
  let detailsName = $state<string>("");
  let detailsData = $state<Record<string, unknown> | null>(null);
  let detailsLoading = $state<boolean>(false);
  let smokePrompt = $state<string>("Reply with one short sentence confirming Ollama is ready.");
  // ---- Test prompt: live streaming run state ("test" view) ----
  type TestPhase = "idle" | "waiting" | "streaming" | "done" | "error" | "stopped";
  let testPhase = $state<TestPhase>("idle");
  let testOutput = $state<string>("");
  let testError = $state<string>("");
  /** Final eval counters from the `done` frame — drives the metrics readout. */
  let testMetrics = $state<Extract<GenerateEvent, { kind: "done" }> | null>(null);
  /** Monotonic-ish wall-clock marks (ms) for the live latency display. The
   * timer below ticks `testElapsedMs` while a run is in flight. */
  let testStartedAt = $state<number>(0);
  let testFirstTokenAt = $state<number>(0);
  let testElapsedMs = $state<number>(0);
  let testTimer: number | null = null;
  /** Guards stale streams: a superseded run's late frames are ignored. */
  let testRunId = $state<string>("");
  /** The streamed-output <pre>, kept pinned to the bottom as tokens arrive. */
  let testOutputEl = $state<HTMLPreElement | null>(null);
  $effect(() => {
    // Re-runs on every token (testOutput) while streaming — follow the tail.
    if (testPhase === "streaming" && testOutput && testOutputEl) {
      testOutputEl.scrollTop = testOutputEl.scrollHeight;
    }
  });
  const testRunning = $derived(testPhase === "waiting" || testPhase === "streaming");
  /** Live token tally while streaming. Ollama emits ~one token per stream
   * frame, so counting frames is a close running estimate; the `done` frame's
   * exact `evalCount` supersedes it when the run finishes. */
  let testTokenCount = $state<number>(0);
  /** Tokens/sec from Ollama's eval counters (generation only, excludes load
   * and prompt-eval time — the number people quote). */
  const testTokensPerSec = $derived.by(() => {
    if (!testMetrics?.evalCount || !testMetrics.evalDurationMs) return null;
    return testMetrics.evalCount / (testMetrics.evalDurationMs / 1000);
  });
  /** Live tokens/sec during the run — counted client-side over generation time
   * (wall-clock since the first token, so load + prompt-eval are excluded, same
   * basis as the final number). Recomputes as the 100ms timer ticks
   * `testElapsedMs`. Null until a couple of tokens land so the rate isn't wild. */
  const testLiveTokensPerSec = $derived.by(() => {
    if (!testFirstTokenAt || testTokenCount < 2) return null;
    const genMs = testElapsedMs - (testFirstTokenAt - testStartedAt);
    if (genMs <= 0) return null;
    return testTokenCount / (genMs / 1000);
  });
  /** Time to first token (ms), available the moment streaming starts. */
  const testTtftMs = $derived(testFirstTokenAt ? testFirstTokenAt - testStartedAt : null);
  /** The tok/s to show right now: exact eval-based once done, live estimate
   * while the run is in flight. */
  const testDisplayTokensPerSec = $derived(
    testPhase === "done" ? testTokensPerSec : testLiveTokensPerSec,
  );
  /** Prefill (prompt-eval) tokens/sec — how fast the model chewed through the
   * prompt before generating. A distinct number from generation tok/s, and the
   * one that dominates on long prompts. Only known once the run finishes. */
  const testPrefillTokensPerSec = $derived.by(() => {
    if (!testMetrics?.promptEvalCount || !testMetrics.promptEvalDurationMs) return null;
    return testMetrics.promptEvalCount / (testMetrics.promptEvalDurationMs / 1000);
  });

  // ---- Power-user test knobs: system prompt, thinking, sampling options ----
  let testSystem = $state<string>("");
  let testThink = $state<boolean>(false);
  /** Sampling knobs as raw strings — blank means "omit, use the model default"
   * rather than pinning the value to zero. Parsed to numbers at send time. */
  let testTemperature = $state<string>("");
  let testTopP = $state<string>("");
  let testTopK = $state<string>("");
  let testRepeatPenalty = $state<string>("");
  let testSeed = $state<string>("");
  let testNumPredict = $state<string>("");
  let testNumCtx = $state<string>("");
  let testOptionsOpen = $state<boolean>(false);
  /** Streamed reasoning (when `think` is on) and its timing. `testThinkingMs`
   * is the gap from the first reasoning token to the first answer token. */
  let testThinking = $state<string>("");
  let testThinkingStartedAt = $state<number>(0);
  let testThinkingMs = $state<number>(0);
  /** Best-effort "does this model reason?" check by name family, so the Think
   * toggle only shows where `think: true` is valid (sending it to a plain model
   * errors). Covers the thinking families in the catalog plus common tags. */
  const THINKING_MODEL_RE =
    /(deepseek-r1|qwen3(?!-coder|-embedding)|qwq|kimi.*thinking|phi4-reasoning|exaone-deep|magistral|gpt-oss|-?reasoning|-?thinking)/i;
  const selectedSupportsThinking = $derived(THINKING_MODEL_RE.test(selectedModel));
  let logLines = $state<string[]>([]);
  let copied = $state<string | null>(null);
  let restartNotice = $state<boolean>(false);
  let menuFilter = $state<string>("");
  let activeView = $state<AiView>("home");
  let selectedFamilyId = $state<string>("qwen25");
  let library = $state<LibraryCatalog | null>(null);
  let libraryError = $state<boolean>(false);
  let libraryRefreshing = $state<boolean>(false);
  let variantFilter = $state<string>("");
  let variantSort = $state<VariantSort>("popular");
  /** Catalog-wide search across every family (and the STT catalog) — the
   * per-family `variantFilter` only narrows the selected family. */
  let catalogQuery = $state<string>("");
  /** Scroll container for the page body — selecting a family scrolls back to
   * the top so the newly loaded catalog is visible (clicking the bottom-most
   * tile otherwise lands you staring below the fold). */
  let mainEl: HTMLElement | null = null;
  /** Download size / context per pullable tag, filled lazily per family. */
  let tagInfo = $state<Record<string, LibraryTag>>({});
  const tagsRequested = new Set<string>();
  /** Streamed progress from the `ollama_install` backend command (same event
   * shape as the language runtime installer). */
  type InstallEvent =
    | { kind: "log"; line: string }
    | { kind: "progress"; downloaded: number; total: number | null }
    | { kind: "done"; success: boolean };
  let installing = $state<boolean>(false);
  let installStatus = $state<string>("");
  let installFailed = $state<boolean>(false);
  /** Result of the managed-binary manifest check (null until requested). */
  let updateCheck = $state<OllamaUpdateCheck | null>(null);
  let checkingUpdate = $state<boolean>(false);

  // ---- Local speech-to-text (portbay-stt sidecar, "Speech to text" view) ----
  let stt = $state<SttOverview | null>(null);
  let sttLoading = $state<boolean>(false);
  /** Catalog id of the in-flight download ("" = none). One at a time — these
   * are multi-GB CoreML bundles; parallel downloads just fight for I/O. */
  let sttDownloadingModel = $state<string>("");
  let sttDownloadId = $state<string>("");
  let sttProgress = $state<{ fraction: number; phase: string } | null>(null);
  /** Terminal failure of the last download, keyed by model id. */
  let sttDownloadError = $state<{ model: string; detail: string } | null>(null);
  let sttBusy = $state<string | null>(null);

  const running = $derived(
    overview?.status.state === "running_managed" ||
      overview?.status.state === "running_external" ||
      overview?.status.state === "unreachable_managed",
  );
  const external = $derived(overview?.status.external === true);
  const managed = $derived(overview?.status.state === "running_managed");
  // PortBay owns the local Ollama lifecycle outright: when an external server
  // answers at the endpoint, Start/Restart take it over (replace it with a
  // managed server) and Stop shuts it down — same model as managed runtimes.
  const canStart = $derived((!running || external) && overview?.binary.detected === true && !busy);
  const canStop = $derived(running && !busy);
  const canRestart = $derived(running && overview?.binary.detected === true && !busy);
  const configDirty = $derived(
    overview && config ? JSON.stringify(overview.config) !== JSON.stringify(config) : false,
  );
  const selectedInstalled = $derived(
    overview?.installedModels.find((m) => m.name === selectedModel) ?? null,
  );
  const endpointSnippet = $derived(config?.endpoint ?? overview?.status.endpoint ?? "");
  const runSnippet = $derived(
    selectedModel
      ? `OLLAMA_HOST=${endpointSnippet.replace(/^https?:\/\//, "")} ollama run ${selectedModel}`
      : "",
  );
  /** Sampling knobs as an Ollama `options` object — blanks dropped so the model
   * keeps its defaults. Shared by the live run and the curl snippets so they
   * stay byte-for-byte the same request. */
  function buildTestOptions(): Record<string, number> {
    const num = (s: string) => {
      const n = Number(s.trim());
      return s.trim() !== "" && Number.isFinite(n) ? n : undefined;
    };
    const out: Record<string, number> = {};
    const pairs: [string, number | undefined][] = [
      ["temperature", num(testTemperature)],
      ["top_p", num(testTopP)],
      ["top_k", num(testTopK)],
      ["repeat_penalty", num(testRepeatPenalty)],
      ["seed", num(testSeed)],
      ["num_predict", num(testNumPredict)],
      ["num_ctx", num(testNumCtx)],
    ];
    for (const [key, value] of pairs) if (value !== undefined) out[key] = value;
    return out;
  }
  // The curl snippets carry the user's ACTUAL request (prompt + system + think +
  // sampling options, JSON-encoded then single-quote-escaped for the shell) so
  // "test the curl request" runs the exact thing the page just ran.
  function curlFor(stream: boolean): string {
    if (!selectedModel) return "";
    const options = buildTestOptions();
    const body: Record<string, unknown> = { model: selectedModel, prompt: smokePrompt, stream };
    if (testSystem.trim()) body.system = testSystem.trim();
    if (selectedSupportsThinking && testThink) body.think = true;
    if (Object.keys(options).length > 0) body.options = options;
    const url = `${endpointSnippet.replace(/\/$/, "")}/api/generate`;
    const quoted = `'${JSON.stringify(body).replace(/'/g, `'\\''`)}'`;
    return `curl ${stream ? "-N " : ""}${url} -d ${quoted}`;
  }
  const curlSnippet = $derived(curlFor(false));
  /** Streaming curl (`-N`, `"stream":true`) — the "copy run stream" command:
   * the same request the live run above issues, watchable in a terminal. */
  const curlStreamSnippet = $derived(curlFor(true));
  const pullPct = $derived(
    pullEvent?.total && pullEvent.completed
      ? Math.min(100, Math.round((pullEvent.completed / pullEvent.total) * 100))
      : null,
  );
  /** Pull card state machine. Every backend failure path emits a terminal
   * error event, so the card can never freeze mid-progress. Errors and
   * cancels both keep Ollama's partial layers on disk — Resume re-pulls and
   * continues from where it left off. */
  type PullPhase = "idle" | "active" | "error" | "cancelled" | "done";
  const pullPhase = $derived.by((): PullPhase => {
    if (!pullEvent) return "idle";
    if (!pullEvent.done) return "active";
    if (pullEvent.error) return "error";
    if (pullEvent.status === "cancelled") return "cancelled";
    return "done";
  });
  /** Ollama's raw stream statuses name layer digests ("pulling dec52a44569a"),
   * not the model — translate to user-facing phases keyed on the model name. */
  function pullStatusLabel(status: string): string {
    const model = lastPullModel || "model";
    if (status === "queued") return `Requesting ${model}…`;
    if (status === "pulling manifest") return `Checking ${model} on ollama.com…`;
    if (/^pulling [0-9a-f]{8,}/.test(status)) return `Downloading ${model}…`;
    if (status.startsWith("verifying")) return `Verifying ${model} download…`;
    if (status.startsWith("writing manifest") || status.startsWith("removing")) return "Finishing up…";
    return status;
  }
  const visibleMenuModels = $derived(
    overview?.installedModels.filter((model) => {
      const q = menuFilter.trim().toLowerCase();
      if (!q) return true;
      return model.name.toLowerCase().includes(q) || (model.family ?? "").toLowerCase().includes(q);
    }) ?? [],
  );
  /** Installed speech-to-text models for the sidebar list — same filter box
   * as the Ollama models, matched on display name, engine, or "speech". */
  const visibleMenuSttModels = $derived.by(() => {
    const installed = stt?.installed ?? [];
    if (installed.length === 0) return [];
    const q = menuFilter.trim().toLowerCase();
    return installed
      .map((m) => ({
        id: m.id,
        sizeBytes: m.sizeBytes,
        engine: m.engine,
        displayName: stt?.catalog.find((c) => c.id === m.id)?.displayName ?? m.id,
      }))
      .filter(
        (m) =>
          !q ||
          m.displayName.toLowerCase().includes(q) ||
          m.engine.toLowerCase().includes(q) ||
          "speech to text".includes(q),
      );
  });
  const libraryModels = $derived(library?.models ?? FALLBACK_LIBRARY);
  const families = $derived.by(() => {
    const grouped = new Map<string, LibraryModel[]>();
    const other: LibraryModel[] = [];
    for (const model of libraryModels) {
      const def = FAMILY_DEFS.find((d) => d.match(model));
      if (def) {
        const bucket = grouped.get(def.id);
        if (bucket) bucket.push(model);
        else grouped.set(def.id, [model]);
      } else {
        other.push(model);
      }
    }
    const result: ModelFamily[] = [];
    for (const { match, ...def } of FAMILY_DEFS) {
      const models = grouped.get(def.id) ?? [];
      if (models.length === 0) continue;
      result.push({ ...def, models, variants: buildVariants(models) });
    }
    if (other.length > 0) {
      result.push({ ...OTHER_FAMILY, models: other, variants: buildVariants(other) });
    }
    return result;
  });
  const selectedFamily = $derived(
    families.find((family) => family.id === selectedFamilyId) ?? families[0],
  );
  // Catalog-tile display order: Speech-to-Text leads (rendered separately,
  // above this list), then Vision, then Embeddings, then everything else in
  // its existing order. This is presentation only — FAMILY_DEFS keeps its
  // match precedence (e.g. vision-capable Gemma still lands in Gemma, not
  // Vision), so reordering here can't reshuffle which family claims a model.
  const FAMILY_TILE_PRIORITY: Record<string, number> = { vision: 0, embeddings: 1 };
  const orderedFamilies = $derived(
    [...families].sort(
      (a, b) => (FAMILY_TILE_PRIORITY[a.id] ?? 2) - (FAMILY_TILE_PRIORITY[b.id] ?? 2),
    ),
  );
  const visibleVariants = $derived.by(() => {
    const q = variantFilter.trim().toLowerCase();
    const list = q
      ? selectedFamily.variants.filter(
          (variant) => variant.name.toLowerCase().includes(q) || variant.fit.toLowerCase().includes(q),
        )
      : selectedFamily.variants;
    if (variantSort === "popular") return list; // library order = popularity
    const sorted = [...list];
    if (variantSort === "updated") {
      sorted.sort((a, b) => updatedDays(a.updated) - updatedDays(b.updated));
      return sorted;
    }
    // Size sorts prefer the exact download GB (once the tags fetch fills it
    // in) and fall back to the parameter count, which orders the same way.
    // Cloud/unknown sizes go last in both directions.
    const value = (variant: ModelVariant) => {
      const gb = parseGb(tagInfo[variant.name]?.size);
      return Number.isFinite(gb) ? gb : paramBillions(variant.sizeHint);
    };
    sorted.sort((a, b) => {
      const av = value(a);
      const bv = value(b);
      const aKnown = Number.isFinite(av);
      const bKnown = Number.isFinite(bv);
      if (aKnown !== bKnown) return aKnown ? -1 : 1;
      if (!aKnown) return 0;
      return variantSort === "size-asc" ? av - bv : bv - av;
    });
    return sorted;
  });
  const installedModelNames = $derived(new Set(overview?.installedModels.map((model) => model.name) ?? []));
  /** Local manifest digests by installed model name (from `/api/tags`). */
  const installedDigests = $derived(
    new Map(overview?.installedModels.map((model) => [model.name, model.digest ?? null]) ?? []),
  );
  /** True only when ollama.com lists a DIFFERENT manifest digest than the
   * local install — a real update. Unknown on either side (offline, custom
   * build, delisted model) means no Update button, not a guess. */
  function hasUpdate(name: string): boolean {
    // `X:latest` installs match the bare `X` key the tags loader fills with
    // whatever `:latest` points at on ollama.com.
    const tag = tagInfo[name] ?? (name.endsWith(":latest") ? tagInfo[name.slice(0, -":latest".length)] : undefined);
    const remote = tag?.digest;
    const local = installedDigests.get(name);
    return !!remote && !!local && !local.startsWith(remote);
  }
  const installedCatalogCount = $derived(
    selectedFamily.variants.filter((model) => installedModelNames.has(model.name)).length,
  );
  /** The shared AI models root, when both engines follow the
   * `<root>/ollama` + `<root>/speech` convention (the defaults, and what
   * the Change… picker writes). Null for mixed/legacy custom paths — the
   * card then lists the two locations individually. */
  const modelsRoot = $derived.by(() => {
    const o = (config?.modelsDir ?? "").replace(/\/$/, "");
    const sttDir = (preferences.value.stt.modelsDir || "").replace(/\/$/, "");
    if (!o.endsWith("/ollama") || !sttDir.endsWith("/speech")) return null;
    const root = o.slice(0, -"/ollama".length);
    return sttDir.slice(0, -"/speech".length) === root ? root : null;
  });
  /** Installed models with no row in any catalog family (custom tags,
   * delisted models) — the only ones that still need a separate list now
   * that catalog rows manage their own installed state in place. */
  const catalogVariantNames = $derived(new Set(families.flatMap((f) => f.variants.map((v) => v.name))));
  const orphanInstalled = $derived(
    overview?.installedModels.filter((m) => !catalogVariantNames.has(m.name)) ?? [],
  );
  type CatalogMatch = ModelVariant & { familyId: string; familyLabel: string };
  const catalogMatches = $derived.by((): CatalogMatch[] => {
    const q = catalogQuery.trim().toLowerCase();
    if (!q) return [];
    const out: CatalogMatch[] = [];
    for (const family of families) {
      for (const variant of family.variants) {
        if (
          variant.name.toLowerCase().includes(q) ||
          variant.fit.toLowerCase().includes(q) ||
          family.label.toLowerCase().includes(q)
        ) {
          out.push({ ...variant, familyId: family.id, familyLabel: family.label });
          if (out.length >= 60) return out; // plenty for a refine-your-query nudge
        }
      }
    }
    return out;
  });
  /** STT catalog entries matching the catalog-wide search. */
  const sttMatches = $derived.by(() => {
    const q = catalogQuery.trim().toLowerCase();
    if (!q || !stt) return [];
    return stt.catalog.filter(
      (m) =>
        m.displayName.toLowerCase().includes(q) ||
        m.engine.toLowerCase().includes(q) ||
        "speech to text".includes(q),
    );
  });
  const activeTitle = $derived(
    activeView === "home"
      ? "Ollama"
      : activeView === "models"
        ? "Models"
        : activeView === "test"
          ? "Test prompt"
          : activeView === "dictation"
            ? "Speech-to-Text"
            : activeView === "config"
              ? "Configuration"
              : "Logs",
  );
  const activeSubtitle = $derived(
    activeView === "home"
      ? "Server health, ownership, storage, and setup guidance."
      : activeView === "models"
        ? "Pull, inspect, copy, unload, and remove local models — chat, vision, embeddings, and speech-to-text."
        : activeView === "test"
          ? "Verify the selected local model end-to-end without leaving PortBay."
          : activeView === "dictation"
            ? "On-device transcription and the rewrite layer — engine, model, dictate-anywhere, and custom terms."
            : activeView === "config"
              ? "Persist ServBay-style Ollama environment settings for the next managed start."
              : "Tail the PortBay-managed Ollama log file.",
  );
  // Speech-to-Text (the dictation view and the STT model family) is on-device
  // Whisper/Parakeet — nothing to do with the Ollama server. On those views the
  // header drops the Ollama logo, the running/stopped pill, and the
  // Start/Stop/Restart controls, which only ever acted on Ollama.
  const sttContext = $derived(
    activeView === "dictation" || (activeView === "models" && selectedFamilyId === "stt"),
  );

  // Pull download sizes (GB) for the selected family's models from their
  // ollama.com tags pages — lazily, once per model, disk-cached backend-side.
  // "More models" is skipped: hundreds of models, and its rows already show
  // the parameter size.
  $effect(() => {
    const family = selectedFamily;
    if (!library || family.id === "other") return;
    for (const model of family.models) {
      if (tagsRequested.has(model.name)) continue;
      tagsRequested.add(model.name);
      void loadModelTags(model.name);
    }
  });

  // Entering the Models view always re-pulls the live ollama.com catalog —
  // no manual Refresh button; the page is current on entry (disk cache
  // serves instantly underneath while the refresh swaps in).
  $effect(() => {
    if (activeView === "models") void loadLibrary(true);
  });

  // Update detection needs the ollama.com digest for every INSTALLED model,
  // wherever it lives (an un-selected family, "More models", an orphan row) —
  // fetch each installed model's tags page once per session. The separate
  // guard set never un-marks on failure: models with no ollama.com page
  // (custom builds) must not retry on every 3 s overview poll.
  const installedTagsRequested = new Set<string>();
  $effect(() => {
    if (activeView !== "models") return;
    for (const model of overview?.installedModels ?? []) {
      const base = model.name.split(":")[0];
      if (installedTagsRequested.has(base)) continue;
      installedTagsRequested.add(base);
      if (!tagsRequested.has(base)) {
        tagsRequested.add(base);
        void loadModelTags(base);
      }
    }
  });

  // Re-walk the speech-to-text inventory when its Models family is opened
  // with nothing loaded yet (the mount-time load below can race a slow
  // sidecar probe, and a failed load shouldn't leave the family view empty).
  $effect(() => {
    if (activeView === "models" && selectedFamilyId === "stt" && !stt && !sttLoading) {
      void refreshStt();
    }
  });

  onMount(() => {
    // Preferences are loaded once at the root layout; no page-level reload (it
    // raced the layout's load and the panel/controls loads on every visit).
    void refresh();
    void loadLibrary();
    // The sidebar's "Installed models" list includes downloaded
    // speech-to-text models, so the STT inventory loads at mount too —
    // quietly; an unavailable sidecar just means no STT rows.
    void refreshStt(false);
    const poll = window.setInterval(() => {
      // Polls run during pulls too: an adopted pull (page remounted) has no
      // live channel, so the stored-event snapshot is its only heartbeat.
      void refresh(false);
    }, 3000);
    const channel = new Channel<string>();
    channel.onmessage = (line) => {
      logLines = [...logLines.slice(-119), line];
    };
    // Quiet: before the managed server has ever run, `ollama.log` doesn't
    // exist yet — a failure here is expected and must not toast on every visit
    // to the page. The log pane simply stays empty until the server writes.
    void invokeQuiet<void>("subscribe_logs", { id: "ollama", onLine: channel });
    return () => {
      window.clearInterval(poll);
      stopTestTimer();
    };
  });

  // Quiet invokes: an offline launch shouldn't toast — the catalog header
  // says "bundled list" and the page stays fully usable.
  async function loadLibrary(refresh = false) {
    if (refresh) {
      // In-flight guard: the models-view `$effect` re-fires `loadLibrary(true)`
      // on every entry; without this, concurrent refreshes race on `library`.
      if (libraryRefreshing) return;
      libraryRefreshing = true;
    }
    try {
      library = await invokeQuiet<LibraryCatalog>("ollama_library", { refresh });
      libraryError = false;
    } catch {
      libraryError = true;
    } finally {
      libraryRefreshing = false;
    }
  }

  async function loadModelTags(model: string) {
    try {
      const result = await invokeQuiet<LibraryTagsResult>("ollama_library_tags", { model });
      const next = { ...tagInfo };
      for (const tag of result.tags) next[tag.name] = tag;
      // Models listed without size badges render as a bare name (e.g.
      // nomic-embed-text), which matches no tag key — give that row whatever
      // `:latest` points at so it still gets a download size.
      const latest = result.tags.find((tag) => tag.latest) ?? result.tags[0];
      if (latest && !next[model]) next[model] = latest;
      tagInfo = next;
    } catch {
      // Size column falls back to the parameter-count hint; un-mark the model
      // so switching back to the family retries instead of staying blank.
      tagsRequested.delete(model);
    }
  }

  async function refresh(showSpinner = true) {
    if (showSpinner) loading = true;
    try {
      const next = await safeInvoke<OllamaOverview>("ollama_overview");
      loadError = false;
      overview = next;
      // Re-attach to a pull that outlived this component (navigated away and
      // back) — the backend command keeps downloading and stores its latest
      // event; the poll below keeps the card moving while the original
      // channel is gone. Terminal states stick backend-side until dismissed,
      // so errors that happened while elsewhere still surface.
      const ap = next.activePull;
      // Never let a stale snapshot hijack a locally live pull (its channel is
      // the truth); adoption is for fresh mounts, where `pulling` is false.
      if (ap && ap.pullId !== pullId && !pulling) {
        pullId = ap.pullId;
        lastPullModel = ap.model;
        pullName = ap.model;
        pullEvent = ap.event;
        pulling = !ap.event.done;
      } else if (ap && ap.pullId === pullId && pullEvent && !pullEvent.done) {
        pullEvent = ap.event;
        if (ap.event.done) pulling = false;
      }
      if (!configDirty || !config) config = structuredClone(next.config);
      if (!selectedModel && next.installedModels.length > 0) {
        selectedModel =
          next.installedModels.find((m) => m.name.startsWith("qwen2.5:7b"))?.name ??
          next.installedModels[0].name;
      }
    } catch {
      // safeInvoke already surfaced the failure as a toast. Only flag the
      // error state when there's no prior overview to keep showing — a failed
      // background poll shouldn't blank out a working page.
      if (!overview) loadError = true;
    } finally {
      loading = false;
    }
  }

  async function runAction(action: "ollama_start" | "ollama_stop" | "ollama_restart") {
    busy = action;
    try {
      // Clone from the raw IPC result, never from the `$state` proxy —
      // structuredClone on a reactive proxy throws DataCloneError ("The
      // object can not be cloned").
      const next = await safeInvoke<OllamaOverview>(action);
      overview = next;
      if (next) config = structuredClone(next.config);
      restartNotice = false;
    } finally {
      busy = null;
    }
  }

  async function saveConfig() {
    if (!config) return;
    busy = "save";
    try {
      await preferences.update({
        ai: $state.snapshot(config),
        dictation: { ...preferences.value.dictation, endpoint: config.endpoint },
      });
      const next = await safeInvoke<OllamaOverview>("ollama_overview");
      overview = next;
      config = structuredClone(next.config);
      restartNotice = managed;
    } finally {
      busy = null;
    }
  }

  async function pullModel(name = pullName, opts: { force?: boolean } = {}) {
    const model = name.trim();
    if (!model || pulling) return;
    pullName = model;
    pullPrompt = null;
    if (!opts.force && overview?.installedModels.some((installed) => installed.name === model)) {
      // Already installed isn't a dead end: a re-pull checks the registry and
      // downloads only changed layers — that IS Ollama's update mechanism.
      selectedModel = model;
      pullPrompt = model;
      return;
    }
    const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    pullId = id;
    lastPullModel = model;
    pullEvent = { status: "queued", digest: null, total: null, completed: null, error: null, done: false };
    pulling = true;
    const channel = new Channel<PullEvent>();
    channel.onmessage = (event) => {
      // A cancelled/superseded pull's stream can keep emitting (its stall
      // watchdog, late error frames) — never let it clobber the current card.
      if (pullId !== id) return;
      pullEvent = event;
      if (event.done) {
        pulling = false;
        void refresh(false);
      }
    };
    try {
      // Quiet invoke: the card itself renders the terminal error state with a
      // Resume button; a toast on top would double-report it.
      await invokeQuiet<void>("ollama_pull_model", { model, pullId: id, onEvent: channel });
    } catch (e) {
      // Backend always emits a terminal event before erroring; this is the
      // belt-and-braces fallback for IPC-level failures.
      if (pullId === id && pullEvent && !pullEvent.done) {
        pullEvent = {
          status: "error",
          digest: null,
          total: null,
          completed: null,
          error: e instanceof Error ? e.message : String(e),
          done: true,
        };
      }
    } finally {
      if (pullId === id) pulling = false;
      void refresh(false);
    }
  }

  /** Re-pull an installed model: checks the registry, downloads only changed
   * layers, finishes instantly when already current. */
  function updateModel(name: string) {
    void pullModel(name, { force: true });
  }

  /** Re-pull after an error/cancel — Ollama keeps completed layers, so this
   * continues from where the download left off. */
  function resumePull() {
    if (lastPullModel) void pullModel(lastPullModel, { force: true });
  }

  function dismissPull() {
    pullEvent = null;
    pullPrompt = null;
    // Clear the backend's sticky terminal state so the overview poll doesn't
    // re-adopt the card we just dismissed (no-op for an active pull).
    void invokeQuiet<void>("ollama_dismiss_pull");
  }

  async function cancelPull() {
    if (!pullId) return;
    const id = pullId;
    // Land the card in its terminal state immediately — on a stalled stream
    // the backend only notices the cancel when data (or the watchdog) next
    // arrives, and the old behaviour left "downloading…" frozen until then.
    pullId = "";
    pulling = false;
    pullEvent = { status: "cancelled", digest: null, total: null, completed: null, error: null, done: true };
    await safeInvoke<void>("ollama_cancel_pull", { pullId: id });
  }

  /** Download the PortBay-managed Ollama build (signed runtimes manifest —
   * the same pipeline as the language runtimes). Also the update and repair
   * path: re-running installs the manifest's newest version, replacing an
   * existing same-version install atomically. */
  async function installOllama() {
    if (installing) return;
    installing = true;
    installFailed = false;
    installStatus = "Preparing download…";
    const channel = new Channel<InstallEvent>();
    channel.onmessage = (event) => {
      if (event.kind === "log") {
        installStatus = event.line;
      } else if (event.kind === "progress") {
        installStatus = event.total
          ? `Downloading… ${Math.min(100, Math.round((event.downloaded / event.total) * 100))}%`
          : `Downloading… ${bytes(event.downloaded)}`;
      } else if (event.kind === "done") {
        installFailed = !event.success;
        if (event.success) installStatus = "Installed.";
        // On failure the preceding log event already carries the reason.
      }
    };
    try {
      await safeInvoke<void>("ollama_install", { onEvent: channel });
      updateCheck = null; // stale after a successful install
      await refresh(false);
    } catch {
      installFailed = true;
    } finally {
      installing = false;
    }
  }

  /** True when the resolved binary is PortBay's own managed install — the
   * only binary PortBay can update/reinstall itself. */
  const managedBinary = $derived(
    overview?.binary.path?.includes("/PortBay/runtimes/ollama/") === true,
  );

  async function checkBinaryUpdate() {
    if (checkingUpdate) return;
    checkingUpdate = true;
    try {
      updateCheck = await safeInvoke<OllamaUpdateCheck>("ollama_update_check");
    } finally {
      checkingUpdate = false;
    }
  }

  async function deleteModel(model: OllamaModel) {
    const choice = await confirmDialog.open({
      title: `Delete ${model.name}?`,
      message: "This removes the local Ollama model files from disk. You can pull the model again later.",
      destructive: true,
      actions: [{ label: "Delete model", value: "delete", tone: "destructive", icon: "trash-2" }],
    });
    if (choice !== "delete") return;
    busy = `delete:${model.name}`;
    try {
      await safeInvoke<void>("ollama_delete_model", { model: model.name });
      if (selectedModel === model.name) selectedModel = "";
      // A dictation rewrite pinned to the deleted model would fail on every
      // session — re-point it at Auto so dictation keeps working.
      if (preferences.value.dictation.model === model.name) {
        await preferences.update({
          dictation: { ...preferences.value.dictation, model: "" },
        });
      }
      await refresh(false);
    } finally {
      busy = null;
    }
  }

  /** Select a family tile and bring the freshly swapped catalog into view —
   * the bottom tiles otherwise leave you scrolled past the new content. */
  function selectFamily(id: string) {
    selectedFamilyId = id;
    catalogQuery = "";
    mainEl?.scrollTo({ top: 0, behavior: "smooth" });
  }

  /** Pick the shared AI models ROOT. Every engine stores in its own
   * subdirectory — `<root>/ollama` (OLLAMA_MODELS) and `<root>/speech`
   * (Whisper/Parakeet) — so one folder choice manages all model downloads.
   * Ollama's dir applies on the next managed start (saveConfig raises the
   * restart banner when one is up); the STT dir applies immediately. */
  async function chooseModelsDir() {
    if (!config) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({
      multiple: false,
      directory: true,
      title: "Choose the AI models folder",
      defaultPath: modelsRoot ?? overview?.modelsDisk.path,
    });
    if (typeof result !== "string") return;
    const root = result.replace(/\/$/, "");
    if (root === modelsRoot) return;
    // Roll back the local edit if persistence fails — otherwise the on-screen
    // config points at the new folder while the backend still has the old one
    // (and the STT dir update below is skipped), leaving the two stores out of
    // sync until the next reload.
    const prevConfig = config;
    config = { ...config, modelsDir: `${root}/ollama` };
    try {
      await saveConfig();
      await preferences.update({ stt: { modelsDir: `${root}/speech` } });
    } catch {
      config = prevConfig;
      return;
    }
    if (stt) void refreshStt(false);
  }

  // ---- Speech to text (local Whisper/Parakeet via the portbay-stt sidecar) ----

  async function refreshStt(showSpinner = true) {
    if (showSpinner) sttLoading = true;
    try {
      stt = await invokeQuiet<SttOverview>("stt_overview");
    } catch {
      // Quiet — runs unprompted at page mount for the sidebar list; the STT
      // family view renders its own "overview unavailable" copy off null.
    } finally {
      sttLoading = false;
    }
  }

  /** Stall watchdog (same contract as the Ollama pull stream's backend
   * watchdog): no progress event for this long → declare the download dead
   * instead of leaving the bar frozen forever. Generous because the
   * Neural-Engine compile phase reports sparsely. */
  const STT_STALL_MS = 180_000;
  let sttStallTimer: number | null = null;

  function clearSttStallTimer() {
    if (sttStallTimer !== null) {
      window.clearTimeout(sttStallTimer);
      sttStallTimer = null;
    }
  }

  function armSttStallTimer(id: string, model: string) {
    clearSttStallTimer();
    sttStallTimer = window.setTimeout(() => {
      if (sttDownloadId !== id) return;
      sttDownloadId = "";
      sttDownloadingModel = "";
      sttProgress = null;
      sttDownloadError = {
        model,
        detail: `No progress for ${STT_STALL_MS / 60_000} minutes — the download stalled. Retry to start it again.`,
      };
      void safeInvoke<void>("stt_cancel_download", { downloadId: id });
    }, STT_STALL_MS);
  }

  async function sttDownload(model: string) {
    if (sttDownloadingModel) return;
    const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    sttDownloadId = id;
    sttDownloadingModel = model;
    sttDownloadError = null;
    sttProgress = { fraction: 0, phase: "starting" };
    armSttStallTimer(id, model);
    const channel = new Channel<SttDownloadEvent>();
    channel.onmessage = (event) => {
      if (sttDownloadId !== id) return; // superseded/cancelled stream
      if (event.kind === "progress") {
        sttProgress = { fraction: event.fraction, phase: event.phase };
        armSttStallTimer(id, model);
      } else {
        clearSttStallTimer();
        if (!event.success && !event.cancelled) {
          sttDownloadError = { model, detail: event.error ?? "download failed" };
        }
        sttDownloadingModel = "";
        sttProgress = null;
        void refreshStt(false);
      }
    };
    try {
      // Quiet invoke: the card renders the terminal error state with Retry;
      // a toast on top would double-report it.
      await invokeQuiet<void>("stt_download_model", { model, downloadId: id, onEvent: channel });
    } catch (e) {
      if (sttDownloadId === id && sttDownloadingModel) {
        clearSttStallTimer();
        sttDownloadError = { model, detail: e instanceof Error ? e.message : String(e) };
        sttDownloadingModel = "";
        sttProgress = null;
      }
    }
  }

  async function sttCancelDownload() {
    if (!sttDownloadId) return;
    const id = sttDownloadId;
    // Land the card immediately; the backend's terminal event is ignored
    // once the id no longer matches (same pattern as cancelPull).
    clearSttStallTimer();
    sttDownloadId = "";
    sttDownloadingModel = "";
    sttProgress = null;
    await safeInvoke<void>("stt_cancel_download", { downloadId: id });
    void refreshStt(false);
  }

  async function sttDelete(modelId: string) {
    const entry = stt?.catalog.find((m) => m.id === modelId);
    const choice = await confirmDialog.open({
      title: `Delete ${entry?.displayName ?? modelId}?`,
      message: "Removes the downloaded model files from disk. You can download it again later.",
      destructive: true,
      actions: [{ label: "Delete model", value: "delete", tone: "destructive" }],
    });
    if (choice !== "delete") return;
    sttBusy = `delete:${modelId}`;
    try {
      await safeInvoke<void>("stt_delete_model", { model: modelId });
      if (preferences.value.dictation.sttModel === modelId) {
        // The dictation engine can't keep pointing at a deleted model —
        // fall back to macOS dictation until another model is chosen.
        await preferences.update({
          dictation: { ...preferences.value.dictation, sttModel: "", sttEngine: "macos" },
        });
      }
      await refreshStt(false);
    } finally {
      sttBusy = null;
    }
  }

  async function unloadModel(model: OllamaLoadedModel) {
    busy = `unload:${model.name}`;
    try {
      await safeInvoke<void>("ollama_unload_model", { model: model.name });
      await refresh(false);
    } finally {
      busy = null;
    }
  }

  /** Toggle the inline details panel under an installed model's row. */
  async function toggleDetails(model: string) {
    if (detailsName === model) {
      detailsName = "";
      detailsData = null;
      return;
    }
    detailsName = model;
    detailsData = null;
    detailsLoading = true;
    try {
      const value = await safeInvoke<Record<string, unknown>>("ollama_show_model", { model });
      if (detailsName === model) detailsData = value;
    } catch {
      // safeInvoke already toasted — close the empty panel.
      if (detailsName === model) detailsName = "";
    } finally {
      detailsLoading = false;
    }
  }

  /** Human-facing facts pulled out of the `/api/show` payload — the raw JSON
   * stays available behind the "Raw metadata" disclosure. */
  function detailFacts(data: Record<string, unknown>): { label: string; value: string }[] {
    const out: { label: string; value: string }[] = [];
    const push = (label: string, value: unknown) => {
      if (value === null || value === undefined || value === "") return;
      out.push({ label, value: String(value) });
    };
    const details = (data.details ?? {}) as Record<string, unknown>;
    const info = (data.model_info ?? {}) as Record<string, unknown>;
    // model_info keys are architecture-prefixed ("qwen2.context_length").
    const infoValue = (suffix: string): unknown => {
      const key = Object.keys(info).find((k) => k.endsWith(suffix));
      return key ? info[key] : undefined;
    };
    push("Family", details.family);
    push("Parameters", details.parameter_size);
    push("Quantization", details.quantization_level);
    push("Format", details.format);
    const ctx = infoValue(".context_length");
    if (typeof ctx === "number") push("Context length", ctx.toLocaleString());
    const embed = infoValue(".embedding_length");
    if (typeof embed === "number") push("Embedding length", embed.toLocaleString());
    if (Array.isArray(data.capabilities) && data.capabilities.length > 0) {
      push("Capabilities", data.capabilities.join(", "));
    }
    if (typeof data.license === "string" && data.license.trim()) {
      push("License", data.license.trim().split("\n")[0]);
    }
    return out;
  }

  function stopTestTimer() {
    if (testTimer !== null) {
      window.clearInterval(testTimer);
      testTimer = null;
    }
  }

  /** Stream a test prompt token-by-token: waiting → streaming → done|error,
   * with a live elapsed timer and end-of-run eval metrics (tokens/sec, time to
   * first token, total duration). Mirrors the pull command's channel contract —
   * every terminal path lands the card in done or error, never frozen. */
  async function runTestStream() {
    if (!selectedModel || testRunning) return;
    const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    testRunId = id;
    testPhase = "waiting";
    testOutput = "";
    testError = "";
    testMetrics = null;
    testTokenCount = 0;
    testThinking = "";
    testThinkingStartedAt = 0;
    testThinkingMs = 0;
    testStartedAt = Date.now();
    testFirstTokenAt = 0;
    testElapsedMs = 0;
    stopTestTimer();
    testTimer = window.setInterval(() => {
      if (testRunId === id && testRunning) testElapsedMs = Date.now() - testStartedAt;
    }, 100);
    const channel = new Channel<GenerateEvent>();
    channel.onmessage = (event) => {
      if (testRunId !== id) return; // superseded run
      // First fragment of any kind (reasoning or answer) ends the wait and
      // starts the latency clock.
      const markStreaming = () => {
        if (testPhase === "waiting") {
          testFirstTokenAt = Date.now();
          testPhase = "streaming";
        }
      };
      if (event.kind === "token") {
        markStreaming();
        // First answer token after reasoning → freeze the thinking duration.
        if (testThinkingStartedAt && !testThinkingMs) {
          testThinkingMs = Date.now() - testThinkingStartedAt;
        }
        testOutput += event.text;
        testTokenCount += 1;
      } else if (event.kind === "thinking") {
        markStreaming();
        if (!testThinkingStartedAt) testThinkingStartedAt = testFirstTokenAt;
        testThinking += event.text;
        testTokenCount += 1; // counts toward tok/s, same as Ollama's eval_count
      } else if (event.kind === "done") {
        testMetrics = event;
        testElapsedMs = Date.now() - testStartedAt;
        testPhase = "done";
        stopTestTimer();
      } else if (event.kind === "stopped") {
        testPhase = "stopped";
        stopTestTimer();
      } else {
        testError = event.message;
        testPhase = "error";
        stopTestTimer();
      }
    };
    try {
      const options = buildTestOptions();
      // Quiet invoke: the card renders its own inline error state; a toast on
      // top would double-report it.
      await invokeQuiet<void>("ollama_test_stream", {
        model: selectedModel,
        prompt: smokePrompt,
        runId: id,
        onEvent: channel,
        system: testSystem.trim() || null,
        think: selectedSupportsThinking && testThink,
        options: Object.keys(options).length > 0 ? options : null,
      });
    } catch (e) {
      if (testRunId === id && testRunning) {
        testError = e instanceof Error ? e.message : String(e);
        testPhase = "error";
        stopTestTimer();
      }
    }
  }

  /** Stop a running test (double-Esc). Asks the backend to drop the stream —
   * which ends Ollama's generation — and lands the card in "stopped" with
   * whatever streamed so far. The backend also emits a terminal Stopped event;
   * landing the phase here first keeps the UI responsive even if the model is
   * wedged and that event is slow to arrive. */
  function stopTest() {
    if (!testRunning) return;
    const id = testRunId;
    // Freeze the output immediately: superseding the id makes the handler
    // ignore any in-flight frames (and the backend's own Stopped echo).
    testRunId = "";
    testPhase = "stopped";
    testElapsedMs = Date.now() - testStartedAt;
    stopTestTimer();
    void invokeQuiet<void>("ollama_cancel_generate", { runId: id });
  }

  /** Double-press Esc while a test is generating to stop it — the escape
   * hatch for a wedged model. A single Esc is left alone (it still blurs the
   * textarea etc.); two within the window trigger the stop. */
  let lastEscAt = 0;
  function onTestKeydown(e: KeyboardEvent) {
    if (e.key !== "Escape" || activeView !== "test" || !testRunning) return;
    const now = Date.now();
    if (now - lastEscAt < 500) {
      lastEscAt = 0;
      stopTest();
    } else {
      lastEscAt = now;
    }
  }

  async function copyText(key: string, text: string) {
    if (!text) return;
    await navigator.clipboard?.writeText(text);
    copied = key;
    window.setTimeout(() => {
      if (copied === key) copied = null;
    }, 1400);
  }

  function updateConfig<K extends keyof AiPrefs>(key: K, value: AiPrefs[K]) {
    if (!config) return;
    config = { ...config, [key]: value };
  }

  function endpointUrl(): URL {
    try {
      return new URL(config?.endpoint || "http://127.0.0.1:11434");
    } catch {
      return new URL("http://127.0.0.1:11434");
    }
  }

  function endpointHost(): string {
    return endpointUrl().hostname;
  }

  function endpointPort(): string {
    return String(endpointUrl().port || "11434");
  }

  function updateEndpoint(part: "host" | "port", value: string) {
    if (!config) return;
    const url = endpointUrl();
    if (part === "host") {
      url.hostname = value.trim() || "127.0.0.1";
    } else {
      const port = value.trim();
      if (port && !/^\d{1,5}$/.test(port)) return;
      url.port = port || "11434";
    }
    updateConfig("endpoint", url.toString().replace(/\/$/, ""));
  }

  function numberValue(value: string): number | null {
    const trimmed = value.trim();
    if (!trimmed) return null;
    const n = Number(trimmed);
    return Number.isFinite(n) && n >= 0 ? n : null;
  }

  // Decimal units, deliberately: Finder, `df -H`, and `ollama list` all report
  // decimal GB, so binary division here made "Volume free" disagree with every
  // number the user can compare it against.
  function bytes(value: number | null | undefined): string {
    if (!value) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let n = value;
    let i = 0;
    while (n >= 1000 && i < units.length - 1) {
      n /= 1000;
      i += 1;
    }
    return `${i <= 1 ? Math.round(n) : n.toFixed(1)} ${units[i]}`;
  }

  function dateLabel(value: string | null): string {
    if (!value) return "Unknown";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
  }

  /** Compact duration for the test metrics: sub-second in ms, else seconds. */
  function formatMs(value: number | null | undefined): string {
    if (value === null || value === undefined) return "—";
    return value < 1000 ? `${Math.round(value)} ms` : `${(value / 1000).toFixed(value < 10_000 ? 2 : 1)} s`;
  }

  /** "expires in 4m" style countdown for a loaded model's keep-alive. */
  function expiresIn(value: string | null): string {
    if (!value) return "no expiry";
    const ms = new Date(value).getTime() - Date.now();
    if (Number.isNaN(ms)) return "no expiry";
    if (ms <= 0) return "expiring now";
    const mins = Math.round(ms / 60_000);
    if (mins < 60) return `expires in ${mins}m`;
    const hrs = Math.floor(mins / 60);
    return `expires in ${hrs}h ${mins % 60}m`;
  }

  function statusCopy(state: string | undefined): { label: string; tone: string } {
    switch (state) {
      case "running_managed":
        return { label: "Running", tone: "bg-status-running/15 text-status-running" };
      case "running_external":
        return { label: "Running externally", tone: "bg-accent/15 text-accent" };
      case "unreachable_managed":
      case "starting":
        return { label: "Starting", tone: "bg-status-warning/15 text-status-warning" };
      default:
        return { label: "Stopped", tone: "bg-surface-2 text-fg-muted" };
    }
  }

  function navClass(view: AiView): string {
    return `w-full flex items-center gap-2 px-2.5 py-1.5 rounded-md text-left text-[12.5px] transition-colors ${
      activeView === view ? "bg-surface-2 text-fg" : "text-fg-muted hover:bg-surface-2/60 hover:text-fg"
    }`;
  }

  function modelRowClass(active: boolean): string {
    return `w-full flex items-center justify-between gap-2 px-2.5 py-1.5 rounded-md text-left transition-colors ${
      active ? "bg-surface-2 text-fg" : "text-fg-muted hover:bg-surface-2/60 hover:text-fg"
    }`;
  }
</script>

<svelte:head>
  <title>AI — PortBay</title>
</svelte:head>

<!-- Double-Esc stops a running test prompt (escape hatch for a wedged model). -->
<svelte:window onkeydown={onTestKeydown} />

<!-- Inline details under an installed model's row (catalog and orphan lists
     share it): key facts up front, full /api/show JSON behind a disclosure. -->
{#snippet modelDetailsPanel(span = "lg:col-span-2")}
  <div class="{span} rounded-md border border-border bg-surface-2/40 px-3 py-2.5">
    {#if detailsLoading}
      <p class="text-[11px] text-fg-subtle">
        <Icon name="loader-circle" size={11} class="inline mr-1 animate-spin" /> Loading model details…
      </p>
    {:else if detailsData}
      {@const facts = detailFacts(detailsData)}
      {#if facts.length > 0}
        <dl class="grid gap-x-8 gap-y-1.5 sm:grid-cols-2">
          {#each facts as fact (fact.label)}
            <div class="flex items-baseline justify-between gap-3 text-[11.5px]">
              <dt class="shrink-0 text-fg-subtle">{fact.label}</dt>
              <dd class="min-w-0 truncate text-right font-mono text-fg-muted" title={fact.value}>{fact.value}</dd>
            </div>
          {/each}
        </dl>
      {/if}
      <details class="mt-2">
        <summary class="cursor-pointer text-[11px] text-fg-subtle hover:text-fg">Raw metadata</summary>
        <pre class="mt-1.5 max-h-64 overflow-auto rounded bg-bg p-2.5 text-[10.5px] leading-relaxed text-fg-muted">{JSON.stringify(detailsData, null, 2)}</pre>
      </details>
    {:else}
      <p class="text-[11px] text-fg-subtle">Details unavailable.</p>
    {/if}
  </div>
{/snippet}

<div class="h-full flex overflow-hidden max-[900px]:flex-col">
  <aside
    class="w-[300px] shrink-0 border-r border-border bg-surface/40
           overflow-y-auto flex flex-col max-[900px]:h-auto max-[900px]:max-h-[42vh] max-[900px]:w-full
           max-[900px]:border-r-0 max-[900px]:border-b"
    aria-label="AI sections"
  >
    <header class="sticky top-0 z-10 px-4 pt-4 pb-3 bg-surface/95 border-b border-border/40">
      <h2 class="text-[13px] font-semibold text-fg mb-2.5">AI</h2>
      <div class="relative">
        <Icon
          name="search"
          size={12}
          class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle pointer-events-none"
        />
        <input
          type="search"
          class="w-full pl-7 pr-2 h-8 rounded-md bg-surface/80 border border-border/60
                 text-[12px] text-fg placeholder:text-fg-subtle
                 focus:outline-none focus:ring-1 focus:ring-accent/60
                 focus:border-accent/40 transition-colors"
          placeholder="Search models..."
          bind:value={menuFilter}
          spellcheck="false"
          aria-label="Search AI models"
        />
      </div>
    </header>

    <div class="px-2 py-2 space-y-3 flex-1 min-h-0">
      <div class="space-y-1">
        <button
          type="button"
          onclick={() => (activeView = "home")}
          class="w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-left
                 transition-colors cursor-pointer
                 {activeView === 'home'
            ? 'bg-accent/10 ring-1 ring-inset ring-accent/40'
            : 'hover:bg-surface-2/60'}"
        >
          <span
            class="shrink-0 inline-flex items-center justify-center w-8 h-8 rounded-lg
                   bg-surface-2 overflow-hidden"
          >
            <!-- Fill the rounded container: inset, the PNG's own white square
                 shows sharp corners and looks off next to the model marks. -->
            <img src="/apps/ollama.png" alt="" class="h-full w-full object-cover" draggable="false" />
          </span>
          <span class="min-w-0 flex-1 leading-tight">
            <span class="flex items-center gap-1.5">
              <span class="text-[13px] font-semibold text-fg truncate">Ollama</span>
              {#if overview}
                {@const copy = statusCopy(overview.status.state)}
                <span class="rounded px-1.5 py-0.5 text-[10px] {copy.tone}">{copy.label}</span>
              {/if}
            </span>
            <span class="block text-[11px] font-mono text-fg-subtle truncate">
              {overview?.status.version ? `v${overview.status.version}` : overview?.status.endpoint ?? "Local server"}
            </span>
          </span>
        </button>
      </div>

      <nav class="space-y-0.5" aria-label="AI page navigation">
        <p class="px-2 py-1 text-[11px] uppercase tracking-wide text-fg-subtle flex items-center gap-1.5">
          AI sections <span class="font-mono">{AI_VIEWS.length}</span>
        </p>
        <button type="button" class={navClass("home")} onclick={() => (activeView = "home")}>
          <Icon name="power" size={13} /> Server home
        </button>
        <button type="button" class={navClass("models")} onclick={() => (activeView = "models")}>
          <Icon name="package" size={13} /> Models
        </button>
        <button type="button" class={navClass("test")} onclick={() => (activeView = "test")}>
          <Icon name="message-square" size={13} /> Test prompt
        </button>
        <button type="button" class={navClass("dictation")} onclick={() => (activeView = "dictation")}>
          <Icon name="audio-lines" size={13} /> Speech-to-Text
        </button>
        <button type="button" class={navClass("config")} onclick={() => (activeView = "config")}>
          <Icon name="sliders-horizontal" size={13} /> Configuration
        </button>
        <button type="button" class={navClass("logs")} onclick={() => (activeView = "logs")}>
          <Icon name="file-text" size={13} /> Logs
        </button>
      </nav>

      {#if overview}
        <div class="space-y-0.5">
          <p class="px-2 py-1 text-[11px] uppercase tracking-wide text-fg-subtle flex items-center gap-1.5">
            Installed models <span class="font-mono">{overview.installedModels.length + (stt?.installed.length ?? 0)}</span>
          </p>
          {#each visibleMenuModels as model}
            <button
              type="button"
              class={modelRowClass(selectedModel === model.name)}
              onclick={() => {
                selectedModel = model.name;
                activeView = "models";
              }}
            >
              <span class="min-w-0">
                <span class="block truncate font-mono text-[12px] text-fg">{model.name}</span>
                <span class="block truncate text-[10.5px] text-fg-subtle">{model.family ?? "model"} · {bytes(model.size)}</span>
              </span>
              <span class="rounded bg-status-running/15 px-1.5 py-0.5 text-[10px] text-status-running">MODEL</span>
            </button>
          {/each}
          <!-- Downloaded speech-to-text models live in the same list — they
               are installed models too, just managed by the STT sidecar
               instead of Ollama. Clicking opens their catalog family. -->
          {#each visibleMenuSttModels as model (model.id)}
            <button
              type="button"
              class={modelRowClass(false)}
              onclick={() => {
                selectedFamilyId = "stt";
                activeView = "models";
              }}
            >
              <span class="min-w-0">
                <span class="block truncate font-mono text-[12px] text-fg">{model.displayName}</span>
                <span class="block truncate text-[10.5px] text-fg-subtle">{model.engine} speech-to-text · {bytes(model.sizeBytes)}</span>
              </span>
              <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] text-accent">SPEECH</span>
            </button>
          {/each}
          {#if visibleMenuModels.length === 0 && visibleMenuSttModels.length === 0}
            <p class="px-2 py-1.5 text-[11px] text-fg-subtle">No matching models.</p>
          {/if}
        </div>
      {/if}

    </div>

    {#if overview}
      <div class="border-t border-border px-4 py-3 max-[900px]:hidden">
        <p class="text-[10px] font-semibold uppercase tracking-wide text-fg-subtle">Models volume</p>
        <p class="mt-1 truncate text-[11px] text-fg">{overview.modelsDisk.volume ?? overview.modelsDisk.path}</p>
        <p class="mt-0.5 text-[11px] text-fg-subtle">
          {bytes(overview.modelsDisk.usedBytes)} used · {bytes(overview.modelsDisk.availableBytes)} free
        </p>
      </div>
    {/if}
  </aside>

  <main class="flex-1 min-w-0 overflow-y-auto bg-bg">
    <header class="sticky top-0 z-20 border-b border-border bg-bg/95 px-8 py-5 backdrop-blur max-[900px]:px-4">
      <div class="flex flex-wrap items-start justify-between gap-4">
        <div class="flex items-start gap-3">
          <span class="mt-0.5 inline-flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-surface-2">
            {#if sttContext}
              <Icon name="audio-lines" size={18} class="text-fg-muted" />
            {:else}
              <img src="/apps/ollama.png" alt="" class="h-full w-full object-cover" draggable="false" />
            {/if}
          </span>
          <div>
            <div class="flex flex-wrap items-center gap-2">
              <h1 class="text-[20px] font-semibold text-fg">{activeTitle}</h1>
              <!-- The running/stopped pill tracks the Ollama server, so it only
                   belongs on Ollama views — not the on-device speech screens. -->
              {#if overview && !sttContext}
                {@const copy = statusCopy(overview.status.state)}
                <span class="rounded-md px-2 py-0.5 text-[10.5px] font-semibold {copy.tone}">
                  {copy.label}
                </span>
              {/if}
            </div>
            <p class="mt-1 text-[12px] text-fg-subtle">
              {activeSubtitle}
            </p>
          </div>
        </div>
        <!-- Start/Stop/Restart act on the Ollama server only — hidden on the
             Speech-to-Text views where they'd do nothing. -->
        {#if !sttContext}
        <div class="flex flex-wrap gap-2">
          <button
            type="button"
            class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent disabled:opacity-50"
            disabled={!canStart}
            title={overview && !overview.binary.detected
              ? "Ollama binary not found — download it on Server home, or set a custom binary path in Configuration."
              : undefined}
            onclick={() => runAction("ollama_start")}
          >
            <Icon name="play" size={13} class="inline mr-1" /> Start
          </button>
          <button type="button" class="rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2 disabled:opacity-50" disabled={!canStop} onclick={() => runAction("ollama_stop")}>
            <Icon name="square" size={13} class="inline mr-1" /> Stop
          </button>
          <button type="button" class="rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2 disabled:opacity-50" disabled={!canRestart} onclick={() => runAction("ollama_restart")}>
            <Icon name="rotate-cw" size={13} class="inline mr-1" /> Restart
          </button>
        </div>
        {/if}
      </div>
    </header>

  <div class="w-full px-8 py-6 space-y-6 max-[900px]:px-4">
    {#if loading && !overview}
      <div class="rounded-lg border border-border bg-surface p-6 text-[13px] text-fg-muted">Loading Ollama…</div>
    {:else if overview && config}
      {#if external}
        <div class="rounded-lg border border-accent/30 bg-accent/10 px-4 py-3 text-[12px] text-fg">
          Ollama is running outside PortBay. The controls above apply to it anyway: <strong>Stop</strong> shuts it down, and <strong>Start</strong> or <strong>Restart</strong> replace it with a PortBay-managed server using the saved configuration. Installed models stay on disk either way.
        </div>
      {/if}
      {#if restartNotice || (configDirty && managed)}
        <div class="rounded-lg border border-status-warning/30 bg-status-warning/10 px-4 py-3 text-[12px] text-fg">
          Configuration changes apply on the next managed start. Restart Ollama when current work can be interrupted.
        </div>
      {/if}
      {#if overview.status.portConflict}
        <div class="rounded-lg border border-status-unhealthy/30 bg-status-unhealthy/10 px-4 py-3 text-[12px] text-fg">
          Port conflict on the configured endpoint:
          <pre class="mt-2 max-h-36 overflow-auto whitespace-pre-wrap rounded bg-surface px-3 py-2 text-[11px] text-fg-muted">{overview.status.portConflict}</pre>
        </div>
      {/if}

      {#if activeView === "home"}
        <section class="space-y-4">
          <!-- Setup prompt — only shown while the server isn't running; once
               it's healthy the card disappears entirely. -->
          {#if !running}
          <article
            class="rounded-2xl px-5 py-4 border bg-status-unhealthy/5 border-status-unhealthy/30"
          >
            <header class="flex items-center justify-between gap-3 mb-3">
              <div class="flex items-center gap-2 min-w-0">
                <Icon name="circle-alert" size={15} class="text-status-unhealthy" />
                <h2 class="text-[13px] font-semibold text-fg">Local Ollama is not running</h2>
              </div>
              <button
                type="button"
                onclick={() => refresh()}
                disabled={loading || busy !== null}
                class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3 rounded-md border border-border bg-surface
                       text-[12px] text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors
                       disabled:opacity-50 disabled:cursor-not-allowed"
              >
                <Icon name="refresh-cw" size={11} class={loading ? "animate-spin" : ""} />
                Refresh
              </button>
            </header>
            <p class="text-[11.5px] text-fg-muted leading-relaxed mb-3">
              Start a PortBay-managed server after confirming the binary path and models directory. If another app already owns port 11434, the start diagnostic will show who is blocking it.
            </p>
            <div class="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
              <div class="flex items-center gap-2 text-[12px]">
                <Icon name={overview.binary.detected ? "circle-check" : "circle-stop"} size={13} class={overview.binary.detected ? "text-status-running" : "text-fg-subtle"} />
                <span class={overview.binary.detected ? "text-fg" : "text-fg-muted"}>Ollama binary detected</span>
              </div>
              <div class="flex items-center gap-2 text-[12px]">
                <Icon name="circle-stop" size={13} class="text-fg-subtle" />
                <span class="text-fg-muted">HTTP API reachable</span>
              </div>
              <div class="flex items-center gap-2 text-[12px]">
                <Icon name={external ? "circle-stop" : "circle-check"} size={13} class={external ? "text-fg-subtle" : "text-status-running"} />
                <span class={external ? "text-fg-muted" : "text-fg"}>{external ? "External server — Start takes over" : "Safe lifecycle ownership"}</span>
              </div>
              <div class="flex items-center gap-2 text-[12px]">
                <Icon name={overview.installedModels.length > 0 ? "circle-check" : "circle-stop"} size={13} class={overview.installedModels.length > 0 ? "text-status-running" : "text-fg-subtle"} />
                <span class={overview.installedModels.length > 0 ? "text-fg" : "text-fg-muted"}>{overview.installedModels.length} installed model{overview.installedModels.length === 1 ? "" : "s"}</span>
              </div>
            </div>
          </article>
          {/if}

          <div class="grid gap-4 xl:grid-cols-[1.05fr_0.95fr]">
            <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
              <header class="flex items-center gap-2 mb-3.5">
                <Icon name="package" size={13} class="text-fg-muted" />
                <h2 class="text-[13px] font-semibold text-fg">Storage and runtime</h2>
              </header>
              <dl class="space-y-3 text-[12px]">
                <div class="flex items-start justify-between gap-4">
                  <div class="min-w-0">
                    <dt class="text-fg">Models directory</dt>
                    <dd class="text-[10.5px] text-fg-subtle">Keep large weights off the boot disk.</dd>
                  </div>
                  <span class="shrink-0 max-w-[52%] truncate font-mono text-[11.5px] text-fg-muted">{overview.modelsDisk.path}</span>
                </div>
                <div class="flex items-start justify-between gap-4">
                  <div class="min-w-0">
                    <dt class="text-fg">Volume free</dt>
                    <dd class="text-[10.5px] text-fg-subtle">{overview.modelsDisk.volume ?? "Selected volume"}</dd>
                  </div>
                  <span class="shrink-0 font-mono text-[11.5px] text-fg-muted">{bytes(overview.modelsDisk.availableBytes)}</span>
                </div>
                <div class="flex items-start justify-between gap-4">
                  <div class="min-w-0">
                    <dt class="text-fg">Binary</dt>
                    <dd class="text-[10.5px] text-fg-subtle">PATH, common installs, or configured tarball path.</dd>
                  </div>
                  <span class="shrink-0 max-w-[52%] truncate font-mono text-[11.5px] text-fg-muted">{overview.binary.path ?? "Not detected"}</span>
                </div>
                <div class="flex items-start justify-between gap-4">
                  <div class="min-w-0">
                    <dt class="text-fg">Endpoint</dt>
                    <dd class="text-[10.5px] text-fg-subtle">Shared source of truth for local AI consumers.</dd>
                  </div>
                  <button type="button" class="shrink-0 font-mono text-[11.5px] text-accent hover:underline" onclick={() => copyText("endpoint", endpointSnippet)}>
                    {copied === "endpoint" ? "copied" : endpointSnippet}
                  </button>
                </div>
              </dl>
              {#if !overview.binary.detected}
                <div class="mt-4 rounded-lg border {installFailed ? 'border-status-unhealthy/40 bg-status-unhealthy/10' : 'border-border bg-surface-2/60'} p-3 text-[12px] text-fg-muted">
                  <p>Ollama is not installed yet. Download a PortBay-managed build (signed, kept under PortBay's runtimes folder like the language runtimes), or install it yourself and set a custom binary path.</p>
                  <div class="mt-2.5 flex flex-wrap items-center gap-2">
                    <button
                      type="button"
                      class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent disabled:opacity-50"
                      disabled={installing}
                      onclick={() => void installOllama()}
                    >
                      <Icon name={installing ? "loader-circle" : "download"} size={13} class="inline mr-1 {installing ? 'animate-spin' : ''}" />
                      {installing ? "Downloading…" : installFailed ? "Retry download" : "Download Ollama"}
                    </button>
                    <button class="text-[12px] text-accent hover:underline" type="button" onclick={() => openUrl("https://ollama.com/download")}>ollama.com/download</button>
                  </div>
                  {#if installing || installStatus}
                    <p class="mt-2 font-mono text-[11px] {installFailed ? 'text-status-unhealthy' : 'text-fg-subtle'}">{installStatus}</p>
                  {/if}
                </div>
              {:else if managedBinary}
                <!-- PortBay's own build → PortBay updates it. System installs
                     (brew / Ollama.app) update through their own channel. -->
                <div class="mt-4 rounded-lg border border-border bg-surface-2/40 p-3 text-[12px] text-fg-muted">
                  <div class="flex flex-wrap items-center justify-between gap-2">
                    <span class="text-[11.5px]">
                      PortBay-managed build
                      {#if updateCheck?.installedVersion}
                        · v{updateCheck.installedVersion}
                        {#if !updateCheck.updateAvailable && updateCheck.latestVersion}
                          <span class="text-status-running">— up to date</span>
                        {/if}
                      {/if}
                    </span>
                    <div class="flex flex-wrap gap-2">
                      {#if updateCheck?.updateAvailable}
                        <button
                          type="button"
                          class="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-on-accent disabled:opacity-50"
                          disabled={installing}
                          onclick={() => void installOllama()}
                        >
                          {installing ? "Updating…" : `Update to v${updateCheck.latestVersion}`}
                        </button>
                      {:else if updateCheck}
                        <button
                          type="button"
                          class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2 disabled:opacity-50"
                          disabled={installing}
                          title="Re-download the same version — repairs a broken install"
                          onclick={() => void installOllama()}
                        >
                          {installing ? "Reinstalling…" : "Reinstall"}
                        </button>
                      {/if}
                      <button
                        type="button"
                        class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2 disabled:opacity-50"
                        disabled={checkingUpdate || installing}
                        onclick={() => void checkBinaryUpdate()}
                      >
                        {checkingUpdate ? "Checking…" : "Check for updates"}
                      </button>
                    </div>
                  </div>
                  {#if installing || (installStatus && installFailed)}
                    <p class="mt-2 font-mono text-[11px] {installFailed ? 'text-status-unhealthy' : 'text-fg-subtle'}">{installStatus}</p>
                  {/if}
                </div>
              {/if}
            </article>

            <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
              <header class="flex items-center gap-2 mb-3.5">
                <Icon name="sparkles" size={13} class="text-fg-muted" />
                <h2 class="text-[13px] font-semibold text-fg">Next useful actions</h2>
              </header>
              <div class="grid gap-2">
                <button type="button" class="rounded-md border border-border px-3 py-2 text-left hover:bg-surface-2 disabled:opacity-50" disabled={!running} onclick={() => (activeView = "models")}>
                  <span class="block text-[12px] font-medium text-fg">Pull or inspect models</span>
                  <span class="mt-0.5 block text-[11px] text-fg-subtle">Start with qwen2.5:7b for dictation, coding prompts, and local chat.</span>
                </button>
                <button type="button" class="rounded-md border border-border px-3 py-2 text-left hover:bg-surface-2 disabled:opacity-50" disabled={!running || overview.installedModels.length === 0} onclick={() => (activeView = "test")}>
                  <span class="block text-[12px] font-medium text-fg">Run a smoke test</span>
                  <span class="mt-0.5 block text-[11px] text-fg-subtle">Verify the endpoint, selected model, and response path end to end.</span>
                </button>
                <button type="button" class="rounded-md border border-border px-3 py-2 text-left hover:bg-surface-2 disabled:opacity-50" onclick={() => (activeView = "config")}>
                  <span class="block text-[12px] font-medium text-fg">Tune managed server config</span>
                  <span class="mt-0.5 block text-[11px] text-fg-subtle">Move models to external storage, change keep-alive, origins, parallel loads, and logging.</span>
                </button>
              </div>
            </article>
          </div>

          <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
            <header class="flex items-center gap-2 mb-3.5">
              <Icon name="link" size={13} class="text-fg-muted" />
              <h2 class="text-[13px] font-semibold text-fg">Where this local server is used</h2>
            </header>
            <div class="grid gap-2 md:grid-cols-2">
              <button type="button" class="rounded-md border border-border px-3 py-2 text-left text-[12px] text-fg hover:bg-surface-2" onclick={() => (activeView = "dictation")}>
                <Icon name="mic" size={13} class="inline mr-2 text-accent" /> Speech-to-Text rewrites
              </button>
              <a class="rounded-md border border-border px-3 py-2 text-[12px] text-fg hover:bg-surface-2" href="/ssh">
                <Icon name="terminal" size={13} class="inline mr-2 text-accent" /> SSH agent local-model workflows
              </a>
            </div>
          </article>
        </section>
      {:else if activeView === "models"}
      <section id="models" class="grid scroll-mt-4 gap-4 2xl:grid-cols-[minmax(420px,0.9fr)_minmax(0,1.4fr)]">
        <aside class="rounded-lg border border-border bg-surface p-4">
          <div class="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h2 class="text-[14px] font-semibold text-fg">Model catalog</h2>
              {#if libraryError && !library}
                <p class="mt-1 text-[10.5px] text-fg-subtle">Live catalog unavailable — showing the bundled list.</p>
              {:else if library?.stale}
                <p class="mt-1 inline-flex items-center gap-1.5 text-[10.5px] text-amber-500">
                  <span class="w-1.5 h-1.5 rounded-full bg-amber-400"></span>
                  Couldn't refresh from ollama.com — showing a cached catalog that may be out of date.
                </p>
              {/if}
            </div>
            <span class="rounded bg-surface-2 px-2 py-1 text-[10.5px] text-fg-muted">
              {families.length} families
            </span>
          </div>

          <div class="mt-4 flex gap-2">
            <input
              class="min-w-0 flex-1 rounded-md border border-border bg-bg px-2.5 py-2 text-[12px] text-fg"
              bind:value={pullName}
              placeholder="Custom model, e.g. qwen3:8b"
              disabled={pulling || !running}
            />
            <button
              class="rounded-md bg-accent px-3 py-2 text-[12px] font-semibold text-on-accent disabled:opacity-50"
              disabled={pulling || !running || !pullName.trim()}
              onclick={() => pullModel()}
            >
              Download
            </button>
          </div>

          {#if pullPrompt}
            <div class="mt-3 rounded-md border border-accent/30 bg-accent/10 p-3 text-[11.5px] text-fg">
              <p><span class="font-mono">{pullPrompt}</span> is already installed.</p>
              <p class="mt-0.5 text-fg-muted">Checking for updates re-pulls from ollama.com and downloads only what changed — an up-to-date model finishes instantly.</p>
              <div class="mt-2 flex flex-wrap gap-2">
                <button class="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-on-accent disabled:opacity-50" type="button" disabled={!running || pulling} onclick={() => updateModel(pullPrompt!)}>Check for updates</button>
                <button class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2" type="button" onclick={dismissPull}>Dismiss</button>
              </div>
            </div>
          {/if}

          <!-- Pull state machine: active → (done | error | cancelled). Every
               backend failure path emits a terminal error event, so the card
               can't freeze mid-progress. Error/cancel keep Ollama's partial
               layers on disk; Resume re-pulls and continues from there. -->
          {#if pullEvent}
            <div
              class="mt-3 rounded-md p-3 {pullPhase === 'error'
                ? 'border border-status-unhealthy/40 bg-status-unhealthy/10'
                : pullPhase === 'done'
                  ? 'border border-status-running/30 bg-status-running/10'
                  : 'bg-surface-2'}"
            >
              <div class="flex items-center justify-between gap-3 text-[11px] {pullPhase === 'error' ? 'text-fg' : 'text-fg-muted'}">
                <span class="min-w-0 flex items-center gap-1.5">
                  {#if pullPhase === "error"}
                    <Icon name="circle-alert" size={12} class="shrink-0 text-status-unhealthy" />
                  {:else if pullPhase === "done"}
                    <Icon name="circle-check" size={12} class="shrink-0 text-status-running" />
                  {:else if pullPhase === "active"}
                    <Icon name="loader-circle" size={12} class="shrink-0 animate-spin" />
                  {/if}
                  <span class="min-w-0 truncate" title={pullEvent.error ?? pullEvent.status}>
                    {#if pullPhase === "cancelled"}
                      Cancelled — downloaded layers are kept on disk.
                    {:else if pullPhase === "done"}
                      {lastPullModel ? `${lastPullModel} is ready.` : "Download complete."}
                    {:else}
                      {pullEvent.error ?? pullStatusLabel(pullEvent.status)}
                    {/if}
                  </span>
                </span>
                {#if pullPhase === "active"}
                  <button class="shrink-0 text-status-unhealthy hover:underline" type="button" onclick={cancelPull}>Cancel</button>
                {/if}
              </div>
              {#if pullPhase === "active"}
                <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-bg">
                  <div class="h-full bg-accent transition-all" style={`width:${pullPct ?? 18}%`}></div>
                </div>
                {#if pullPct !== null}
                  <p class="mt-1 text-[10.5px] text-fg-subtle">{pullPct}% · {bytes(pullEvent.completed)} / {bytes(pullEvent.total)}</p>
                {/if}
              {:else if pullPhase === "error" || pullPhase === "cancelled"}
                <div class="mt-2 flex flex-wrap items-center gap-2">
                  <button
                    class="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-on-accent disabled:opacity-50"
                    type="button"
                    disabled={!running || pulling || !lastPullModel}
                    onclick={resumePull}
                  >
                    <Icon name="rotate-cw" size={11} class="inline mr-1" /> Resume download
                  </button>
                  <button class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2" type="button" onclick={dismissPull}>Dismiss</button>
                  {#if !running}
                    <span class="text-[10.5px] text-fg-subtle">Start the server to resume.</span>
                  {/if}
                </div>
              {:else}
                <div class="mt-2">
                  <button class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2" type="button" onclick={dismissPull}>Dismiss</button>
                </div>
              {/if}
            </div>
          {/if}

          <!-- ONE download location for every model kind: `<root>/ollama`
               (OLLAMA_MODELS — applies on the next managed start; the disk
               numbers reflect the RUNNING server's actual dir) and
               `<root>/speech` (Whisper/Parakeet — applies immediately). -->
          <div class="mt-3 rounded-md border border-border bg-surface-2/40 px-3 py-2.5">
            <div class="flex items-center justify-between gap-3">
              <div class="min-w-0">
                <p class="text-[10.5px] font-semibold uppercase tracking-wide text-fg-subtle">Download location — all models</p>
                {#if modelsRoot}
                  <p class="mt-0.5 truncate font-mono text-[11.5px] text-fg" title={modelsRoot}>{modelsRoot}</p>
                  <p class="mt-0.5 text-[10.5px] text-fg-subtle">ollama/ + speech/ inside · one folder for every model</p>
                {:else}
                  <p class="mt-0.5 truncate font-mono text-[11.5px] text-fg" title={overview.modelsDisk.path}>{overview.modelsDisk.path}</p>
                  <p class="mt-0.5 truncate font-mono text-[10.5px] text-fg-muted" title={preferences.value.stt.modelsDir}>speech: {preferences.value.stt.modelsDir}</p>
                {/if}
                <p class="mt-0.5 text-[10.5px] text-fg-subtle">
                  {bytes(overview.modelsDisk.availableBytes)} free on {overview.modelsDisk.volume ?? "this volume"}
                </p>
              </div>
              <button
                type="button"
                class="shrink-0 rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2 disabled:opacity-50"
                disabled={busy === "save"}
                onclick={() => void chooseModelsDir()}
              >
                Change…
              </button>
            </div>
            {#if external}
              <p class="mt-1.5 text-[10.5px] text-fg-subtle">
                Currently set by the external server — changes apply when Restart takes over with a managed server.
              </p>
            {/if}
          </div>

          <!-- Catalog-wide search: matches every family's variants plus the
               speech-to-text catalog; results render in the right pane. -->
          <div class="relative mt-4">
            <Icon
              name="search"
              size={12}
              class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle pointer-events-none"
            />
            <input
              type="search"
              class="w-full pl-7 pr-2 h-8 rounded-md bg-bg border border-border
                     text-[12px] text-fg placeholder:text-fg-subtle
                     focus:outline-none focus:ring-1 focus:ring-accent/60 focus:border-accent/40"
              placeholder="Search the whole catalog…"
              bind:value={catalogQuery}
              spellcheck="false"
              aria-label="Search all model families"
            />
          </div>

          <div class="mt-4 grid gap-2 sm:grid-cols-2 2xl:grid-cols-1">
            <!-- Speech to text leads the catalog (user request 2026-06-07):
                 it's the most-used local model here, and it lives with the
                 other model categories — not an Ollama model, but downloaded
                 and managed right here so everything model-shaped is in one
                 place. Vision and Embeddings follow (see orderedFamilies). -->
            <button
              type="button"
              class="rounded-lg border px-3 py-2.5 text-left transition-colors {selectedFamilyId === 'stt'
                ? 'border-accent/60 bg-accent/[0.08]'
                : 'border-border hover:border-border-strong hover:bg-surface-2'}"
              onclick={() => selectFamily("stt")}
            >
              <span class="flex items-center justify-between gap-2">
                <span class="flex min-w-0 items-center gap-2">
                  <span class="inline-flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded bg-surface-2">
                    <Icon name="audio-lines" size={12} class="text-fg-muted" />
                  </span>
                  <span class="truncate text-[13px] font-semibold {selectedFamilyId === 'stt' ? 'text-accent' : 'text-fg'}">Speech-to-Text</span>
                </span>
                <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-subtle">on-device</span>
              </span>
              <span class="mt-1 block text-[11px] text-fg-subtle">
                Whisper · Parakeet{stt ? ` · ${stt.catalog.length} options · ${stt.installed.length} installed` : ""}
              </span>
              <span class="mt-1 block text-[11px] leading-relaxed text-fg-muted">Transcription models for dictation — run on the Neural Engine.</span>
            </button>
            {#each orderedFamilies as family}
              {@const active = selectedFamilyId === family.id}
              {@const installedCount = family.variants.filter((model) => installedModelNames.has(model.name)).length}
              <button
                type="button"
                class="rounded-lg border px-3 py-2.5 text-left transition-colors {active
                  ? 'border-accent/60 bg-accent/[0.08]'
                  : 'border-border hover:border-border-strong hover:bg-surface-2'}"
                onclick={() => selectFamily(family.id)}
              >
                <span class="flex items-center justify-between gap-2">
                  <span class="flex min-w-0 items-center gap-2">
                    <ModelMark family={family.id} size={18} class="shrink-0" />
                    <span class="truncate text-[13px] font-semibold {active ? 'text-accent' : 'text-fg'}">{family.label}</span>
                  </span>
                  <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-subtle">{family.badge}</span>
                </span>
                <span class="mt-1 block text-[11px] text-fg-subtle">{family.vendor} · {family.variants.length} options · {installedCount} installed</span>
                <span class="mt-1 block text-[11px] leading-relaxed text-fg-muted">{family.summary}</span>
              </button>
            {/each}
          </div>
        </aside>

        <div class="space-y-4">
          {#if catalogQuery.trim()}
          <!-- Catalog-wide search results — across every family + STT. -->
          <div class="rounded-lg border border-border bg-surface">
            <div class="border-b border-border px-4 py-3">
              <div class="flex flex-wrap items-center justify-between gap-3">
                <div>
                  <h2 class="text-[14px] font-semibold text-fg">Search results</h2>
                  <p class="mt-0.5 text-[11px] text-fg-subtle">
                    {catalogMatches.length + sttMatches.length} match{catalogMatches.length + sttMatches.length === 1 ? "" : "es"} for "{catalogQuery.trim()}" across the whole catalog
                  </p>
                </div>
                <button
                  type="button"
                  class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2"
                  onclick={() => (catalogQuery = "")}
                >
                  Clear search
                </button>
              </div>
            </div>
            <div class="grid divide-y divide-border">
              {#each catalogMatches as match}
                {@const installed = installedModelNames.has(match.name)}
                {@const matchPulling = pulling && lastPullModel === match.name}
                <div class="grid gap-3 px-4 py-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <div class="min-w-0">
                    <span class="flex flex-wrap items-center gap-2">
                      <ModelMark family={match.familyId} size={16} class="shrink-0" />
                      <span class="font-mono text-[13px] font-semibold text-fg">{match.name}</span>
                      <button
                        type="button"
                        class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-subtle hover:text-accent"
                        title="Open this family"
                        onclick={() => selectFamily(match.familyId)}
                      >
                        {match.familyLabel}
                      </button>
                      {#if match.recommended}
                        <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">Recommended</span>
                      {/if}
                      {#if installed}
                        <span class="rounded bg-status-running/15 px-1.5 py-0.5 text-[10px] font-semibold text-status-running">Installed</span>
                      {/if}
                    </span>
                    <span class="mt-1 block text-[11px] text-fg-subtle">{match.workload} · {tagInfo[match.name]?.size ?? match.sizeHint}</span>
                    <span class="mt-1 block text-[11px] leading-relaxed text-fg-muted">{match.fit}</span>
                  </div>
                  <div class="flex flex-wrap gap-2 lg:justify-end">
                    {#if matchPulling}
                      <button class="rounded-md bg-accent/15 px-2.5 py-1.5 text-[11px] font-semibold text-accent" disabled>
                        <Icon name="loader-circle" size={11} class="inline mr-1 animate-spin" />
                        {pullPct !== null ? `Downloading ${pullPct}%` : "Downloading…"}
                      </button>
                    {:else}
                      <button
                        class="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-on-accent disabled:opacity-50"
                        disabled={!running || pulling || installed}
                        onclick={() => pullModel(match.name)}
                      >
                        {installed ? "Installed" : "Download"}
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
              {#each sttMatches as model}
                {@const installedEntry = stt?.installed.find((m) => m.id === model.id)}
                <div class="grid gap-3 px-4 py-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <div class="min-w-0">
                    <span class="flex flex-wrap items-center gap-2">
                      <span class="font-mono text-[13px] font-semibold text-fg">{model.displayName}</span>
                      <button
                        type="button"
                        class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-subtle hover:text-accent"
                        title="Open Speech-to-Text"
                        onclick={() => selectFamily("stt")}
                      >
                        Speech-to-Text
                      </button>
                      {#if installedEntry}
                        <span class="rounded bg-status-running/15 px-1.5 py-0.5 text-[10px] font-semibold text-status-running">Installed</span>
                      {/if}
                    </span>
                    <span class="mt-1 block text-[11px] text-fg-subtle">{model.languages} · ~{bytes(model.approxSizeBytes)} download</span>
                    <span class="mt-1 block text-[11px] leading-relaxed text-fg-muted">{model.speedNote}</span>
                  </div>
                  <div class="flex flex-wrap gap-2 lg:justify-end">
                    <button
                      class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2"
                      type="button"
                      onclick={() => selectFamily("stt")}
                    >
                      Open
                    </button>
                  </div>
                </div>
              {/each}
              {#if catalogMatches.length === 0 && sttMatches.length === 0}
                <div class="px-4 py-8 text-center text-[12px] text-fg-subtle">
                  Nothing in the catalog matches "{catalogQuery.trim()}". The custom field on the left pulls any ollama.com tag directly.
                </div>
              {/if}
            </div>
          </div>
          {:else if selectedFamilyId === "stt"}
          <!-- Speech to text — not Ollama models (portbay-stt sidecar:
               Whisper/Parakeet on the Neural Engine), but grouped with the
               other model categories so everything downloadable lives here. -->
          {#if !stt}
            <div class="rounded-lg border border-border bg-surface px-4 py-8 text-center text-[12px] text-fg-subtle">
              {sttLoading ? "Checking the speech-to-text engine…" : "Speech-to-text overview unavailable."}
            </div>
          {:else if !stt.status.available}
            <div class="rounded-lg border border-border bg-surface p-4">
              <h2 class="text-[14px] font-semibold text-fg">Local Speech-to-Text isn't available</h2>
              <p class="mt-1.5 text-[12px] leading-relaxed text-fg-muted">
                {stt.status.reason === "requires_macos_14"
                  ? "Local transcription needs macOS 14 or newer — dictation keeps using macOS Dictation on this Mac."
                  : stt.status.reason === "sidecar_missing"
                    ? "The bundled speech-to-text helper is missing — reinstall PortBay."
                    : stt.status.reason === "unsupported"
                      ? "Local speech-to-text is macOS-only."
                      : "The bundled speech-to-text helper didn't respond — reinstall PortBay."}
              </p>
            </div>
          {:else}
            <!-- Catalog: curated and static (no registry exists for STT models —
                 Ollama's library has none). One download at a time. -->
            <div class="rounded-lg border border-border bg-surface">
              <div class="border-b border-border px-4 py-3">
                <div class="flex flex-wrap items-center justify-between gap-3">
                  <div class="flex items-center gap-3">
                    <span class="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-surface-2">
                      <Icon name="audio-lines" size={18} class="text-fg-muted" />
                    </span>
                    <div>
                      <h2 class="text-[14px] font-semibold text-fg">Transcription models</h2>
                      <p class="mt-0.5 text-[11px] text-fg-subtle">
                        Run entirely on this Mac's Neural Engine — audio never leaves the machine.
                        {stt.installed.length} of {stt.catalog.length} installed.
                      </p>
                    </div>
                  </div>
                  <button
                    type="button"
                    onclick={() => void refreshStt()}
                    disabled={sttLoading}
                    class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3 rounded-md border border-border bg-surface
                           text-[12px] text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors
                           disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    <Icon name="refresh-cw" size={11} class={sttLoading ? "animate-spin" : ""} />
                    Refresh
                  </button>
                </div>
              </div>
              <div class="grid divide-y divide-border">
                {#each stt.catalog as model}
                  {@const installedEntry = stt.installed.find((m) => m.id === model.id)}
                  {@const downloading = sttDownloadingModel === model.id}
                  {@const failed = sttDownloadError?.model === model.id}
                  <div class="grid gap-3 px-4 py-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                    <div class="min-w-0">
                      <span class="flex flex-wrap items-center gap-2">
                        <span class="font-mono text-[13px] font-semibold text-fg">{model.displayName}</span>
                        <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-subtle">{model.engine === "parakeet" ? "Parakeet · NVIDIA" : "Whisper · OpenAI"}</span>
                        {#if model.recommended}
                          <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">Recommended</span>
                        {/if}
                        {#if installedEntry}
                          <span class="rounded bg-status-running/15 px-1.5 py-0.5 text-[10px] font-semibold text-status-running">Installed</span>
                        {/if}
                      </span>
                      <span class="mt-1 block text-[11px] text-fg-subtle">
                        {model.languages} · {installedEntry ? bytes(installedEntry.sizeBytes) : `~${bytes(model.approxSizeBytes)} download`}{model.streaming ? " · live partial text" : ""}
                      </span>
                      <span class="mt-1 block text-[11px] leading-relaxed text-fg-muted">{model.speedNote}</span>
                      {#if downloading && sttProgress}
                        <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-bg">
                          <div class="h-full bg-accent transition-all" style={`width:${Math.max(2, Math.round(sttProgress.fraction * 100))}%`}></div>
                        </div>
                        <p class="mt-1 text-[10.5px] text-fg-subtle">
                          {Math.round(sttProgress.fraction * 100)}% · {sttProgress.phase === "compiling" ? "Compiling for the Neural Engine…" : sttProgress.phase === "starting" ? "Contacting huggingface.co…" : "Downloading…"}
                        </p>
                      {:else if failed}
                        <p class="mt-1.5 text-[11px] text-status-unhealthy">{sttDownloadError?.detail}</p>
                      {/if}
                    </div>
                    <div class="flex flex-wrap gap-2 lg:justify-end">
                      {#if downloading}
                        <button
                          class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2"
                          type="button"
                          onclick={() => void sttCancelDownload()}
                        >
                          Cancel
                        </button>
                      {:else if installedEntry}
                        <button
                          class="rounded-md border border-status-unhealthy/40 px-2.5 py-1.5 text-[11px] text-status-unhealthy hover:bg-status-unhealthy/10 disabled:opacity-50"
                          type="button"
                          disabled={sttBusy === `delete:${model.id}`}
                          onclick={() => void sttDelete(model.id)}
                        >
                          Delete
                        </button>
                      {:else}
                        <button
                          class="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-on-accent disabled:opacity-50"
                          type="button"
                          disabled={sttDownloadingModel !== ""}
                          onclick={() => void sttDownload(model.id)}
                        >
                          {failed ? "Retry download" : "Download"}
                        </button>
                      {/if}
                    </div>
                  </div>
                {/each}
              </div>
            </div>

            <!-- Where these models are used. -->
            <div class="rounded-lg border border-border bg-surface px-4 py-3">
              <p class="text-[12px] text-fg-muted leading-relaxed">
                Installed models become selectable as the
                <button type="button" class="text-accent hover:underline" onclick={() => (activeView = "dictation")}>Speech-to-Text</button>
                transcription engine — replacing macOS Dictation with on-device Whisper or Parakeet while the rewrite layer stays unchanged.
              </p>
            </div>
          {/if}
          {:else}
          <div class="rounded-lg border border-border bg-surface">
            <div class="border-b border-border px-4 py-3">
              <div class="flex flex-wrap items-center justify-between gap-3">
                <div class="flex items-center gap-3">
                  <ModelMark family={selectedFamily.id} size={36} class="shrink-0" />
                  <div>
                    <h2 class="text-[14px] font-semibold text-fg">{selectedFamily.label} models</h2>
                    <p class="mt-0.5 text-[11px] text-fg-subtle">
                      {selectedFamily.vendor} · {installedCatalogCount} of {selectedFamily.variants.length} installed locally
                    </p>
                  </div>
                </div>
                <div class="flex items-center gap-2">
                  <input
                    class="w-36 rounded-md border border-border bg-bg px-2 py-1.5 text-[11px] text-fg placeholder:text-fg-subtle"
                    bind:value={variantFilter}
                    placeholder="Filter variants…"
                    spellcheck="false"
                    aria-label="Filter model variants"
                  />
                  <select
                    class="rounded-md border border-border bg-bg px-2 py-1.5 text-[11px] text-fg"
                    bind:value={variantSort}
                    aria-label="Sort model variants"
                  >
                    <option value="popular">Most popular</option>
                    <option value="updated">Recently updated</option>
                    <option value="size-asc">Smallest first</option>
                    <option value="size-desc">Largest first</option>
                  </select>
                  <button
                    type="button"
                    class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2 disabled:opacity-50"
                    disabled={!running || pulling}
                    onclick={() => {
                      const recommended = selectedFamily.variants.find((model) => model.recommended) ?? selectedFamily.variants[0];
                      pullName = recommended.name;
                    }}
                  >
                    Use recommended
                  </button>
                </div>
              </div>
            </div>
            <div class="grid divide-y divide-border">
              {#each visibleVariants as variant}
                {@const installed = installedModelNames.has(variant.name)}
                {@const tag = tagInfo[variant.name]}
                {@const rowPulling = pulling && lastPullModel === variant.name}
                <div class="grid gap-3 px-4 py-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <button
                    class="min-w-0 text-left"
                    type="button"
                    onclick={() => {
                      pullName = variant.name;
                      if (installed) selectedModel = variant.name;
                    }}
                  >
                    <span class="flex flex-wrap items-center gap-2">
                      <span class="font-mono text-[13px] font-semibold text-fg">{variant.name}</span>
                      {#if variant.recommended}
                        <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">Recommended</span>
                      {/if}
                      {#if installed}
                        <span class="rounded bg-status-running/15 px-1.5 py-0.5 text-[10px] font-semibold text-status-running">Installed</span>
                      {/if}
                    </span>
                    <span class="mt-1 block text-[11px] text-fg-subtle">
                      {variant.workload} · {tag?.size ?? variant.sizeHint}{tag?.context ? ` · ${tag.context} context` : ""}{variant.updated ? ` · updated ${variant.updated}` : ""}
                    </span>
                    <span class="mt-1 block text-[11px] leading-relaxed text-fg-muted">{variant.fit}</span>
                  </button>
                  <div class="flex flex-wrap gap-2 lg:justify-end">
                    {#if installed}
                      <!-- Installed rows manage the model in place — no
                           separate "installed" list to scroll for. -->
                      <button class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2" onclick={() => void toggleDetails(variant.name)}>
                        {detailsName === variant.name ? "Hide details" : "Details"}
                      </button>
                      {#if rowPulling}
                        <button class="rounded-md bg-accent/15 px-2.5 py-1.5 text-[11px] font-semibold text-accent" disabled>
                          <Icon name="loader-circle" size={11} class="inline mr-1 animate-spin" />
                          Updating…
                        </button>
                      {:else if hasUpdate(variant.name)}
                        <!-- Only rendered when ollama.com's manifest digest
                             differs from the local one — a real update. -->
                        <button
                          class="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-on-accent disabled:opacity-50"
                          disabled={pulling || !running}
                          title="A newer build is on ollama.com — downloads only the changed layers"
                          onclick={() => updateModel(variant.name)}
                        >
                          Update
                        </button>
                      {/if}
                      <button
                        class="rounded-md border border-status-unhealthy/40 px-2.5 py-1.5 text-[11px] text-status-unhealthy hover:bg-status-unhealthy/10 disabled:opacity-50"
                        disabled={busy === `delete:${variant.name}`}
                        onclick={() => {
                          const m = overview?.installedModels.find((im) => im.name === variant.name);
                          if (m) void deleteModel(m);
                        }}
                      >
                        Delete
                      </button>
                    {:else if rowPulling}
                      <button class="rounded-md bg-accent/15 px-2.5 py-1.5 text-[11px] font-semibold text-accent" disabled>
                        <Icon name="loader-circle" size={11} class="inline mr-1 animate-spin" />
                        {pullPct !== null ? `Downloading ${pullPct}%` : "Downloading…"}
                      </button>
                      <button class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2" type="button" onclick={cancelPull}>Cancel</button>
                    {:else}
                      <button
                        class="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-on-accent disabled:opacity-50"
                        disabled={!running || pulling}
                        onclick={() => pullModel(variant.name)}
                      >
                        Download
                      </button>
                      <button
                        class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2"
                        onclick={() => copyText(`pull-${variant.name}`, `ollama pull ${variant.name}`)}
                      >
                        {copied === `pull-${variant.name}` ? "Copied" : "Copy CLI"}
                      </button>
                    {/if}
                  </div>
                  {#if detailsName === variant.name}
                    {@render modelDetailsPanel()}
                  {/if}
                </div>
              {:else}
                <div class="px-4 py-8 text-center text-[12px] text-fg-subtle">
                  No variants match "{variantFilter}". The custom field on the left pulls any tag directly.
                </div>
              {/each}
            </div>
          </div>

          <!-- Catalog rows manage installed models in place; this card only
               exists for installs with no catalog row (custom tags, delisted
               models) — usually empty, so usually invisible. -->
          {#if orphanInstalled.length > 0}
            <div class="rounded-lg border border-border bg-surface">
              <div class="border-b border-border px-4 py-3">
                <h2 class="text-[14px] font-semibold text-fg">Other installed models</h2>
                <p class="mt-0.5 text-[11px] text-fg-subtle">Installed locally but not part of the catalog above.</p>
              </div>
              <div class="divide-y divide-border">
                {#each orphanInstalled as model}
                  <div class="grid gap-3 px-4 py-3 md:grid-cols-[1fr_auto] md:items-center">
                    <button class="min-w-0 text-left" type="button" onclick={() => (selectedModel = model.name)}>
                      <span class="font-mono text-[13px] font-semibold text-fg">{model.name}</span>
                      <span class="ml-2 text-[11px] text-fg-subtle">{bytes(model.size)} · {model.family ?? "family unknown"} · {model.parameterSize ?? "size unknown"} · {model.quantizationLevel ?? "quant unknown"}</span>
                      <span class="block text-[11px] text-fg-subtle">Modified {dateLabel(model.modifiedAt)}</span>
                    </button>
                    <div class="flex flex-wrap gap-2">
                      <button class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2" onclick={() => void toggleDetails(model.name)}>
                        {detailsName === model.name ? "Hide details" : "Details"}
                      </button>
                      {#if pulling && lastPullModel === model.name}
                        <button class="rounded-md bg-accent/15 px-2.5 py-1.5 text-[11px] font-semibold text-accent" disabled>
                          <Icon name="loader-circle" size={11} class="inline mr-1 animate-spin" />
                          Updating…
                        </button>
                      {:else if hasUpdate(model.name)}
                        <button
                          class="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-on-accent disabled:opacity-50"
                          disabled={pulling || !running}
                          title="A newer build is on ollama.com — downloads only the changed layers"
                          onclick={() => updateModel(model.name)}
                        >
                          Update
                        </button>
                      {/if}
                      <button class="rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2" onclick={() => copyText(`run-${model.name}`, `OLLAMA_HOST=${endpointSnippet.replace(/^https?:\/\//, "")} ollama run ${model.name}`)}>{copied === `run-${model.name}` ? "Copied" : "Copy run"}</button>
                      <button class="rounded-md border border-status-unhealthy/40 px-2.5 py-1.5 text-[11px] text-status-unhealthy hover:bg-status-unhealthy/10" disabled={busy === `delete:${model.name}`} onclick={() => deleteModel(model)}>Delete</button>
                    </div>
                    {#if detailsName === model.name}
                      {@render modelDetailsPanel("md:col-span-2")}
                    {/if}
                  </div>
                {/each}
              </div>
            </div>
          {/if}
          {/if}
        </div>
      </section>
      {:else if activeView === "test"}
      <!-- minmax(0,…) on both tracks is load-bearing: without it the long curl
           <pre> blocks on the right force their track wider than the viewport,
           spilling the page into horizontal scroll and starving this hero
           column. The prompt+response is the important part, so it takes the
           larger share. -->
      <section id="test" class="grid scroll-mt-4 gap-4 xl:grid-cols-[minmax(0,1.7fr)_minmax(0,1fr)]">
        <!-- Live run: prompt in, tokens streamed out, with latency + tokens/sec
             so the test reads like a real-world generation, not a black box. -->
        <div class="min-w-0 rounded-lg border border-border bg-surface p-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <Icon name="message-square" size={14} class="text-fg-muted" />
              <h2 class="text-[14px] font-semibold text-fg">Test prompt</h2>
            </div>
            <label class="flex items-center gap-2 text-[11px] text-fg-subtle">
              Model
              <select class="rounded-md border border-border bg-bg px-2 py-1.5 text-[12px] text-fg" bind:value={selectedModel}>
                {#if overview.installedModels.length === 0}
                  <option value="">No models installed</option>
                {/if}
                {#each overview.installedModels as model}
                  <option value={model.name}>{model.name}</option>
                {/each}
              </select>
            </label>
          </div>

          <textarea
            class="mt-3 h-24 w-full resize-y rounded-md border border-border bg-bg px-3 py-2 text-[12px] text-fg focus:outline-none focus:border-accent/60"
            placeholder="Ask the model anything — the response streams in below."
            bind:value={smokePrompt}
            onkeydown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === "Enter") void runTestStream(); }}
          ></textarea>

          <!-- Power-user knobs: a system prompt, the thinking toggle (reasoning
               models only), and Ollama sampling options. Collapsed by default so
               the common case stays a one-field prompt. Blank knobs keep the
               model's own defaults. -->
          <details class="mt-3 rounded-md border border-border bg-bg" bind:open={testOptionsOpen}>
            <summary class="flex cursor-pointer list-none items-center justify-between gap-2 px-3 py-2 text-[12px] text-fg select-none">
              <span class="inline-flex items-center gap-1.5">
                <Icon name="sliders-horizontal" size={13} class="text-fg-muted" />
                Options
              </span>
              <Icon name="chevron-down" size={14} class="text-fg-subtle transition-transform {testOptionsOpen ? 'rotate-180' : ''}" />
            </summary>
            <div class="space-y-3 border-t border-border/70 px-3 py-3">
              <label class="block">
                <span class="text-[11px] font-medium text-fg-muted">System prompt</span>
                <textarea
                  class="mt-1 h-16 w-full resize-y rounded-md border border-border bg-surface px-2.5 py-1.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
                  placeholder="Optional — sets the model's role/behaviour (sent as `system`)."
                  bind:value={testSystem}
                ></textarea>
              </label>

              {#if selectedSupportsThinking}
                <label class="flex items-center justify-between gap-3">
                  <span class="min-w-0">
                    <span class="text-[12px] text-fg">Thinking</span>
                    <span class="block text-[11px] text-fg-subtle">Stream the model's reasoning separately from the answer.</span>
                  </span>
                  <Toggle checked={testThink} label="Request thinking" onchange={(v) => (testThink = v)} />
                </label>
              {/if}

              <div class="grid grid-cols-2 gap-2 sm:grid-cols-4">
                {#each [
                  { label: "Temperature", value: () => testTemperature, set: (v: string) => (testTemperature = v), step: "0.1", placeholder: "0.8" },
                  { label: "Top P", value: () => testTopP, set: (v: string) => (testTopP = v), step: "0.05", placeholder: "0.9" },
                  { label: "Top K", value: () => testTopK, set: (v: string) => (testTopK = v), step: "1", placeholder: "40" },
                  { label: "Repeat penalty", value: () => testRepeatPenalty, set: (v: string) => (testRepeatPenalty = v), step: "0.05", placeholder: "1.1" },
                  { label: "Seed", value: () => testSeed, set: (v: string) => (testSeed = v), step: "1", placeholder: "random" },
                  { label: "Max tokens", value: () => testNumPredict, set: (v: string) => (testNumPredict = v), step: "1", placeholder: "∞" },
                  { label: "Context (num_ctx)", value: () => testNumCtx, set: (v: string) => (testNumCtx = v), step: "1", placeholder: "model default" },
                ] as knob (knob.label)}
                  <label class="block">
                    <span class="text-[10.5px] uppercase tracking-wide text-fg-subtle">{knob.label}</span>
                    <input
                      type="number"
                      step={knob.step}
                      value={knob.value()}
                      placeholder={knob.placeholder}
                      oninput={(e) => knob.set(e.currentTarget.value)}
                      class="mt-1 w-full rounded-md border border-border bg-surface px-2 py-1.5 font-mono text-[12px] text-fg focus:outline-none focus:border-accent/60"
                    />
                  </label>
                {/each}
              </div>
              <p class="text-[10.5px] text-fg-subtle">Leave a field blank to use the model's default.</p>
            </div>
          </details>

          <div class="mt-3 flex flex-wrap items-center gap-2">
            {#if testRunning}
              <button
                class="inline-flex items-center gap-1.5 rounded-md border border-status-unhealthy/50 px-3 py-1.5 text-[12px] font-semibold text-status-unhealthy hover:bg-status-unhealthy/10"
                onclick={stopTest}
              >
                <Icon name="square" size={13} /> Stop
              </button>
            {:else}
              <button
                class="inline-flex items-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent disabled:opacity-50"
                disabled={!selectedModel || !running}
                onclick={() => void runTestStream()}
              >
                <Icon name="play" size={13} /> Run test
              </button>
            {/if}
            {#if (testPhase === "done" || testPhase === "error" || testPhase === "stopped") && !testRunning}
              <button
                class="inline-flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2"
                onclick={() => void runTestStream()}
              >
                <Icon name="rotate-cw" size={12} /> Run again
              </button>
            {/if}
            <span class="text-[10.5px] text-fg-subtle">{testRunning ? "double-press Esc to stop" : "⌘↵ to run"}</span>
            {#if !running}
              <span class="text-[10.5px] text-status-warning">Start the server to run a prompt.</span>
            {/if}
          </div>

          <!-- Output + state machine. The bar above the response shows live
               latency while waiting/streaming and the final metrics on done. -->
          <div class="mt-3 rounded-md border border-border bg-bg">
            <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/70 px-3 py-2 text-[11px]">
              <span class="inline-flex items-center gap-1.5 font-medium">
                {#if testPhase === "waiting"}
                  <Icon name="loader-circle" size={12} class="animate-spin text-accent" />
                  <span class="text-fg">Waiting for first token…</span>
                {:else if testPhase === "streaming"}
                  <span class="h-2 w-2 rounded-full bg-status-running animate-pulse"></span>
                  <span class="text-fg">Streaming response</span>
                {:else if testPhase === "done"}
                  <Icon name="circle-check" size={12} class="text-status-running" />
                  <span class="text-fg">Done</span>
                {:else if testPhase === "error"}
                  <Icon name="circle-alert" size={12} class="text-status-unhealthy" />
                  <span class="text-fg">Failed</span>
                {:else if testPhase === "stopped"}
                  <Icon name="square" size={12} class="text-fg-muted" />
                  <span class="text-fg">Stopped</span>
                {:else}
                  <Icon name="sparkles" size={12} class="text-fg-subtle" />
                  <span class="text-fg-subtle">Response</span>
                {/if}
              </span>
              <div class="flex items-center gap-3 font-mono text-[10.5px] text-fg-subtle">
                {#if testRunning}
                  <span>{(testElapsedMs / 1000).toFixed(1)}s</span>
                  {#if testLiveTokensPerSec !== null}
                    <span class="text-fg-muted">{testLiveTokensPerSec.toFixed(1)} tok/s</span>
                  {/if}
                  {#if testTokenCount > 0}
                    <span>{testTokenCount} tok</span>
                  {/if}
                {:else if testPhase === "done"}
                  {#if testTokensPerSec !== null}
                    <span class="text-fg-muted">{testTokensPerSec.toFixed(1)} tok/s</span>
                  {/if}
                  {#if testMetrics?.evalCount}
                    <span>{testMetrics.evalCount} tokens</span>
                  {/if}
                  <span>{formatMs(testMetrics?.totalDurationMs ?? testElapsedMs)}</span>
                {:else if testPhase === "stopped"}
                  <span>stopped at {(testElapsedMs / 1000).toFixed(1)}s</span>
                {/if}
                {#if (testPhase === "done" || testPhase === "streaming" || testPhase === "stopped") && testOutput}
                  <button class="text-accent hover:underline" onclick={() => copyText("test-output", testOutput)}>
                    {copied === "test-output" ? "Copied" : "Copy"}
                  </button>
                {/if}
              </div>
            </div>

            <!-- Reasoning trace (think: true) — kept above and visually apart
                 from the answer so the chain-of-thought doesn't read as output. -->
            {#if testThinking}
              <details class="border-b border-border/70" open>
                <summary class="flex cursor-pointer list-none items-center gap-1.5 px-3 py-2 text-[11px] font-medium text-fg-muted select-none">
                  <Icon name="lightbulb" size={12} class="text-fg-subtle" />
                  Thinking
                  {#if testThinkingMs}
                    <span class="font-mono text-[10.5px] text-fg-subtle">· {formatMs(testThinkingMs)}</span>
                  {:else if testRunning && !testOutput}
                    <Icon name="loader-circle" size={11} class="animate-spin text-fg-subtle" />
                  {/if}
                </summary>
                <pre class="max-h-[200px] overflow-auto whitespace-pre-wrap bg-surface-2/30 px-4 py-2.5 text-[12px] leading-relaxed text-fg-muted">{testThinking}</pre>
              </details>
            {/if}

            {#if testPhase === "error"}
              <p class="min-h-[200px] px-3 py-3 text-[12px] text-status-unhealthy">{testError}</p>
            {:else if testOutput}
              <pre bind:this={testOutputEl} class="min-h-[200px] max-h-[440px] overflow-auto whitespace-pre-wrap px-4 py-3 text-[13px] leading-relaxed text-fg">{testOutput}{#if testPhase === "streaming"}<span class="inline-block h-3.5 w-[2px] translate-y-0.5 animate-pulse bg-accent"></span>{/if}</pre>
            {:else if testPhase === "waiting"}
              <p class="flex min-h-[200px] items-center justify-center px-3 text-[12px] text-fg-subtle">Loading the model and starting generation…</p>
            {:else if testPhase === "stopped"}
              <p class="flex min-h-[200px] items-center justify-center px-6 text-center text-[12px] text-fg-subtle">Stopped before any output. Run again to retry.</p>
            {:else if testPhase === "streaming"}
              <p class="flex min-h-[200px] items-center justify-center px-6 text-center text-[12px] text-fg-subtle">{testThinking ? "Reasoning… the answer will appear here." : "Generating…"}</p>
            {:else}
              <p class="flex min-h-[200px] items-center justify-center px-6 text-center text-[12px] text-fg-subtle">Run a prompt to watch the response stream in, with latency and tokens/sec.</p>
            {/if}
          </div>

          <!-- Metrics breakdown — live estimates as tokens arrive, then the
               exact eval counters once the `done` frame lands. -->
          {#if testPhase === "streaming" || testPhase === "done" || testPhase === "stopped"}
            {@const live = testPhase !== "done"}
            <dl class="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
              <div class="rounded-md border border-border bg-surface-2/40 px-3 py-2">
                <dt class="text-[10px] uppercase tracking-wide text-fg-subtle">
                  Tokens/sec{#if live}<span class="text-fg-subtle/70"> · live</span>{/if}
                </dt>
                <dd class="mt-0.5 font-mono text-[13px] text-fg">{testDisplayTokensPerSec !== null ? testDisplayTokensPerSec.toFixed(1) : "—"}</dd>
              </div>
              <div class="rounded-md border border-border bg-surface-2/40 px-3 py-2">
                <dt class="text-[10px] uppercase tracking-wide text-fg-subtle">Prefill tok/s</dt>
                <dd class="mt-0.5 font-mono text-[13px] text-fg">{testPrefillTokensPerSec !== null ? testPrefillTokensPerSec.toFixed(1) : "—"}</dd>
              </div>
              <div class="rounded-md border border-border bg-surface-2/40 px-3 py-2">
                <dt class="text-[10px] uppercase tracking-wide text-fg-subtle">First token</dt>
                <dd class="mt-0.5 font-mono text-[13px] text-fg">{formatMs(testTtftMs)}</dd>
              </div>
              <div class="rounded-md border border-border bg-surface-2/40 px-3 py-2">
                <dt class="text-[10px] uppercase tracking-wide text-fg-subtle">Thinking</dt>
                <dd class="mt-0.5 font-mono text-[13px] text-fg">{testThinkingMs ? formatMs(testThinkingMs) : "—"}</dd>
              </div>
              <div class="rounded-md border border-border bg-surface-2/40 px-3 py-2">
                <dt class="text-[10px] uppercase tracking-wide text-fg-subtle">{live ? "Elapsed" : "Total"}</dt>
                <dd class="mt-0.5 font-mono text-[13px] text-fg">{formatMs(testMetrics?.totalDurationMs ?? testElapsedMs)}</dd>
              </div>
              <div class="rounded-md border border-border bg-surface-2/40 px-3 py-2">
                <dt class="text-[10px] uppercase tracking-wide text-fg-subtle">Eval / prompt</dt>
                <dd class="mt-0.5 font-mono text-[13px] text-fg">{testMetrics?.evalCount ?? testTokenCount} / {testMetrics?.promptEvalCount ?? "—"}</dd>
              </div>
            </dl>
          {/if}
        </div>

        <div class="min-w-0 space-y-4">
          <!-- Call it yourself — the same request, runnable from a terminal or
               any HTTP client. Streaming variant included ("copy run stream"). -->
          <div class="rounded-lg border border-border bg-surface p-4">
            <div class="flex items-center gap-2">
              <Icon name="terminal" size={14} class="text-fg-muted" />
              <h2 class="text-[14px] font-semibold text-fg">Call it yourself</h2>
            </div>
            <p class="mt-1 text-[11px] text-fg-subtle">The exact request this page runs — copy it into a terminal to verify the model end-to-end.</p>
            <div class="mt-3 space-y-2.5">
              {#each [
                { key: "endpoint", label: "Endpoint", value: endpointSnippet },
                { key: "curl-stream", label: "Streaming request (watch tokens arrive)", value: curlStreamSnippet },
                { key: "curl", label: "Single response (JSON)", value: curlSnippet },
                { key: "run", label: "Interactive CLI", value: runSnippet },
              ] as snip (snip.key)}
                {#if snip.value}
                  <div>
                    <div class="mb-1 flex items-center justify-between">
                      <span class="text-[10.5px] font-medium uppercase tracking-wide text-fg-subtle">{snip.label}</span>
                      <button
                        class="inline-flex items-center gap-1 text-[11px] text-accent hover:underline"
                        onclick={() => copyText(snip.key, snip.value)}
                      >
                        <Icon name={copied === snip.key ? "check" : "copy"} size={11} />
                        {copied === snip.key ? "Copied" : "Copy"}
                      </button>
                    </div>
                    <pre class="overflow-auto rounded-md bg-bg px-3 py-2 font-mono text-[11px] leading-relaxed text-fg-muted">{snip.value}</pre>
                  </div>
                {/if}
              {/each}
              {#if !selectedModel}
                <p class="text-[11px] text-fg-subtle">Select a model to generate the request snippets.</p>
              {/if}
            </div>
          </div>

          <!-- Loaded models — what's resident in memory right now, with VRAM,
               placement, and keep-alive countdown. The model under test is
               highlighted so you can see it warm up after the first run. -->
          <div class="rounded-lg border border-border bg-surface p-4">
            <div class="flex items-center justify-between gap-2">
              <div class="flex items-center gap-2">
                <Icon name="cpu" size={14} class="text-fg-muted" />
                <h2 class="text-[14px] font-semibold text-fg">Loaded in memory</h2>
              </div>
              <span class="rounded bg-surface-2 px-2 py-0.5 text-[10.5px] text-fg-muted">{overview.loadedModels.length}</span>
            </div>
            <div class="mt-3 space-y-2">
              {#each overview.loadedModels as model}
                {@const active = model.name === selectedModel}
                <div class="rounded-md border px-3 py-2.5 {active ? 'border-accent/50 bg-accent/[0.05]' : 'border-border'}">
                  <div class="flex items-center justify-between gap-3">
                    <p class="min-w-0 truncate font-mono text-[12px] text-fg">{model.name}</p>
                    <button class="shrink-0 rounded-md border border-border px-2.5 py-1 text-[11px] text-fg hover:bg-surface-2 disabled:opacity-50" disabled={busy === `unload:${model.name}`} onclick={() => unloadModel(model)}>Unload</button>
                  </div>
                  <div class="mt-1.5 flex flex-wrap items-center gap-1.5">
                    <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-muted">{bytes(model.sizeVram)} VRAM</span>
                    {#if model.processor}
                      <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-muted">{model.processor}</span>
                    {/if}
                    <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-subtle">{expiresIn(model.expiresAt)}</span>
                    {#if active}
                      <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">Under test</span>
                    {/if}
                  </div>
                </div>
              {:else}
                <p class="rounded-md border border-dashed border-border px-3 py-6 text-center text-[12px] text-fg-subtle">
                  Nothing loaded yet. Running a test loads the selected model — it stays warm for the keep-alive window.
                </p>
              {/each}
            </div>
          </div>
        </div>
      </section>
      {:else if activeView === "dictation"}
      <section id="dictation" class="w-full">
        <SmartDictationPanel
          onManageSpeechModels={() => {
            activeView = "models";
            selectedFamilyId = "stt";
          }}
        />
      </section>
      {:else if activeView === "config"}
      <section id="config" class="w-full">
        <div class="rounded-lg border border-border bg-surface p-4">
          <div class="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h2 class="text-[14px] font-semibold text-fg">Configuration</h2>
              <p class="mt-1 text-[11px] text-fg-subtle">
                Saved as PortBay-managed Ollama environment settings and applied on the next managed start.
              </p>
            </div>
            <div class="flex flex-wrap gap-2">
              <button class="rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2 disabled:opacity-50" disabled={busy !== null} onclick={() => refresh()}>
                <Icon name="refresh-cw" size={12} class="inline mr-1" /> Refresh model list
              </button>
              <button class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent disabled:opacity-50" disabled={!configDirty || busy === "save"} onclick={saveConfig}>Save changes</button>
            </div>
          </div>
          {#if external}
            <div class="mt-4 rounded-md border border-accent/30 bg-accent/10 px-3 py-2 text-[12px] text-fg">
              An external server currently answers at this endpoint. These settings apply when PortBay starts its managed server — Restart takes over and applies them.
            </div>
          {/if}

          <div class="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Model download threads</span>
              <input class="field" value={config.modelDownloadThreads ?? ""} inputmode="numeric" placeholder="1" oninput={(e) => updateConfig("modelDownloadThreads", numberValue(e.currentTarget.value))} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Bind IP</span>
              <input class="field" value={endpointHost()} placeholder="127.0.0.1" oninput={(e) => updateEndpoint("host", e.currentTarget.value)} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Bind port</span>
              <input class="field" value={endpointPort()} inputmode="numeric" placeholder="11434" oninput={(e) => updateEndpoint("port", e.currentTarget.value)} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Keep alive</span>
              <input class="field" value={config.keepAlive} placeholder="5m" oninput={(e) => updateConfig("keepAlive", e.currentTarget.value)} />
            </label>
          </div>

          <div class="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">KV cache type</span>
              <input class="field" value={config.kvCacheType} placeholder="f16" oninput={(e) => updateConfig("kvCacheType", e.currentTarget.value)} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">GPU overhead</span>
              <input class="field" value={config.gpuOverhead ?? ""} inputmode="numeric" placeholder="0" oninput={(e) => updateConfig("gpuOverhead", numberValue(e.currentTarget.value))} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Load timeout</span>
              <input class="field" value={config.loadTimeout} placeholder="5m" oninput={(e) => updateConfig("loadTimeout", e.currentTarget.value)} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Parallel num.</span>
              <input class="field" value={config.numParallel ?? ""} inputmode="numeric" placeholder="0" oninput={(e) => updateConfig("numParallel", numberValue(e.currentTarget.value))} />
            </label>
          </div>

          <div class="mt-4 grid gap-3 md:grid-cols-3">
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Max loaded models</span>
              <input class="field" value={config.maxLoadedModels ?? ""} inputmode="numeric" placeholder="0" oninput={(e) => updateConfig("maxLoadedModels", numberValue(e.currentTarget.value))} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Max queue</span>
              <input class="field" value={config.maxQueue ?? ""} inputmode="numeric" placeholder="512" oninput={(e) => updateConfig("maxQueue", numberValue(e.currentTarget.value))} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">LLM library</span>
              <input class="field" value={config.llmLibrary} placeholder="rocm_v6 cpu cpu_avx cpu_avx2 cuda_v11 rocm_v5" oninput={(e) => updateConfig("llmLibrary", e.currentTarget.value)} />
            </label>
          </div>

          <div class="mt-4 grid gap-3 md:grid-cols-[1fr_auto]">
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Models folder</span>
              <input class="field" value={config.modelsDir} oninput={(e) => updateConfig("modelsDir", e.currentTarget.value)} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">Binary path</span>
              <input class="field" value={config.binaryPath} placeholder="Auto-detect" oninput={(e) => updateConfig("binaryPath", e.currentTarget.value)} />
            </label>
          </div>

          <label class="mt-4 block">
            <span class="mb-1 block text-[11px] font-medium text-fg-muted">Origins</span>
            <input class="field" value={config.origins} oninput={(e) => updateConfig("origins", e.currentTarget.value)} />
          </label>

          <div class="mt-4 grid gap-3 md:grid-cols-3">
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">HTTP proxy</span>
              <input class="field" value={config.httpProxy} placeholder="http://proxy.servbay.host:8080" oninput={(e) => updateConfig("httpProxy", e.currentTarget.value)} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">HTTPS proxy</span>
              <input class="field" value={config.httpsProxy} placeholder="http://proxy.servbay.host:8080" oninput={(e) => updateConfig("httpsProxy", e.currentTarget.value)} />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] font-medium text-fg-muted">No proxy</span>
              <input class="field" value={config.noProxy} placeholder="localhost,127.0.0.1" oninput={(e) => updateConfig("noProxy", e.currentTarget.value)} />
            </label>
          </div>

          <div class="mt-4 flex flex-wrap gap-3">
            <label class="toggle"><input type="checkbox" checked={config.debug} onchange={(e) => updateConfig("debug", e.currentTarget.checked)} /> Debug</label>
            <label class="toggle"><input type="checkbox" checked={config.flashAttention} onchange={(e) => updateConfig("flashAttention", e.currentTarget.checked)} /> Flash attention</label>
            <label class="toggle"><input type="checkbox" checked={config.noHistory} onchange={(e) => updateConfig("noHistory", e.currentTarget.checked)} /> No history</label>
            <label class="toggle"><input type="checkbox" checked={config.noPrune} onchange={(e) => updateConfig("noPrune", e.currentTarget.checked)} /> No prune</label>
            <label class="toggle"><input type="checkbox" checked={config.scheduleSpread} onchange={(e) => updateConfig("scheduleSpread", e.currentTarget.checked)} /> Schedule spread</label>
            <label class="toggle"><input type="checkbox" checked={config.multiUserCache} onchange={(e) => updateConfig("multiUserCache", e.currentTarget.checked)} /> Multi-user cache</label>
          </div>
        </div>
      </section>
      {:else if activeView === "logs"}
      <section id="logs" class="w-full">
        <div class="rounded-lg border border-border bg-surface p-4">
          <h2 class="text-[14px] font-semibold text-fg">Logs</h2>
          <p class="mt-1 text-[11px] text-fg-subtle">{overview.logPath}</p>
          <pre class="mt-3 h-[520px] overflow-auto rounded-md bg-bg p-3 text-[11px] leading-relaxed text-fg-muted">{logLines.length ? logLines.join("\n") : "No log lines yet. Start the managed server or enable Follow after it writes to the log."}</pre>
        </div>
      </section>
      {/if}
    {:else}
      <div class="rounded-lg border border-status-unhealthy/30 bg-status-unhealthy/10 p-6 text-[13px] text-fg">
        <p class="font-semibold">Couldn't load Ollama{loadError ? "" : "…"}</p>
        <p class="mt-1 text-[12px] text-fg-muted">The local AI service didn't respond. It may still be starting up, or the backend is unavailable.</p>
        <button
          type="button"
          class="mt-4 rounded-md border border-border bg-surface px-3 py-1.5 text-[12px] font-medium text-fg hover:bg-bg disabled:opacity-60"
          disabled={loading}
          onclick={() => refresh()}
        >
          <Icon name="rotate-cw" size={13} class="inline mr-1" /> {loading ? "Retrying…" : "Retry"}
        </button>
      </div>
    {/if}
  </div>
  </main>
</div>

<style>
  :global(.field) {
    width: 100%;
    border-radius: 0.375rem;
    border: 1px solid var(--color-border);
    background: var(--color-bg);
    padding: 0.5rem 0.625rem;
    color: var(--color-fg);
    font-size: 12px;
  }

  :global(.field:disabled) {
    opacity: 0.6;
  }

  :global(.toggle) {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    border: 1px solid var(--color-border);
    border-radius: 0.375rem;
    padding: 0.45rem 0.65rem;
    color: var(--color-fg);
    font-size: 12px;
  }

</style>
