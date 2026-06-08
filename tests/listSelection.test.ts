/**
 * listSelection — the shared multi-select model behind the SFTP browser rows
 * and the Explorer tree. These tests pin the Finder/VS Code interaction
 * semantics, including the mixed row-click + checkbox sequences that
 * previously desynced the rendered checkboxes from the real selection.
 */
import { describe, expect, it } from "vitest";

import {
  EMPTY_SELECTION,
  plainSelect,
  pruneSelection,
  rangeSelect,
  toggleSelect,
  toggleSelectAll,
  type Selection,
} from "$lib/listSelection";

const ORDER = ["/a", "/b", "/c", "/d", "/e"];

const sel = (paths: string[], anchor: string | null = null): Selection => ({
  paths: new Set(paths),
  anchor,
});

const pathsOf = (s: Selection) => [...s.paths].sort();

describe("plainSelect", () => {
  it("selects exactly the clicked row and anchors on it", () => {
    const s = plainSelect("/b");
    expect(pathsOf(s)).toEqual(["/b"]);
    expect(s.anchor).toBe("/b");
  });

  it("replaces a previous multi-selection", () => {
    // (callers pass no previous state — plain click always starts over)
    const s = plainSelect("/d");
    expect(pathsOf(s)).toEqual(["/d"]);
  });
});

describe("toggleSelect (⌘-click / checkbox)", () => {
  it("row-click a folder, then checkbox another → BOTH selected", () => {
    // The reported regression: click .well-known, check cgi-bin — the footer
    // said "2 selected" but the first checkbox rendered unchecked.
    let s = plainSelect("/.well-known");
    s = toggleSelect(s, "/cgi-bin");
    expect(pathsOf(s)).toEqual(["/.well-known", "/cgi-bin"]);
    // Both rows must read as selected — the checkboxes render from this set.
    expect(s.paths.has("/.well-known")).toBe(true);
    expect(s.paths.has("/cgi-bin")).toBe(true);
  });

  it("adds without dropping the rest, and moves the anchor", () => {
    const s = toggleSelect(sel(["/a", "/b"], "/a"), "/d");
    expect(pathsOf(s)).toEqual(["/a", "/b", "/d"]);
    expect(s.anchor).toBe("/d");
  });

  it("toggles an already-selected row off", () => {
    const s = toggleSelect(sel(["/a", "/b"], "/a"), "/b");
    expect(pathsOf(s)).toEqual(["/a"]);
    expect(s.anchor).toBe("/b");
  });

  it("checkbox sequences are order-independent", () => {
    let s = EMPTY_SELECTION;
    s = toggleSelect(s, "/c");
    s = toggleSelect(s, "/a");
    s = toggleSelect(s, "/e");
    s = toggleSelect(s, "/c"); // un-check the first again
    expect(pathsOf(s)).toEqual(["/a", "/e"]);
  });
});

