import { describe, expect, it } from "vitest";

import {
  createPathCompleter,
  type DirEntry,
  isPathContext,
  parseOsc7Cwd,
  pathToken,
  resolvePath,
  splitToken,
} from "../src/lib/ide/terminal/pathComplete";

const sig = () => new AbortController().signal;

describe("pathToken", () => {
  it("treats the argument of a path command as a path", () => {
    expect(pathToken("cd a")).toBe("a");
    expect(pathToken("ls -la src")).toBe("src");
    expect(pathToken("cat clients/ak")).toBe("clients/ak");
  });

  it("ignores non-path commands unless the token is explicitly a path", () => {
    expect(pathToken("git checkout ma")).toBeNull();
    expect(pathToken("echo hello")).toBeNull();
    expect(pathToken("git ./scripts/de")).toBe("./scripts/de");
  });

  it("completes a bare first token only when it is itself a path", () => {
    expect(pathToken("./scr")).toBe("./scr");
    expect(pathToken("/usr/bin/th")).toBe("/usr/bin/th");
    expect(pathToken("vim")).toBeNull(); // the command word itself, no arg yet
  });

  it("strips a leading quote", () => {
    expect(pathToken('cd "my dir/fi')).toBe("my dir/fi");
  });

  it("resolves a slashed command name to its base", () => {
    expect(pathToken("/usr/bin/vim src/ma")).toBe("src/ma");
  });
});

describe("isPathContext", () => {
  it("flags path-argument lines that must only use real listings", () => {
    expect(isPathContext("cd clients/ak")).toBe(true);
    expect(isPathContext("cat /etc/ho")).toBe(true);
    expect(isPathContext("./scr")).toBe(true);
  });

  it("does not flag command lines that should keep model/history completion", () => {
    expect(isPathContext("git checkout ma")).toBe(false);
    expect(isPathContext("npm run de")).toBe(false);
    expect(isPathContext("")).toBe(false);
  });
});

describe("splitToken", () => {
  it("splits a slashed token into dir + base", () => {
    expect(splitToken("clients/ak")).toEqual({ dir: "clients/", base: "ak" });
    expect(splitToken("a")).toEqual({ dir: "", base: "a" });
    expect(splitToken("clients/")).toEqual({ dir: "clients/", base: "" });
    expect(splitToken("/etc/ho")).toEqual({ dir: "/etc/", base: "ho" });
  });
});

describe("resolvePath", () => {
  const home = "/home/nour";
  const cwd = "/home/nour/clients";

  it("resolves relative against cwd", () => {
    expect(resolvePath("", cwd, home)).toBe("/home/nour/clients");
    expect(resolvePath("sub/", cwd, home)).toBe("/home/nour/clients/sub");
  });

  it("expands ~", () => {
    expect(resolvePath("~", cwd, home)).toBe("/home/nour");
    expect(resolvePath("~/projects/", cwd, home)).toBe("/home/nour/projects");
  });

  it("keeps absolute paths and normalises . / ..", () => {
    expect(resolvePath("/var/log/", cwd, home)).toBe("/var/log");
    expect(resolvePath("../", cwd, home)).toBe("/home/nour");
    expect(resolvePath("a/./b/../c/", cwd, home)).toBe("/home/nour/clients/a/c");
  });

  it("returns null when a relative ref can't be resolved yet", () => {
    expect(resolvePath("sub", null, home)).toBeNull();
    expect(resolvePath("~/x", cwd, null)).toBeNull();
  });
});

describe("parseOsc7Cwd", () => {
  it("extracts the path from file://host/path", () => {
    expect(parseOsc7Cwd("file://myhost/home/nour/clients")).toBe("/home/nour/clients");
    expect(parseOsc7Cwd("file:///var/www/")).toBe("/var/www");
  });

  it("decodes percent-encoding and accepts a bare path", () => {
    expect(parseOsc7Cwd("file://h/home/a%20b")).toBe("/home/a b");
    expect(parseOsc7Cwd("/home/nour")).toBe("/home/nour");
  });

  it("rejects non-absolute payloads", () => {
    expect(parseOsc7Cwd("")).toBeNull();
    expect(parseOsc7Cwd("relative/path")).toBeNull();
  });
});

