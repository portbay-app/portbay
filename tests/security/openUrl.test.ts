import { describe, expect, it } from "vitest";
import { assertSafeOpenUrl } from "$lib/security/urlPolicy";

describe("assertSafeOpenUrl", () => {
  it("allows http, https, and file URLs", () => {
    expect(assertSafeOpenUrl("https://portbay.app/docs")).toBe("https://portbay.app/docs");
    expect(assertSafeOpenUrl("http://127.0.0.1:8025")).toBe("http://127.0.0.1:8025/");
    expect(assertSafeOpenUrl("file:///Users/nour/Sites/app")).toBe("file:///Users/nour/Sites/app");
  });

  it("blocks custom schemes from webview-triggered opener calls", () => {
    expect(() => assertSafeOpenUrl("javascript:alert(1)")).toThrow();
    expect(() => assertSafeOpenUrl("x-apple.systempreferences:Privacy_AllFiles")).toThrow();
    expect(() => assertSafeOpenUrl("mysql://root@127.0.0.1/db")).toThrow();
  });

  it("recovers linkified URLs that dragged in trailing punctuation", () => {
    // codex login prints "…on http://localhost:1455." — the period lands in
    // the port and the raw string is unparseable.
    expect(assertSafeOpenUrl("http://localhost:1455.")).toBe("http://localhost:1455/");
    expect(assertSafeOpenUrl("http://localhost:1455,")).toBe("http://localhost:1455/");
  });

  it("does not trim punctuation off URLs that already parse", () => {
    // Trailing punctuation in a *path* is valid as-is; only unparseable
    // strings get the retry, so parseable URLs pass through untouched.
    expect(assertSafeOpenUrl("https://portbay.app/docs.")).toBe("https://portbay.app/docs.");
    expect(assertSafeOpenUrl("https://portbay.app/docs).")).toBe("https://portbay.app/docs).");
  });

  it("still rejects strings that stay unparseable after the trim", () => {
    expect(() => assertSafeOpenUrl("http://[bad")).toThrow();
  });
});
