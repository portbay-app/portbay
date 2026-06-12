// Hardware-fit scoring for the AI page's model catalog.
//
// Local LLM generation is memory-bandwidth-bound: every active weight is read
// once per token, so tokens/sec ≈ bandwidth ÷ active-weight bytes. That single
// observation drives everything here — fit badges (does the model fit the
// GPU-usable memory budget?), speed estimates, and the "Best fit" ranking.
// The hardware side (chip, RAM, bandwidth) comes from the `hardware_profile`
// Tauri command; this module is pure so it can be unit-tested directly.
//
// MoE models are scored on ACTIVE parameters for speed (only the routed
// experts run per token — why qwen3:30b-a3b is fast) but on TOTAL size for
// memory (all experts must be resident).

export interface HardwareProfile {
  chip: string;
  totalRamGb: number;
  budgetGb: number;
  bandwidthGbps: number | null;
  estimated: boolean;
}

export type FitLevel = "fits" | "tight" | "too-tight" | "unknown";

export interface VariantFit {
  level: FitLevel;
  /** Estimated resident memory (weights + KV/runtime overhead), GB. */
  requiredGb: number;
  /** Estimated generation speed; null when it can't be estimated. */
  tps: number | null;
  /** Composite 0–100 for the "Best fit" sort. */
  score: number;
}

export type UseCase = "general" | "coding" | "embedding";

/** Composite weights per use case: (quality, speed, fit). Embeddings care
 * about throughput over per-token quality; coding leans on quality. */
const USE_CASE_WEIGHTS: Record<UseCase, [number, number, number]> = {
  general: [0.45, 0.3, 0.25],
  coding: [0.5, 0.2, 0.3],
  embedding: [0.3, 0.4, 0.3],
};

/** tokens/sec that earns a full speed score. Embedding models burst far
 * higher and are judged accordingly. */
const SPEED_TARGET: Record<UseCase, number> = {
  general: 40,
  coding: 40,
  embedding: 200,
};

/** Ollama's default tags are Q4_K_M (~4.85 bits/weight ≈ 0.6 bytes/param) —
 * used to estimate weight bytes when the exact download size isn't known. */
const DEFAULT_BYTES_PER_PARAM = 0.6;

/** Runtime memory beyond the weights file: KV cache at Ollama's default
 * context plus runtime buffers. Flat because per-model architecture data
 * isn't in the catalog; tuned so estimates match `ollama ps` residency for
 * 7B–14B Q4 models (e.g. qwen2.5:14b ≈ 10.5 GB resident from a 9 GB file). */
const RUNTIME_OVERHEAD_GB = 0.8;
/** Weights expand slightly when loaded (alignment, dequant buffers). */
const LOAD_FACTOR = 1.08;

/** Fraction of theoretical bandwidth real inference achieves. */
const BANDWIDTH_EFFICIENCY = 0.55;
/** Expert routing overhead: MoE models don't hit dense streaming efficiency. */
const MOE_DISCOUNT = 0.8;

export interface ParsedParams {
  /** Total parameters, billions. */
  totalB: number;
  /** Parameters active per token, billions (== totalB for dense models). */
  activeB: number;
  moe: boolean;
}

/** Parse an Ollama size hint into parameter counts. Handles dense hints
 * ("7b", "3.8b", "0.6b") and MoE active-param hints ("30b-a3b" = 30B total,
 * 3B active). Returns null for cloud/unknown hints. */
export function parseParams(sizeHint: string): ParsedParams | null {
  const hint = sizeHint.trim().toLowerCase();
  const moe = /^(\d+(?:\.\d+)?)b-a(\d+(?:\.\d+)?)b$/.exec(hint);
  if (moe) {
    const totalB = Number.parseFloat(moe[1]);
    const activeB = Number.parseFloat(moe[2]);
    if (totalB > 0 && activeB > 0) return { totalB, activeB: Math.min(activeB, totalB), moe: true };
  }
  const dense = /^(\d+(?:\.\d+)?)([bm])\b/.exec(hint);
  if (!dense) return null;
  const n = Number.parseFloat(dense[1]);
  const totalB = dense[2] === "m" ? n / 1000 : n;
  if (!(totalB > 0)) return null;
  return { totalB, activeB: totalB, moe: false };
}

/** Resident memory estimate. Prefers the exact download size from the tags
 * page (weights on disk); falls back to params × Q4 bytes-per-param. */
