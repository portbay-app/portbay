import { describe, expect, it } from "vitest";

import { extractTechnicalTerms } from "../src/lib/dictation/vocabulary";

describe("extractTechnicalTerms", () => {
  it("keeps technical shapes and drops plain words", () => {
    const terms = extractTechnicalTerms([
      "restart the nginx-proxy service and check /var/log/nginx/error.log please",
    ]);
    expect(terms).toContain("nginx-proxy");
    expect(terms).toContain("/var/log/nginx/error.log");
    expect(terms).not.toContain("restart");
    expect(terms).not.toContain("the");
    expect(terms).not.toContain("please");
  });

  it("recognizes flags, versions, camelCase, and digit mixes", () => {
    const terms = extractTechnicalTerms([
      "run cargo with --dry-run against portbay-app v0.1.4 using qwen2 and ideEditor state",
    ]);
    expect(terms).toEqual(
      expect.arrayContaining(["--dry-run", "portbay-app", "v0.1.4", "qwen2", "ideEditor"]),
    );
  });

  it("strips wrapping punctuation but keeps internal structure", () => {
    const terms = extractTechnicalTerms(['fails in `src-tauri/src/lib.rs`, see (russh-sftp).']);
    expect(terms).toContain("src-tauri/src/lib.rs");
    expect(terms).toContain("russh-sftp");
  });

  it("dedupes case-insensitively keeping the first spelling, in source order", () => {
    const terms = extractTechnicalTerms([
      "deploy portbay-landing now",
      "PORTBAY-LANDING and api.portbay.test",
    ]);
    expect(terms).toEqual(["portbay-landing", "api.portbay.test"]);
  });

  it("caps the result and prioritizes earlier sources", () => {
    const noisy = Array.from({ length: 30 }, (_, i) => `term-${i}x`).join(" ");
    const terms = extractTechnicalTerms(["first-term second.term", noisy], 4);
    expect(terms).toHaveLength(4);
    expect(terms[0]).toBe("first-term");
    expect(terms[1]).toBe("second.term");
  });

  it("ignores pure numbers and empty sources", () => {
    expect(extractTechnicalTerms(["", "8080 0.1.4 3,000", "   "])).toEqual([]);
  });
});
