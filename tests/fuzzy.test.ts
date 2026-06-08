import { describe, expect, it } from "vitest";

import { fuzzyMatch } from "../src/lib/fuzzy";

describe("fuzzyMatch", () => {
  it("matches a subsequence and reports the matched indices", () => {
    const m = fuzzyMatch("dsc", "doc.sc");
    expect(m).not.toBeNull();
    // d(0) s(4) c(5)
    expect(m!.indices).toEqual([0, 4, 5]);
  });

  it("returns null when not every query char is present in order", () => {
    expect(fuzzyMatch("zzz", "model")).toBeNull();
    expect(fuzzyMatch("cba", "abc")).toBeNull();
  });

  it("treats an empty query as a neutral match with no highlights", () => {
    expect(fuzzyMatch("", "anything")).toEqual({ score: 0, indices: [] });
  });

  it("ranks consecutive and start-anchored matches above scattered ones", () => {
    const tight = fuzzyMatch("son", "sonnet")!;
    const scattered = fuzzyMatch("son", "session-on")!;
    expect(tight.score).toBeGreaterThan(scattered.score);
  });

  it("rewards a word-boundary match over a mid-word one", () => {
    const boundary = fuzzyMatch("o", "claude-opus")!; // 'o' after '-'
    const mid = fuzzyMatch("o", "codex")!; // 'o' mid-word
    expect(boundary.score).toBeGreaterThan(mid.score);
  });
});