describe("createPathCompleter", () => {
  const home = "/home/nour";
  function completer(entries: Record<string, DirEntry[]>, cwd = "/home/nour/clients") {
    const calls: string[] = [];
    const c = createPathCompleter({
      list: async (absDir) => {
        calls.push(absDir);
        return entries[absDir] ?? null;
      },
      getCwd: () => cwd,
      getHome: () => home,
    });
    return { c, calls };
  }

  it("offers the rest of a unique directory match, with a trailing slash", async () => {
    const { c } = completer({
      "/home/nour/clients": [{ name: "akkakappaghana.com", isDir: true }],
    });
    expect(await c.complete("cd a", sig())).toBe("kkakappaghana.com/");
  });

  it("does not glue onto the command — it is purely the token remainder", async () => {
    const { c } = completer({
      "/home/nour/clients": [{ name: "akkakappaghana.com", isDir: true }],
    });
    // `cd ` + ghost should read `cd akkakappaghana.com/`, never `cdakk…`.
    expect(await c.complete("cd ak", sig())).toBe("kakappaghana.com/");
  });

  it("descends into nested directories relative to cwd", async () => {
    const { c, calls } = completer({
      "/home/nour/clients/akkakappaghana.com": [{ name: "public_html", isDir: true }],
    });
    expect(await c.complete("cd akkakappaghana.com/pub", sig())).toBe("lic_html/");
    expect(calls).toContain("/home/nour/clients/akkakappaghana.com");
  });

  it("offers only the common prefix when ambiguous, no slash", async () => {
    const { c } = completer({
      "/home/nour/clients": [
        { name: "shop-alpha", isDir: true },
        { name: "shop-beta", isDir: true },
      ],
    });
    expect(await c.complete("cd shop", sig())).toBe("-"); // common prefix "shop-"
  });

  it("returns null when nothing matches", async () => {
    const { c } = completer({ "/home/nour/clients": [{ name: "alpha", isDir: true }] });
    expect(await c.complete("cd zzz", sig())).toBeNull();
  });

  it("hides dotfiles until the user types the leading dot", async () => {
    const entries = {
      "/home/nour/clients": [
        { name: ".env", isDir: false },
        { name: "app", isDir: true },
      ],
    };
    const { c } = completer(entries);
    expect(await c.complete("cat ", sig())).toBe("app/"); // only the visible dir
    const { c: c2 } = completer(entries);
    expect(await c2.complete("cat .e", sig())).toBe("nv");
  });

  it("appends no slash for a plain file", async () => {
    const { c } = completer({
      "/home/nour/clients": [{ name: "readme.md", isDir: false }],
    });
    expect(await c.complete("cat read", sig())).toBe("me.md");
  });

  it("stays silent for non-path contexts (no listing)", async () => {
    const { c, calls } = completer({ "/home/nour/clients": [{ name: "app", isDir: true }] });
    expect(await c.complete("git checkout ma", sig())).toBeNull();
    expect(calls).toHaveLength(0);
  });

  it("caches the listing within the TTL and re-lists after invalidate", async () => {
    let now = 1000;
    const calls: string[] = [];
    const c = createPathCompleter({
      list: async (absDir) => {
        calls.push(absDir);
        return [{ name: "alpha", isDir: true }];
      },
      getCwd: () => "/home/nour/clients",
      getHome: () => home,
      now: () => now,
      cacheTtlMs: 5000,
    });
    await c.complete("cd a", sig());
    await c.complete("cd al", sig());
    expect(calls).toHaveLength(1); // second probe served from cache
    c.invalidate();
    await c.complete("cd al", sig());
    expect(calls).toHaveLength(2);
    now += 6000; // past TTL
    await c.complete("cd al", sig());
    expect(calls).toHaveLength(3);
  });
});