describe("rangeSelect (⇧-click)", () => {
  it("selects anchor→row inclusive, replacing the old selection", () => {
    const s = rangeSelect(sel(["/e"], "/b"), ORDER, "/d", false);
    expect(pathsOf(s)).toEqual(["/b", "/c", "/d"]);
  });

  it("works upward (row above the anchor)", () => {
    const s = rangeSelect(sel([], "/d"), ORDER, "/a", false);
    expect(pathsOf(s)).toEqual(["/a", "/b", "/c", "/d"]);
  });

  it("keeps the existing selection when additive (⌘⇧ / ⇧-checkbox)", () => {
    const s = rangeSelect(sel(["/a"], "/c"), ORDER, "/e", true);
    expect(pathsOf(s)).toEqual(["/a", "/c", "/d", "/e"]);
  });

  it("keeps the anchor so successive ⇧-clicks re-pivot", () => {
    let s = plainSelect("/b");
    s = rangeSelect(s, ORDER, "/e", false);
    expect(pathsOf(s)).toEqual(["/b", "/c", "/d", "/e"]);
    s = rangeSelect(s, ORDER, "/c", false); // shrink back toward the anchor
    expect(pathsOf(s)).toEqual(["/b", "/c"]);
    expect(s.anchor).toBe("/b");
  });

  it("degrades to a single select when there is no anchor", () => {
    const s = rangeSelect(sel([], null), ORDER, "/c", false);
    expect(pathsOf(s)).toEqual(["/c"]);
    expect(s.anchor).toBe("/c");
  });

  it("degrades additively when there is no anchor and ⌘ is held", () => {
    const s = rangeSelect(sel(["/a"], null), ORDER, "/c", true);
    expect(pathsOf(s)).toEqual(["/a", "/c"]);
  });

  it("treats an anchor that filtered out of the listing as absent", () => {
    // The anchor row was hidden by the search filter — order no longer has it.
    const filtered = ["/a", "/c", "/e"];
    const s = rangeSelect(sel(["/b"], "/b"), filtered, "/e", false);
    expect(pathsOf(s)).toEqual(["/e"]);
  });

  it("ignores a target that isn't in the listing", () => {
    const before = sel(["/a"], "/a");
    const s = rangeSelect(before, ORDER, "/zzz", false);
    expect(s).toBe(before);
  });

  it("ranges over the *visible* order during a search filter", () => {
    const filtered = ["/a", "/d", "/e"]; // /b and /c hidden by the filter
    const s = rangeSelect(sel([], "/a"), filtered, "/e", false);
    expect(pathsOf(s)).toEqual(["/a", "/d", "/e"]); // hidden rows not swept in
  });
});

describe("toggleSelectAll (header checkbox)", () => {
  it("selects everything visible from nothing", () => {
    const s = toggleSelectAll(EMPTY_SELECTION, ORDER);
    expect(pathsOf(s)).toEqual([...ORDER].sort());
  });

  it("completes a partial selection (does not clear it)", () => {
    const s = toggleSelectAll(sel(["/b"], "/b"), ORDER);
    expect(pathsOf(s)).toEqual([...ORDER].sort());
  });

  it("clears when everything is already selected", () => {
    const s = toggleSelectAll(sel(ORDER, "/a"), ORDER);
    expect(s.paths.size).toBe(0);
  });

  it("does nothing on an empty listing", () => {
    const s = toggleSelectAll(EMPTY_SELECTION, []);
    expect(s.paths.size).toBe(0);
  });
});

describe("pruneSelection (after refresh / delete)", () => {
  it("drops paths that no longer exist", () => {
    const s = pruneSelection(sel(["/a", "/b", "/c"], "/a"), new Set(["/a", "/c"]));
    expect(pathsOf(s)).toEqual(["/a", "/c"]);
    expect(s.anchor).toBe("/a");
  });

  it("clears the anchor when its row is gone", () => {
    const s = pruneSelection(sel(["/a", "/b"], "/b"), new Set(["/a"]));
    expect(pathsOf(s)).toEqual(["/a"]);
    expect(s.anchor).toBeNull();
  });

  it("returns the same state when nothing changed", () => {
    const before = sel(["/a"], "/a");
    expect(pruneSelection(before, new Set(["/a", "/b"]))).toBe(before);
  });
});

describe("end-to-end interaction sequences", () => {
  it("click, ⌘-click, ⇧-click, checkbox off — stays consistent throughout", () => {
    let s = plainSelect("/a"); //               [a]      anchor a
    s = toggleSelect(s, "/c"); //               [a c]    anchor c
    s = rangeSelect(s, ORDER, "/e", true); //   [a c d e] anchor c
    expect(pathsOf(s)).toEqual(["/a", "/c", "/d", "/e"]);
    s = toggleSelect(s, "/d"); //               [a c e]  anchor d
    expect(pathsOf(s)).toEqual(["/a", "/c", "/e"]);
    s = plainSelect("/b"); //                   [b]
    expect(pathsOf(s)).toEqual(["/b"]);
    expect(s.anchor).toBe("/b");
  });

  it("delete-key flow: select two, prune after the refresh removes them", () => {
    let s = plainSelect("/a");
    s = toggleSelect(s, "/b");
    s = pruneSelection(s, new Set(["/c", "/d", "/e"])); // both deleted
    expect(s.paths.size).toBe(0);
    expect(s.anchor).toBeNull();
  });
});
