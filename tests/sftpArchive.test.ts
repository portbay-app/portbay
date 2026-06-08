/**
 * sftpArchive — archive detection + the server-side extraction commands.
 * Pins the kind table (compound extensions before single-suffix ones), the
 * default extract-folder stem, and the shell-quoting that keeps hostile file
 * names from breaking out of the exec command.
 */
import { describe, expect, it, vi } from "vitest";

// extractArchive's runtime deps (exec channel + toast bus) aren't under test —
// stub them so importing the module is side-effect free under node. $lib/ipc
// is reached via $lib/sftp's helpers and drags in SvelteKit's $app modules.
vi.mock("$lib/sshExec", () => ({ sshExecRun: vi.fn() }));
vi.mock("$lib/stores/errors.svelte", () => ({ errorBus: {} }));
vi.mock("$lib/ipc", () => ({ invokeQuiet: vi.fn(), safeInvoke: vi.fn() }));

import {
  archiveKind,
  archiveStem,
  compressCommand,
  extractCommand,
  isArchive,
} from "$lib/sftpArchive";

describe("archiveKind / isArchive", () => {
  it.each([
    ["site.zip", "zip"],
    ["site.tar.gz", "tgz"],
    ["site.tgz", "tgz"],
    ["site.tar.bz2", "tbz2"],
    ["site.tbz2", "tbz2"],
    ["site.tbz", "tbz2"],
    ["site.tar.xz", "txz"],
    ["site.txz", "txz"],
    ["site.tar", "tar"],
    ["site.sql.gz", "gz"],
    ["site.bz2", "bz2"],
    ["site.xz", "xz"],
    ["site.7z", "7z"],
    ["site.rar", "rar"],
  ] as const)("%s → %s", (name, kind) => {
    expect(archiveKind(name)).toBe(kind);
    expect(isArchive(name)).toBe(true);
  });

  it("is case-insensitive", () => {
    expect(archiveKind("BACKUP.ZIP")).toBe("zip");
    expect(archiveKind("Backup.Tar.Gz")).toBe("tgz");
  });

  it("prefers compound extensions over their single-suffix cousins", () => {
    // .tar.gz must classify as tgz (tar -xf), never plain gz (gzip -dc).
    expect(archiveKind("a.tar.gz")).toBe("tgz");
    expect(archiveKind("a.tar.bz2")).toBe("tbz2");
    expect(archiveKind("a.tar.xz")).toBe("txz");
  });

  it.each(["readme.txt", "archive", "zipfile.txt", "a.gz.txt", "tar", ".env"])(
    "non-archive: %s",
    (name) => {
      expect(archiveKind(name)).toBeNull();
      expect(isArchive(name)).toBe(false);
    },
  );
});

describe("archiveStem", () => {
  it.each([
    ["site.zip", "site"],
    ["site.tar.gz", "site"],
    ["site.tgz", "site"],
    ["db.sql.gz", "db.sql"],
    ["release-1.2.3.tar.xz", "release-1.2.3"],
  ])("%s → %s", (name, stem) => {
    expect(archiveStem(name)).toBe(stem);
  });

  it("returns the name unchanged for non-archives", () => {
    expect(archiveStem("readme.txt")).toBe("readme.txt");
  });

  it("never returns an empty stem", () => {
    expect(archiveStem(".zip")).toBe(".zip");
  });
});

describe("extractCommand", () => {
  it("always creates the destination first", () => {
    for (const kind of ["zip", "tgz", "gz", "7z", "rar"] as const) {
      expect(extractCommand("/srv/a.bin", "/srv/out", kind)).toContain("mkdir -p '/srv/out'");
    }
  });

  it("uses the right tool per kind", () => {
    expect(extractCommand("/s/a.zip", "/s/out", "zip")).toContain("unzip -o");
    expect(extractCommand("/s/a.tar.gz", "/s/out", "tgz")).toContain("tar -xf");
    expect(extractCommand("/s/a.tar", "/s/out", "tar")).toContain("tar -xf");
    expect(extractCommand("/s/a.7z", "/s/out", "7z")).toContain("7z x -y");
    expect(extractCommand("/s/a.rar", "/s/out", "rar")).toContain("unrar x -o+");
  });

  it("decompresses single-file archives to <dest>/<stem>", () => {
    const cmd = extractCommand("/srv/db.sql.gz", "/srv/out", "gz");
    expect(cmd).toContain("gzip -dc '/srv/db.sql.gz'");
    expect(cmd).toContain("> '/srv/out/db.sql'");
  });

  it("single-quote-escapes hostile names (no shell breakout)", () => {
    const cmd = extractCommand("/srv/pwn'; rm -rf $HOME; '.zip", "/srv/o'ut", "zip");
    // Every single quote in the inputs must appear as the '\'' escape…
    expect(cmd).toContain("'/srv/pwn'\\''; rm -rf $HOME; '\\''.zip'");
    expect(cmd).toContain("'/srv/o'\\''ut'");
    // …so the dangerous payload never sits in an unquoted context.
    expect(cmd).not.toMatch(/[^\\']'; rm -rf/);
  });

  it("quotes spaces in paths", () => {
    const cmd = extractCommand("/srv/my site.zip", "/srv/my out", "zip");
    expect(cmd).toContain("'/srv/my site.zip'");
    expect(cmd).toContain("'/srv/my out'");
  });
});

describe("compressCommand", () => {
  it("zips relative names from the base directory", () => {
    expect(compressCommand("/srv/www", ["public", "app.php"], "site.zip")).toBe(
      "cd '/srv/www' && zip -r -q -y 'site.zip' 'public' 'app.php'",
    );
  });

  it("works from the root directory", () => {
    expect(compressCommand("/", ["etc"], "etc.zip")).toBe(
      "cd '/' && zip -r -q -y 'etc.zip' 'etc'",
    );
  });

  it("single-quote-escapes hostile names (no shell breakout)", () => {
    const cmd = compressCommand("/srv/o'ut", ["pwn'; rm -rf $HOME; '"], "a'b.zip");
    expect(cmd).toContain("'/srv/o'\\''ut'");
    expect(cmd).toContain("'pwn'\\''; rm -rf $HOME; '\\'''");
    expect(cmd).toContain("'a'\\''b.zip'");
    expect(cmd).not.toMatch(/[^\\']'; rm -rf/);
  });

  it("quotes spaces in names", () => {
    expect(compressCommand("/srv/my www", ["my folder"], "my archive.zip")).toBe(
      "cd '/srv/my www' && zip -r -q -y 'my archive.zip' 'my folder'",
    );
  });
});
