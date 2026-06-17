/**
 * Nav-order core rules — persistence parsing/migration, order+visibility
 * reconciliation, and the visible-reorder merge. Pure logic from
 * `$lib/stores/navOrderCore` (the Svelte store is a thin runes wrapper).
 */
import { describe, expect, it } from "vitest";

import {
  NAV_ITEMS,
  mergeVisibleOrder,
  parsePersisted,
  reconcile,
  serialize,
  type NavItem,
} from "$lib/stores/navOrderCore";

/** Small synthetic catalog so each rule is tested in isolation. */
const CATALOG: NavItem[] = [
  { id: "a", href: "/a", icon: "bot", label: "A" },
  { id: "b", href: "/b", icon: "link", label: "B" },
  { id: "c", href: "/c", icon: "globe", label: "C" },
  { id: "locked", href: "/locked", icon: "settings", label: "Locked", hideable: false },
  { id: "shy", href: "/shy", icon: "camera", label: "Shy", defaultHidden: true },
];

const ids = (items: NavItem[]) => items.map((it) => it.id);

describe("parsePersisted", () => {
  it("returns null for missing or corrupt values", () => {
    expect(parsePersisted(null)).toBeNull();
    expect(parsePersisted("")).toBeNull();
    expect(parsePersisted("not json {")).toBeNull();
    expect(parsePersisted('"a string"')).toBeNull();
    expect(parsePersisted("42")).toBeNull();
    expect(parsePersisted('{"hidden":["a"]}')).toBeNull(); // no order array
  });

  it("upgrades a v1 array to the v2 shape with nothing hidden", () => {
    expect(parsePersisted('["b","a"]')).toEqual({ order: ["b", "a"], hidden: [] });
  });

  it("drops non-string entries from a v1 array", () => {
    expect(parsePersisted('["b",3,null,"a"]')).toEqual({ order: ["b", "a"], hidden: [] });
  });

  it("parses the v2 object and filters non-strings", () => {
    expect(parsePersisted('{"order":["a","b",7],"hidden":["b",false]}')).toEqual({
      order: ["a", "b"],
      hidden: ["b"],
    });
  });

  it("tolerates a v2 object without hidden", () => {
    expect(parsePersisted('{"order":["a"]}')).toEqual({ order: ["a"], hidden: [] });
  });
});

describe("reconcile", () => {
  it("falls back to the canonical order (and defaultHidden) on a fresh profile", () => {
    const { items, hidden } = reconcile(null, CATALOG);
    expect(ids(items)).toEqual(["a", "b", "c", "locked", "shy"]);
    expect([...hidden]).toEqual(["shy"]);
  });

  it("keeps a saved order, drops unknown ids, dedupes", () => {
    const { items } = reconcile(
      { order: ["c", "ghost", "a", "c"], hidden: [] },
      CATALOG,
    );
    expect(ids(items)).toEqual(["c", "a", "b", "locked", "shy"]);
  });

  it("appends newly-shipped canonical items instead of dropping them", () => {
    // A save written before "c" and "shy" existed.
    const { items } = reconcile({ order: ["b", "a", "locked"], hidden: [] }, CATALOG);
    expect(ids(items)).toEqual(["b", "a", "locked", "c", "shy"]);
  });

  it("honors saved hidden ids only for known, hideable items", () => {
    const { hidden } = reconcile(
      { order: ids(CATALOG), hidden: ["b", "locked", "ghost"] },
      CATALOG,
    );
    expect(hidden.has("b")).toBe(true);
    expect(hidden.has("locked")).toBe(false); // never hideable
    expect(hidden.has("ghost")).toBe(false);
  });

  it("a defaultHidden item the save has seen keeps the user's explicit choice", () => {
    // "shy" is in the saved order and NOT in hidden → the user pinned it.
    const { hidden } = reconcile({ order: ids(CATALOG), hidden: [] }, CATALOG);
    expect(hidden.has("shy")).toBe(false);
  });

  it("a defaultHidden item absent from the saved order starts hidden (v1 upgrade)", () => {
    const saved = parsePersisted('["a","b","c","locked"]');
    const { hidden } = reconcile(saved, CATALOG);
    expect(hidden.has("shy")).toBe(true);
  });
});

