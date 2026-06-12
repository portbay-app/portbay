import { describe, expect, it } from "vitest";

import {
  estimateRequiredGb,
  estimateTps,
  fitLevel,
  parseParams,
  scoreVariant,
  useCaseFor,
  type HardwareProfile,
} from "../src/lib/hwfit";

// The machine the acceptance criteria were written against (this dev Mac).
const M2_16GB: HardwareProfile = {
  chip: "Apple M2",
  totalRamGb: 16,
  budgetGb: 16 * (2 / 3),
  bandwidthGbps: 100,
  estimated: false,
};

const M4_MAX_64GB: HardwareProfile = {
  chip: "Apple M4 Max",
  totalRamGb: 64,
  budgetGb: 48,
  bandwidthGbps: 546,
  estimated: false,
};

describe("parseParams", () => {
  it("parses dense size hints", () => {
    expect(parseParams("7b")).toEqual({ totalB: 7, activeB: 7, moe: false });
    expect(parseParams("3.8b")).toEqual({ totalB: 3.8, activeB: 3.8, moe: false });
    expect(parseParams("0.6b")).toEqual({ totalB: 0.6, activeB: 0.6, moe: false });
  });

  it("parses MoE active-param hints", () => {
    expect(parseParams("30b-a3b")).toEqual({ totalB: 30, activeB: 3, moe: true });
    expect(parseParams("235b-a22b")).toEqual({ totalB: 235, activeB: 22, moe: true });
  });

  it("parses million-scale hints", () => {
    expect(parseParams("335m")?.totalB).toBeCloseTo(0.335);
  });

  it("rejects cloud and unknown hints", () => {
    expect(parseParams("Cloud — no download")).toBeNull();
    expect(parseParams("latest")).toBeNull();
    expect(parseParams("")).toBeNull();
  });
});

describe("estimateRequiredGb", () => {
  it("prefers the exact download size when known", () => {
    const params = parseParams("7b")!;
    // 4.7 GB download (qwen2.5:7b) → ~5.9 GB resident
    expect(estimateRequiredGb(params, 4.7)).toBeCloseTo(4.7 * 1.08 + 0.8, 1);
  });

  it("falls back to Q4 bytes-per-param without a download size", () => {
    const params = parseParams("7b")!;
    const est = estimateRequiredGb(params);
    // 7B × 0.6 B/param ≈ 4.2 GB weights → ~5.6 GB resident; sanity range.
    expect(est).toBeGreaterThan(4.5);
    expect(est).toBeLessThan(7);
  });
});

describe("estimateTps", () => {
  it("is within 2x of measured speeds for a 7B Q4 on an M2", () => {
    // Apple M2, qwen2.5:7b (4.7 GB Q4_K_M): community-measured ~10–14 tok/s.
    const tps = estimateTps(parseParams("7b")!, 100, 4.7);
    expect(tps).toBeGreaterThan(6);
    expect(tps).toBeLessThan(25);
  });

  it("scores MoE on active params, not total", () => {
    const moe = estimateTps(parseParams("30b-a3b")!, 546, 19);
    const dense = estimateTps(parseParams("30b")!, 546, 19);
    // Only 3/30 of the weights stream per token — close to an order of
    // magnitude faster than the dense model of the same size.
    expect(moe).toBeGreaterThan(dense * 5);
  });
});

describe("fitLevel", () => {
  it("classifies against the usable budget", () => {
    expect(fitLevel(5, 10.7)).toBe("fits");
    expect(fitLevel(9.5, 10.7)).toBe("tight");
    expect(fitLevel(12, 10.7)).toBe("too-tight");
    expect(fitLevel(Number.NaN, 10.7)).toBe("unknown");
  });
});

describe("scoreVariant — acceptance on real machines", () => {
  it("M2 16GB: 7b runs, 14b is tight, 70b is too tight", () => {
    expect(scoreVariant("7b", M2_16GB, "general", 4.7).level).toBe("fits");
    expect(scoreVariant("14b", M2_16GB, "coding", 9.0).level).toBe("tight");
    expect(scoreVariant("70b", M2_16GB, "general", 43).level).toBe("too-tight");
  });

  it("M4 Max 64GB: qwen2.5-coder:14b runs well, 70b is edge-of-budget tight", () => {
    expect(scoreVariant("14b", M4_MAX_64GB, "coding", 9.0).level).toBe("fits");
    // llama3.3:70b Q4 (43 GB) really does run on a 64 GB M4 Max — barely.
    expect(scoreVariant("70b", M4_MAX_64GB, "general", 43).level).toBe("tight");
    // …but a 16 GB budget machine never sees it as runnable (asserted above).
  });

  it("ranks a runnable mid-size model above both a too-tight giant and a tiny model", () => {
    const seven = scoreVariant("7b", M2_16GB, "general", 4.7);
    const giant = scoreVariant("70b", M2_16GB, "general", 43);
    const tiny = scoreVariant("0.6b", M2_16GB, "general", 0.5);
    expect(seven.score).toBeGreaterThan(giant.score);
    expect(seven.score).toBeGreaterThan(tiny.score);
  });

  it("too-tight models report no speed estimate", () => {
    const giant = scoreVariant("70b", M2_16GB, "general", 43);
    expect(giant.tps).toBeNull();
    expect(giant.score).toBe(0);
  });

  it("scores size-only rows (embedding models) from the download size", () => {
    // nomic-embed-text: no parameter badge, 274 MB download.
    const fit = scoreVariant("latest", M2_16GB, "embedding", 0.274);
    expect(fit.level).toBe("fits");
    expect(fit.tps).toBeGreaterThan(100);
  });

  it("returns unknown without a size hint or download size", () => {
    expect(scoreVariant("latest", M2_16GB).level).toBe("unknown");
  });

  it("survives a missing bandwidth (no tps, fit still scored)", () => {
    const noBw: HardwareProfile = { ...M2_16GB, bandwidthGbps: null };
    const fit = scoreVariant("7b", noBw, "general", 4.7);
    expect(fit.level).toBe("fits");
    expect(fit.tps).toBeNull();
    expect(fit.score).toBeGreaterThan(0);
  });
});

describe("useCaseFor", () => {
  it("maps families and model names to scoring use cases", () => {
    expect(useCaseFor("embeddings", "nomic-embed-text")).toBe("embedding");
    expect(useCaseFor("qwen25", "qwen2.5-coder")).toBe("coding");
    expect(useCaseFor("mistral", "devstral")).toBe("coding");
    expect(useCaseFor("qwen25", "qwen2.5")).toBe("general");
  });
});
