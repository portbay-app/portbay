import { describe, expect, it } from "vitest";

import { PrefixCache } from "../src/lib/autocomplete/cache";
import { postprocess } from "../src/lib/autocomplete/postprocess";

describe("PrefixCache (longest-prefix LRU)", () => {
  it("returns the remaining completion after the user types into a suggestion", () => {
    const c = new PrefixCache();
    c.set("console.", "log('hi')");
    // User typed "l" — should now suggest the rest, "og('hi')".
    expect(c.get("console.l")).toBe("og('hi')");
    expect(c.get("console.lo")).toBe("g('hi')");
  });

  it("returns the exact completion on an exact prefix hit", () => {
    const c = new PrefixCache();
    c.set("git ", "status");
    expect(c.get("git ")).toBe("status");
  });

  it("misses when the typed chars diverge from the cached completion", () => {
    const c = new PrefixCache();
    c.set("console.", "log('hi')");
    expect(c.get("console.e")).toBeNull(); // 'e' != 'l'
  });

  it("evicts the oldest entry past capacity", () => {
    const c = new PrefixCache(2);
    c.set("a", "1");
    c.set("b", "2");
    c.set("c", "3"); // evicts "a"
    expect(c.get("a")).toBeNull();
    expect(c.get("b")).toBe("2");
    expect(c.get("c")).toBe("3");
  });
});

describe("postprocess", () => {
  const base = { prefix: "const x = ", suffix: "", multiline: true };

  it("strips code fences the model wraps around output", () => {
    expect(postprocess({ ...base, completion: "```js\n42\n```" })).toBe("42");
  });

  it("drops blank / whitespace-only completions", () => {
    expect(postprocess({ ...base, completion: "   \n  " })).toBeNull();
  });

  it("rejects a completion that just re-emits the current line", () => {
    expect(
      postprocess({ completion: "foo()", prefix: "foo()", suffix: "", multiline: true }),
    ).toBeNull();
  });

  it("rejects extreme repetition", () => {
    const repeated = Array(8).fill("x = x + 1").join("\n");
    expect(postprocess({ ...base, completion: repeated })).toBeNull();
  });

  it("keeps only the first line for single-line surfaces", () => {
    expect(
      postprocess({ completion: "status\n--short", prefix: "git ", suffix: "", multiline: false }),
    ).toBe("status");
  });

  it("drops a leading space the prefix already provides", () => {
    expect(postprocess({ completion: " value", prefix: "x = ", suffix: "", multiline: true })).toBe(
      "value",
    );
  });

  it("stops the completion where it runs into the suffix", () => {
    const out = postprocess({
      completion: "name: string }",
      prefix: "interface T { ",
      suffix: " }\nconst y = 1",
      multiline: true,
    });
    expect(out).toBe("name: string");
  });
});