describe("app-backed items (My Apps governs visibility)", () => {
  // A destination that is also a launcher tool — its sidebar visibility lives
  // in the myApps store, not the pin/hide set.
  const APP_CATALOG: NavItem[] = [
    { id: "a", href: "/a", icon: "bot", label: "A" },
    { id: "cap", href: "/cap", icon: "camera", label: "Cap", appId: "cap" },
  ];

  it("never adds an app-backed item to the hidden set, even if saved as hidden", () => {
    const { hidden } = reconcile({ order: ["a", "cap"], hidden: ["cap"] }, APP_CATALOG);
    expect(hidden.has("cap")).toBe(false);
  });

  it("keeps app-backed items in the order so they stay reorderable", () => {
    const { items } = reconcile({ order: ["cap", "a"], hidden: [] }, APP_CATALOG);
    expect(ids(items)).toEqual(["cap", "a"]);
  });

  it("never marks an app-backed item hidden on a fresh profile", () => {
    const { hidden } = reconcile(null, APP_CATALOG);
    expect(hidden.has("cap")).toBe(false);
  });
});

describe("mergeVisibleOrder", () => {
  const byId = new Map(CATALOG.map((it) => [it.id, it]));
  const pick = (...sel: string[]) => sel.map((id) => byId.get(id)!);

  it("reorders visible items while hidden items keep their slots", () => {
    const current = pick("a", "b", "c", "locked", "shy");
    const hidden = new Set(["b", "shy"]);
    // Visible list is [a, c, locked]; user drags locked to the front.
    const next = mergeVisibleOrder(current, pick("locked", "a", "c"), hidden);
    expect(ids(next)).toEqual(["locked", "b", "a", "c", "shy"]);
  });

  it("ignores ids that are not part of the current order", () => {
    const current = pick("a", "b");
    const rogue: NavItem = { id: "rogue", href: "/r", icon: "bot", label: "R" };
    const next = mergeVisibleOrder(current, [rogue, ...pick("b", "a")], new Set());
    expect(ids(next)).toEqual(["b", "a"]);
  });

  it("is the identity when nothing is hidden and the order is unchanged", () => {
    const current = pick("a", "b", "c");
    expect(ids(mergeVisibleOrder(current, current, new Set()))).toEqual(["a", "b", "c"]);
  });
});

describe("serialize round-trip", () => {
  it("persists order + hidden and survives reconcile", () => {
    const { items } = reconcile({ order: ["c", "a", "b"], hidden: [] }, CATALOG);
    const hidden = new Set(["b"]);
    const raw = serialize(items, hidden);
    const back = reconcile(parsePersisted(raw), CATALOG);
    expect(ids(back.items)).toEqual(ids(items));
    expect([...back.hidden]).toEqual(["b"]);
  });

  it("does not persist hidden ids that left the catalog", () => {
    const items = pickFrom(CATALOG, "a", "b");
    const raw = serialize(items, new Set(["b", "gone"]));
    expect(JSON.parse(raw)).toEqual({ order: ["a", "b"], hidden: ["b"] });
  });
});

describe("real catalog invariants", () => {
  it("Settings and Integrations are present and never hideable", () => {
    const settings = NAV_ITEMS.find((it) => it.id === "settings");
    const integrations = NAV_ITEMS.find((it) => it.id === "integrations");
    expect(settings?.hideable).toBe(false);
    expect(integrations?.hideable).toBe(false);
    expect(integrations?.href).toBe("/integrations");
  });

  it("no current item ships hidden (no surprise removals on upgrade)", () => {
    const { hidden } = reconcile(null);
    expect(hidden.size).toBe(0);
  });

  it("ids are unique", () => {
    expect(new Set(ids(NAV_ITEMS)).size).toBe(NAV_ITEMS.length);
  });

  it("the launcher tools (capture, inspector, sandbox) are app-backed", () => {
    for (const id of ["capture", "inspector", "sandbox"]) {
      expect(NAV_ITEMS.find((it) => it.id === id)?.appId).toBe(id);
    }
  });
});

function pickFrom(catalog: NavItem[], ...sel: string[]): NavItem[] {
  const byId = new Map(catalog.map((it) => [it.id, it]));
  return sel.map((id) => byId.get(id)!);
}