export function estimateRequiredGb(params: ParsedParams, downloadGb?: number): number {
  const weightsGb =
    downloadGb && Number.isFinite(downloadGb) && downloadGb > 0
      ? downloadGb
      : params.totalB * DEFAULT_BYTES_PER_PARAM;
  return weightsGb * LOAD_FACTOR + RUNTIME_OVERHEAD_GB;
}

/** tokens/sec ≈ effective bandwidth ÷ bytes of weights touched per token.
 * For MoE only the active experts' share of the weights is read. */
export function estimateTps(
  params: ParsedParams,
  bandwidthGbps: number,
  downloadGb?: number,
): number {
  const weightsGb =
    downloadGb && Number.isFinite(downloadGb) && downloadGb > 0
      ? downloadGb
      : params.totalB * DEFAULT_BYTES_PER_PARAM;
  const activeGb = weightsGb * (params.activeB / params.totalB);
  if (!(activeGb > 0)) return 0;
  const tps = (bandwidthGbps / activeGb) * BANDWIDTH_EFFICIENCY;
  return params.moe ? tps * MOE_DISCOUNT : tps;
}

export function fitLevel(requiredGb: number, budgetGb: number): FitLevel {
  if (!(budgetGb > 0) || !Number.isFinite(requiredGb)) return "unknown";
  if (requiredGb > budgetGb) return "too-tight";
  if (requiredGb > budgetGb * 0.8) return "tight";
  return "fits";
}

/** Quality proxy from parameter count — bigger models answer better, with
 * diminishing returns. Tiers, not a curve: the composite only needs ordering. */
function qualityScore(totalB: number): number {
  if (totalB < 1) return 30;
  if (totalB < 3) return 45;
  if (totalB < 7) return 60;
  if (totalB < 10) return 75;
  if (totalB < 20) return 82;
  if (totalB < 40) return 89;
  return 95;
}

function speedScore(tps: number, useCase: UseCase): number {
  return Math.max(0, Math.min(100, (tps / SPEED_TARGET[useCase]) * 100));
}

/** Headroom score: best when the model uses ≤80% of the budget comfortably,
 * penalised as it crowds the ceiling (less room for context + everything
 * else on the machine). */
function fitScore(requiredGb: number, budgetGb: number): number {
  if (requiredGb > budgetGb || budgetGb <= 0) return 0;
  const ratio = requiredGb / budgetGb;
  if (ratio <= 0.5) return 60 + (ratio / 0.5) * 40; // tiny models leave budget idle
  if (ratio <= 0.8) return 100;
  if (ratio <= 0.9) return 70;
  return 50;
}

/** Score one catalog variant against the machine. `downloadGb` is the exact
 * size from the tags page when already fetched; omit to estimate from the
 * size hint alone. */
export function scoreVariant(
  sizeHint: string,
  hw: HardwareProfile,
  useCase: UseCase = "general",
  downloadGb?: number,
): VariantFit {
  let params = parseParams(sizeHint);
  // Rows without a parameter badge (embedding models, bare ":latest" tags)
  // can still be scored once the tags fetch supplies the exact download size:
  // back-derive a dense param count from the Q4 bytes-per-param default.
  if (!params && downloadGb && Number.isFinite(downloadGb) && downloadGb > 0) {
    const totalB = downloadGb / DEFAULT_BYTES_PER_PARAM;
    params = { totalB, activeB: totalB, moe: false };
  }
  if (!params) return { level: "unknown", requiredGb: Number.NaN, tps: null, score: 0 };
  const requiredGb = estimateRequiredGb(params, downloadGb);
  const level = fitLevel(requiredGb, hw.budgetGb);
  if (level === "too-tight" || level === "unknown") {
    return { level, requiredGb, tps: null, score: 0 };
  }
  const tps = hw.bandwidthGbps ? estimateTps(params, hw.bandwidthGbps, downloadGb) : null;
  const [wq, ws, wf] = USE_CASE_WEIGHTS[useCase];
  const score =
    qualityScore(params.totalB) * wq +
    (tps === null ? 50 : speedScore(tps, useCase)) * ws +
    fitScore(requiredGb, hw.budgetGb) * wf;
  return { level, requiredGb, tps, score };
}

/** Use case for scoring, from what the family/model is for. */
export function useCaseFor(familyId: string, modelName: string): UseCase {
  if (familyId === "embeddings" || modelName.includes("embed")) return "embedding";
  if (/coder|codellama|codeqwen|codestral|devstral|deepseek-coder/.test(modelName)) return "coding";
  return "general";
}
