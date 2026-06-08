/**
 * sftpMove — the drag-to-move planner. Pins the safety rules: a folder can
 * never move into itself or its own subtree, drops into the current parent
 * are no-ops, and multi-select drags translate to one rename per entry.
 */
import { describe, expect, it } from "vitest";

import { planMoves, splitMoveConflicts } from "$lib/sftpMove";

describe("planMoves", () => {
  it("moves a file into a folder", () => {
    expect(planMoves(["/srv/app.php"], "/srv/public")).toEqual([
      { from: "/srv/app.php", to: "/srv/public/app.php" },
    ]);
  });

  it("moves several selected entries at once, preserving order", () => {
    expect(planMoves(["/srv/a.txt", "/srv/b.txt", "/srv/logs"], "/srv/public")).toEqual([
      { from: "/srv/a.txt", to: "/srv/public/a.txt" },
      { from: "/srv/b.txt", to: "/srv/public/b.txt" },
      { from: "/srv/logs", to: "/srv/public/logs" },
    ]);
  });

  it("keeps the basename when moving from a deep source", () => {
    expect(planMoves(["/var/www/site/index.html"], "/tmp")).toEqual([
      { from: "/var/www/site/index.html", to: "/tmp/index.html" },
    ]);
  });

  it("moves into the root directory cleanly", () => {
    expect(planMoves(["/srv/a.txt"], "/")).toEqual([{ from: "/srv/a.txt", to: "/a.txt" }]);
  });

  it("skips a folder dropped onto itself", () => {
    expect(planMoves(["/srv/public"], "/srv/public")).toEqual([]);
  });

  it("never moves a folder into its own subtree", () => {
    expect(planMoves(["/srv/public"], "/srv/public/assets")).toEqual([]);
    expect(planMoves(["/srv/public"], "/srv/public/a/b/c")).toEqual([]);
  });

  it("does not confuse sibling names sharing a prefix", () => {
    // /srv/pub is NOT an ancestor of /srv/public — the move must go through.
    expect(planMoves(["/srv/pub"], "/srv/public")).toEqual([
      { from: "/srv/pub", to: "/srv/public/pub" },
    ]);
  });

  it("skips entries already directly inside the destination", () => {
    expect(planMoves(["/srv/public/app.php"], "/srv/public")).toEqual([]);
  });

  it("still moves entries from a SUBfolder of the destination upward", () => {
    expect(planMoves(["/srv/public/assets/x.png"], "/srv/public")).toEqual([
      { from: "/srv/public/assets/x.png", to: "/srv/public/x.png" },
    ]);
  });

  it("mixed drag: applies the rules per entry", () => {
    const plan = planMoves(
      ["/srv/public", "/srv/notes.md", "/srv/public/assets/logo.png"],
      "/srv/public",
    );
    expect(plan).toEqual([
      // the folder dropped onto itself is skipped; the other two move
      { from: "/srv/notes.md", to: "/srv/public/notes.md" },
      { from: "/srv/public/assets/logo.png", to: "/srv/public/logo.png" },
    ]);
  });

  it("dedupes repeated sources", () => {
    expect(planMoves(["/srv/a.txt", "/srv/a.txt"], "/srv/public")).toHaveLength(1);
  });

  it("ignores trailing slashes on sources", () => {
    expect(planMoves(["/srv/logs/"], "/srv/public")).toEqual([
      { from: "/srv/logs", to: "/srv/public/logs" },
    ]);
  });

  it("never moves the filesystem root", () => {
    expect(planMoves(["/"], "/srv")).toEqual([]);
  });
});

describe("splitMoveConflicts", () => {
  const plan = [
    { from: "/srv/a.txt", to: "/srv/public/a.txt" },
    { from: "/srv/b.txt", to: "/srv/public/b.txt" },
    { from: "/srv/logs", to: "/srv/public/logs" },
  ];

  it("partitions by destination existence, preserving order", () => {
    const { clean, conflicted } = splitMoveConflicts(
      plan,
      new Set(["/srv/public/b.txt", "/srv/public/logs"]),
    );
    expect(clean).toEqual([{ from: "/srv/a.txt", to: "/srv/public/a.txt" }]);
    expect(conflicted.map((m) => m.to)).toEqual(["/srv/public/b.txt", "/srv/public/logs"]);
  });

  it("everything clean when nothing exists", () => {
    const { clean, conflicted } = splitMoveConflicts(plan, new Set());
    expect(clean).toHaveLength(3);
    expect(conflicted).toHaveLength(0);
  });

  it("matches on the destination, not the source", () => {
    const { conflicted } = splitMoveConflicts(plan, new Set(["/srv/a.txt"]));
    expect(conflicted).toHaveLength(0);
  });
});
