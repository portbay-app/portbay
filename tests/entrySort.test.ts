/**
 * entrySort — the Finder-style ordering behind the SFTP browser's list and
 * icon views. Pins folders-first grouping, numeric-aware name comparison,
 * the per-key rules (dir sizes are meaningless, unknown mtimes sort last),
 * and that descending only flips the key — not the grouping or tiebreaks.
 */
import { describe, expect, it } from "vitest";

import { sortEntries } from "$lib/entrySort";
import type { SftpEntry } from "$lib/types/sshTunnels";

const make = (
  name: string,
  opts: Partial<Pick<SftpEntry, "isDir" | "size" | "mtimeSecs">> = {},
): SftpEntry => ({
  name,
  path: `/srv/${name}`,
  isDir: opts.isDir ?? false,
  isSymlink: false,
  size: opts.size ?? 0,
  permissions: 0o644,
  mtimeSecs: opts.mtimeSecs === undefined ? 1_700_000_000 : opts.mtimeSecs,
});

const names = (entries: SftpEntry[]) => entries.map((e) => e.name);

describe("folders-first grouping", () => {
  const mixed = [
    make("zeta.txt"),
    make("alpha", { isDir: true }),
    make("beta.txt"),
    make("omega", { isDir: true }),
  ];

  it("groups folders before files for every key and direction", () => {
    for (const key of ["name", "size", "mtime"] as const) {
      for (const dir of ["asc", "desc"] as const) {
        const sorted = sortEntries(mixed, key, dir);
        expect(sorted.slice(0, 2).every((e) => e.isDir)).toBe(true);
        expect(sorted.slice(2).every((e) => !e.isDir)).toBe(true);
      }
    }
  });
});

describe("name sort", () => {
  it("is case-insensitive and numeric-aware", () => {
    const sorted = sortEntries(
      [make("File10.txt"), make("file2.txt"), make("apple.txt"), make("Banana.txt")],
      "name",
      "asc",
    );
    expect(names(sorted)).toEqual(["apple.txt", "Banana.txt", "file2.txt", "File10.txt"]);
  });

  it("descending reverses the name order", () => {
    const sorted = sortEntries([make("a.txt"), make("c.txt"), make("b.txt")], "name", "desc");
    expect(names(sorted)).toEqual(["c.txt", "b.txt", "a.txt"]);
  });

  it("does not mutate the input array", () => {
    const input = [make("b.txt"), make("a.txt")];
    sortEntries(input, "name", "asc");
    expect(names(input)).toEqual(["b.txt", "a.txt"]);
  });
});

describe("size sort", () => {
  it("orders files by size with name as tiebreak", () => {
    const sorted = sortEntries(
      [make("big.bin", { size: 300 }), make("z.bin", { size: 100 }), make("a.bin", { size: 100 })],
      "size",
      "asc",
    );
    expect(names(sorted)).toEqual(["a.bin", "z.bin", "big.bin"]);
  });

  it("descending puts the largest first", () => {
    const sorted = sortEntries(
      [make("s.bin", { size: 1 }), make("l.bin", { size: 9 })],
      "size",
      "desc",
    );
    expect(names(sorted)).toEqual(["l.bin", "s.bin"]);
  });

  it("keeps folders name-sorted (server dir sizes are meaningless)", () => {
    const sorted = sortEntries(
      [make("zfolder", { isDir: true, size: 999 }), make("afolder", { isDir: true, size: 1 })],
      "size",
      "desc",
    );
    expect(names(sorted)).toEqual(["afolder", "zfolder"]);
  });
});

describe("mtime sort", () => {
  it("orders by modification time, oldest first ascending", () => {
    const sorted = sortEntries(
      [make("new.txt", { mtimeSecs: 300 }), make("old.txt", { mtimeSecs: 100 })],
      "mtime",
      "asc",
    );
    expect(names(sorted)).toEqual(["old.txt", "new.txt"]);
  });

  it("newest first descending", () => {
    const sorted = sortEntries(
      [make("old.txt", { mtimeSecs: 100 }), make("new.txt", { mtimeSecs: 300 })],
      "mtime",
      "desc",
    );
    expect(names(sorted)).toEqual(["new.txt", "old.txt"]);
  });

  it("sorts unknown mtimes last in both directions", () => {
    const input = [
      make("unknown.txt", { mtimeSecs: null }),
      make("known.txt", { mtimeSecs: 100 }),
    ];
    expect(names(sortEntries(input, "mtime", "asc"))).toEqual(["known.txt", "unknown.txt"]);
    expect(names(sortEntries(input, "mtime", "desc"))).toEqual(["known.txt", "unknown.txt"]);
  });

  it("ties (and all-unknown) fall back to name order", () => {
    const sorted = sortEntries(
      [make("b.txt", { mtimeSecs: null }), make("a.txt", { mtimeSecs: null })],
      "mtime",
      "asc",
    );
    expect(names(sorted)).toEqual(["a.txt", "b.txt"]);
  });
});
