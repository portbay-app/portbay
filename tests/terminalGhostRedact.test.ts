import { describe, expect, it } from "vitest";

import { nextWord, redactSecrets } from "../src/lib/ide/terminal/terminalContext";

describe("redactSecrets — buffer-context scrub", () => {
  it("masks key=value / key: value secrets, keeping the key", () => {
    expect(redactSecrets("export TOKEN=abc123def456")).toBe("export TOKEN=***"); // gitleaks:allow — fake fixture for the redactor under test
    expect(redactSecrets("api_key: sk_live_9f8e7d")).toBe("api_key: ***"); // gitleaks:allow — fake fixture for the redactor under test
    expect(redactSecrets("PASSWORD=hunter2")).toBe("PASSWORD=***");
  });

  it("masks recognizable credential shapes anywhere", () => {
    expect(redactSecrets("AKIAIOSFODNN7EXAMPLE here")).toBe("AKIA*** here");
    expect(redactSecrets("token ghp_0123456789abcdefghijABCDEF")).toContain("gh_***");
    expect(redactSecrets("key sk-0123456789abcdefghijklmn")).toContain("sk-***");
  });

  it("masks JWTs", () => {
    const jwt = "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0NTY3.SflKxwRJSMeKKF2QT4";
    expect(redactSecrets(`auth ${jwt}`)).toContain("jwt***");
  });

  it("leaves ordinary commands and output untouched", () => {
    const plain = "$ git status\n$ npm run build\nBuild succeeded in 4.2s";
    expect(redactSecrets(plain)).toBe(plain);
  });
});

describe("nextWord — partial accept boundary", () => {
  it("takes a single word, keeping a leading space", () => {
    expect(nextWord("status --short")).toBe("status");
    expect(nextWord(" --short")).toBe(" --short");
  });

  it("takes a path-ish run as one word", () => {
    expect(nextWord("install/run.sh next")).toBe("install/run.sh");
  });
});
