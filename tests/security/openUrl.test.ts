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
});
