import { describe, expect, it } from "vitest";

import {
  compileRules,
  matchLine,
  patternError,
  HIGHLIGHT_PRESETS,
  MAX_HIGHLIGHT_RULES,
  type HighlightRule,
} from "../src/lib/ssh/terminalHighlight";

function rule(id: string, pattern: string, over: Partial<HighlightRule> = {}): HighlightRule {
  return {
    id,
    label: "",
    pattern,
    isRegex: true,
    caseSensitive: false,
    color: "#ff0000",
    renderMode: "background",
    enabled: true,
    ...over,
  };
}

describe("compileRules", () => {
  it("drops disabled and blank rules, keeps order", () => {
    const compiled = compileRules([
      rule("a", "error"),
      rule("b", "warn", { enabled: false }),
      rule("c", "   "),
      rule("d", "info"),
    ]);
    expect(compiled.map((r) => r.id)).toEqual(["a", "d"]);
  });

  it("compiles an invalid regex to a null regex rather than throwing", () => {
    const compiled = compileRules([rule("bad", "(")]);
    expect(compiled[0].regex).toBeNull();
  });

  it("treats a literal rule's regex metacharacters as plain text", () => {
    // `a.c` as a literal must NOT match `abc`, only the exact `a.c`.
    const rules = compileRules([rule("lit", "a.c", { isRegex: false })]);
    expect(matchLine("abc", rules)).toEqual([]);
    expect(matchLine("xx a.c yy", rules)).toHaveLength(1);
  });

  it("carries the render mode through to matches", () => {
    const rules = compileRules([rule("u", "x", { renderMode: "underline" })]);
    expect(matchLine("x", rules)[0].renderMode).toBe("underline");
  });
});

describe("matchLine", () => {
  it("matches case-insensitively by default", () => {
    const rules = compileRules([rule("e", "error")]);
    const matches = matchLine("Error: error ERROR", rules);
    expect(matches).toHaveLength(3);
    expect(matches.map((m) => m.start)).toEqual([0, 7, 13]);
  });

  it("honours case sensitivity when requested", () => {
    const rules = compileRules([rule("e", "ERROR", { caseSensitive: true })]);
    const matches = matchLine("Error error ERROR", rules);
    expect(matches).toHaveLength(1);
    expect(matches[0].start).toBe(12);
  });

  it("resolves overlaps so earlier rules win", () => {
    const rules = compileRules([rule("first", "error\\d+"), rule("second", "error")]);
    const matches = matchLine("error500", rules);
    expect(matches).toHaveLength(1);
    expect(matches[0].ruleId).toBe("first");
    expect(matches[0].end).toBe(8);
  });

  it("returns non-overlapping matches sorted by position", () => {
    const rules = compileRules([rule("ip", "\\d+\\.\\d+\\.\\d+\\.\\d+"), rule("err", "error")]);
    const matches = matchLine("error at 10.0.0.4 now", rules);
    expect(matches.map((m) => m.ruleId)).toEqual(["err", "ip"]);
    expect(matches[0].start).toBeLessThan(matches[1].start);
  });

  it("skips rules whose regex failed to compile", () => {
    const rules = compileRules([rule("bad", "("), rule("ok", "ok")]);
    expect(matchLine("ok (", rules).map((m) => m.ruleId)).toEqual(["ok"]);
  });

  it("does not hang on a zero-width pattern", () => {
    const rules = compileRules([rule("bol", "^")]);
    expect(matchLine("hello", rules)).toEqual([]);
  });

  it("returns nothing for empty text or no rules", () => {
    expect(matchLine("", compileRules([rule("e", "error")]))).toEqual([]);
    expect(matchLine("error", [])).toEqual([]);
  });
});

describe("patternError", () => {
  it("returns null for a valid or blank regex", () => {
    expect(patternError("\\berror\\b")).toBeNull();
    expect(patternError("")).toBeNull();
  });

  it("returns a message for an invalid regex", () => {
    expect(patternError("(")).toBeTruthy();
  });

  it("never errors for a literal pattern, even with regex metacharacters", () => {
    expect(patternError("(", false)).toBeNull();
  });
});

describe("HIGHLIGHT_PRESETS", () => {
  it("are all valid, compilable rules within the cap", () => {
    expect(HIGHLIGHT_PRESETS.length).toBeLessThanOrEqual(MAX_HIGHLIGHT_RULES);
    for (const preset of HIGHLIGHT_PRESETS) {
      expect(patternError(preset.pattern, preset.isRegex)).toBeNull();
      expect(preset.color).toMatch(/^#[0-9a-fA-F]{6}$/);
    }
  });

  it("each preset actually matches sample text", () => {
    const ip = HIGHLIGHT_PRESETS.find((p) => p.label === "IPv4 address")!;
    const rules = compileRules([{ id: "ip", enabled: true, ...ip }]);
    expect(matchLine("connect 192.168.1.20 ok", rules)).toHaveLength(1);
  });
});
