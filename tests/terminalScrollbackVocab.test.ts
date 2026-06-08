import { describe, expect, it } from "vitest";

import {
  readScrollback,
  registerScrollbackReader,
} from "../src/lib/dictation/terminalScrollback";
import { extractTechnicalTerms } from "../src/lib/dictation/vocabulary";

describe("terminal scrollback vocabulary", () => {
  it("reads registered panes, active pane first", () => {
    const un1 = registerScrollbackReader("conn-1", {
      read: () => "background pane: redis-cache restarted",
      isActive: () => false,
    });
    const un2 = registerScrollbackReader("conn-1", {
      read: () => "active pane: portbay-landing deployed",
      isActive: () => true,
    });
    const text = readScrollback("conn-1");
    expect(text.indexOf("portbay-landing")).toBeLessThan(text.indexOf("redis-cache"));
    un1();
    un2();
    expect(readScrollback("conn-1")).toBe("");
  });

  it("is empty for unknown connections and isolates connections", () => {
    const un = registerScrollbackReader("conn-a", {
      read: () => "nginx-proxy up",
      isActive: () => true,
    });
    expect(readScrollback("conn-b")).toBe("");
    expect(readScrollback("conn-a")).toContain("nginx-proxy");
    un();
  });

  it("survives a reader that throws (pane mid-teardown)", () => {
    const un1 = registerScrollbackReader("conn-t", {
      read: () => {
        throw new Error("disposed xterm");
      },
      isActive: () => true,
    });
    const un2 = registerScrollbackReader("conn-t", {
      read: () => "still-alive-pane.log tail",
      isActive: () => false,
    });
    expect(readScrollback("conn-t")).toContain("still-alive-pane.log");
    un1();
    un2();
  });

  it("scrubs secrets before the text can reach the extractor", () => {
    const un = registerScrollbackReader("conn-s", {
      read: () =>
        // Fake fixtures: the assertions below prove these never survive the scrub.
        "export API_KEY=sk-abcdefghijklmnopqrstuv123456\n" + // gitleaks:allow
        "curl -H 'Authorization: Bearer ghp_AbCdEfGhIjKlMnOpQrSt123456' portbay-api.test", // gitleaks:allow
      isActive: () => true,
    });
    const text = readScrollback("conn-s");
    expect(text).not.toContain("sk-abcdefghijklmnopqrstuv123456");
    expect(text).not.toContain("ghp_AbCdEfGhIjKlMnOpQrSt123456");
    // The jargon the user would dictate about survives the scrub, and the
    // scrubbed remnants don't become vocabulary terms.
    const terms = extractTechnicalTerms([text]);
    expect(terms).toContain("portbay-api.test");
    expect(terms.some((t) => t.includes("ghp_AbCdEf"))).toBe(false);
    un();
  });
});
