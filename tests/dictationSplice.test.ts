import { describe, expect, it } from "vitest";

import {
  extractInsertion,
  relocateInsertion,
  spliceRewrite,
  worthRewriting,
} from "$lib/dictation/splice";

describe("extractInsertion", () => {
  it("finds an append into an empty field", () => {
    const ins = extractInsertion("", "um fix the login bug");
    expect(ins).toEqual({
      prefixLen: 0,
      suffixLen: 0,
      inserted: "um fix the login bug",
    });
  });

  it("finds an append after existing text", () => {
    const ins = extractInsertion("Existing note. ", "Existing note. and also restart nginx");
    expect(ins?.prefixLen).toBe("Existing note. ".length);
    expect(ins?.suffixLen).toBe(0);
    expect(ins?.inserted).toBe("and also restart nginx");
  });

  it("finds an insertion in the middle of existing text", () => {
    const before = "Header\n\nFooter";
    const after = "Header\num check the logs\nFooter";
    const ins = extractInsertion(before, after);
    expect(ins?.inserted).toBe("um check the logs");
    expect(after.slice(0, ins!.prefixLen)).toBe("Header\n");
    expect(after.slice(after.length - ins!.suffixLen)).toBe("\nFooter");
  });

  it("returns null when nothing changed", () => {
    expect(extractInsertion("same", "same")).toBeNull();
    expect(extractInsertion("", "")).toBeNull();
  });

  it("returns null for a pure deletion", () => {
    expect(extractInsertion("delete some words", "delete words")).toBeNull();
  });

  it("does not let prefix and suffix overlap on repeated text", () => {
    // "aa" -> "aaa": one "a" inserted; prefix+suffix must not double-count.
    const ins = extractInsertion("aa", "aaa");
    expect(ins).not.toBeNull();
    expect(ins!.prefixLen + ins!.suffixLen).toBeLessThanOrEqual("aa".length);
    expect(ins!.inserted).toBe("a");
  });

  it("treats a mid-session manual edit as part of the insertion", () => {
    // The user typed a correction while dictating — still this session's text.
    const ins = extractInsertion("Note: ", "Note: deploy v2 (typed fix) to staging");
    expect(ins?.inserted).toBe("deploy v2 (typed fix) to staging");
  });
});

describe("worthRewriting", () => {
  it("skips short confirmations", () => {
    expect(worthRewriting("yes")).toBe(false);
    expect(worthRewriting("ok done")).toBe(false);
    expect(worthRewriting("  done.  ")).toBe(false);
  });

  it("accepts a three-word task", () => {
    expect(worthRewriting("fix login bug")).toBe(true);
  });

  it("requires both length and word count", () => {
    expect(worthRewriting("a b c")).toBe(false); // 3 words but tiny
    expect(worthRewriting("reconfigure nginx")).toBe(false); // long but 2 words
  });
});

describe("spliceRewrite", () => {
  const ins = (before: string, after: string) => extractInsertion(before, after)!;

  it("replaces a plain append", () => {
    const i = ins("", "um fix the uh login bug");
    expect(spliceRewrite("um fix the uh login bug", i, "Fix the login bug.")).toBe(
      "Fix the login bug.",
    );
  });

  it("repairs missing spaces at both joints", () => {
    const before = "Start.";
    const after = "Start.middle bit here";
    const i = ins(before, after);
    // Rewritten segment splices in with a space after "Start.".
    expect(spliceRewrite(after, i, "Middle bit here.")).toBe("Start. Middle bit here.");
  });

  it("adds no space when the joints already have whitespace", () => {
    const before = "Header\n\nFooter";
    const after = "Header\nnew line content goes here\nFooter";
    const i = ins(before, after);
    expect(spliceRewrite(after, i, "New line content goes here.")).toBe(
      "Header\nNew line content goes here.\nFooter",
    );
  });

  it("adds no space against brackets at either joint", () => {
    const before = "Wrap () end";
    const after = "Wrap (some new words inside) end";
    const i = ins(before, after);
    // Diff anchors on "(" and ") end"; the rewrite must butt against both.
    expect(i.inserted).toBe("some new words inside");
    const out = spliceRewrite(after, i, "Some new words inside.");
    expect(out).toBe("Wrap (Some new words inside.) end");
  });
});

describe("relocateInsertion", () => {
  it("re-anchors when surrounding text changed", () => {
    const found = relocateInsertion("PREPENDED um fix the bug", "um fix the bug");
    expect(found).not.toBeNull();
    expect(found!.prefixLen).toBe("PREPENDED ".length);
    expect(found!.suffixLen).toBe(0);
  });

  it("fails when the raw segment was edited away", () => {
    expect(relocateInsertion("totally different now", "um fix the bug")).toBeNull();
  });

  it("fails when the segment appears twice (ambiguous)", () => {
    expect(relocateInsertion("dup text … dup text", "dup text")).toBeNull();
  });
});
