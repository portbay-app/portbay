/**
 * SftpSearchCache — the deep-search result cache behind the SFTP browser's
 * "search subfolders" mode. Pins the exact-hit path, the substring-narrowing
 * shortcut (a longer query answered from a cached superset without
 * re-walking the server), and the rules for what must NOT be cached.
 */
import { describe, expect, it, vi } from "vitest";

// The cache itself is pure; the module's startSftpSearch pulls in the IPC
// layer, which isn't under test here.
vi.mock("$lib/ipc", () => ({ invokeQuiet: vi.fn() }));

import { SftpSearchCache, type CachedSearch } from "$lib/sftpSearch";
import type { SftpEntry } from "$lib/types/sshTunnels";

const entry = (name: string): SftpEntry => ({
  name,
  path: `/srv/${name}`,
  isDir: false,
  isSymlink: false,
  size: 1,
  permissions: 0o644,
  mtimeSecs: null,
});

const result = (names: string[], scanned = 100, truncated = false): CachedSearch => ({
  hits: names.map(entry),
  scanned,
  truncated,
});

describe("SftpSearchCache", () => {
  it("answers an identical query under the same root", () => {
    const c = new SftpSearchCache();
    c.store("/srv", "client", result(["client.txt", "clients.csv"]));
    const hit = c.resolve("/srv", "client");
    expect(hit?.hits.map((h) => h.name)).toEqual(["client.txt", "clients.csv"]);
  });

  it("misses for a different root", () => {
    const c = new SftpSearchCache();
    c.store("/srv", "client", result(["client.txt"]));
    expect(c.resolve("/home", "client")).toBeNull();
  });

  it("is query-case-insensitive", () => {
    const c = new SftpSearchCache();
    c.store("/srv", "Client", result(["client.txt"]));
    expect(c.resolve("/srv", "CLIENT")).not.toBeNull();
  });

  it("narrows an extended query from the cached superset", () => {
    // "clients" can only match a subset of what "client" matched, so the
    // cached walk is authoritative — no server round-trip needed.
    const c = new SftpSearchCache();
    c.store("/srv", "client", result(["client.txt", "clients.csv", "clientele.md"]));
    const hit = c.resolve("/srv", "clients");
    expect(hit?.hits.map((h) => h.name)).toEqual(["clients.csv"]);
  });

  it("narrows from the LONGEST cached substring (smallest superset)", () => {
    const c = new SftpSearchCache();
    c.store("/srv", "cli", result(["cli.sh", "client.txt", "clients.csv"], 500));
    c.store("/srv", "client", result(["client.txt", "clients.csv"], 200));
    const hit = c.resolve("/srv", "clients");
    // Derived from the "client" walk (scanned 200), not the "cli" one.
    expect(hit?.scanned).toBe(200);
    expect(hit?.hits.map((h) => h.name)).toEqual(["clients.csv"]);
  });

  it("does NOT narrow a shorter query from a longer one", () => {
    // "client" results can't answer "cli" — the walk for "client" skipped
    // files matching only "cli".
    const c = new SftpSearchCache();
    c.store("/srv", "client", result(["client.txt"]));
    expect(c.resolve("/srv", "cli")).toBeNull();
  });

  it("never narrows to or from glob queries", () => {
    const c = new SftpSearchCache();
    // A glob superset can't seed substring narrowing…
    c.store("/srv", "*.zip", result(["a.zip"]));
    expect(c.resolve("/srv", "*.zipx")).toBeNull();
    // …and a glob query never answers from a plain cached walk, even though
    // "zip" is a substring of "*.zip".
    const c2 = new SftpSearchCache();
    c2.store("/srv", "zip", result(["a.zip", "b.zip"]));
    expect(c2.resolve("/srv", "*.zip")).toBeNull();
  });

  it("answers an exact glob repeat", () => {
    const c = new SftpSearchCache();
    c.store("/srv", "*.zip", result(["a.zip"]));
    expect(c.resolve("/srv", "*.zip")?.hits.map((h) => h.name)).toEqual(["a.zip"]);
  });

  it("refuses to store truncated walks (they may have missed matches)", () => {
    const c = new SftpSearchCache();
    c.store("/srv", "client", result(["client.txt"], 100, true));
    expect(c.resolve("/srv", "client")).toBeNull();
  });

  it("ignores empty queries", () => {
    const c = new SftpSearchCache();
    c.store("/srv", "  ", result(["a"]));
    expect(c.resolve("/srv", "")).toBeNull();
    expect(c.resolve("/srv", "   ")).toBeNull();
  });

  it("invalidate() drops everything (tree changed)", () => {
    const c = new SftpSearchCache();
    c.store("/srv", "client", result(["client.txt"]));
    c.invalidate();
    expect(c.resolve("/srv", "client")).toBeNull();
  });

  it("evicts the oldest entries past the per-root cap", () => {
    const c = new SftpSearchCache();
    for (let i = 0; i < 30; i++) c.store("/srv", `query-${i}`, result([`f${i}`]));
    expect(c.resolve("/srv", "query-0")).toBeNull(); // evicted
    expect(c.resolve("/srv", "query-29")).not.toBeNull(); // newest survives
  });
});
