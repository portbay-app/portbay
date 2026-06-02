import { describe, expect, it } from "vitest";

import {
  generateSql,
  type BuilderJoin,
  type BuilderSettings,
  type BuilderTable,
} from "../src/lib/components/databases/visualQuery";

const defaults: BuilderSettings = { where: "", orderBy: [], limit: null, distinct: false };

function table(id: string, name: string, cols: [string, boolean][]): BuilderTable {
  return {
    id,
    name,
    columns: cols.map(([name, selected]) => ({ name, selected, aggregate: "" })),
  };
}

describe("generateSql", () => {
  it("returns empty string with no tables", () => {
    expect(generateSql([], [], defaults)).toBe("");
  });

  it("selects chosen columns with table alias", () => {
    const sql = generateSql(
      [table("u", "users", [["id", true], ["name", true], ["email", false]])],
      [],
      defaults,
    );
    expect(sql).toBe("SELECT t1.id, t1.name\nFROM users AS t1");
  });

  it("falls back to * when nothing is selected", () => {
    const sql = generateSql([table("u", "users", [["id", false]])], [], defaults);
    expect(sql).toBe("SELECT *\nFROM users AS t1");
  });

  it("emits a JOIN between connected tables", () => {
    const tables = [
      table("o", "orders", [["id", true], ["user_id", false]]),
      table("u", "users", [["name", true]]),
    ];
    const joins: BuilderJoin[] = [
      { sourceId: "o", sourceColumn: "user_id", targetId: "u", targetColumn: "id", type: "LEFT" },
    ];
    const sql = generateSql(tables, joins, defaults);
    expect(sql).toBe(
      "SELECT t1.id, t2.name\nFROM orders AS t1\nLEFT JOIN users AS t2 ON t1.user_id = t2.id",
    );
  });

  it("adds GROUP BY when an aggregate is present", () => {
    const t = table("o", "orders", [["status", true], ["id", false]]);
    t.columns[1].selected = true;
    t.columns[1].aggregate = "COUNT";
    const sql = generateSql([t], [], defaults);
    expect(sql).toBe(
      "SELECT t1.status, COUNT(t1.id)\nFROM orders AS t1\nGROUP BY t1.status",
    );
  });

  it("applies DISTINCT, WHERE, ORDER BY and LIMIT", () => {
    const sql = generateSql([table("u", "users", [["id", true]])], [], {
      where: "t1.active = TRUE",
      orderBy: [{ expr: "t1.id", dir: "DESC" }],
      limit: 50,
      distinct: true,
    });
    expect(sql).toBe(
      "SELECT DISTINCT t1.id\nFROM users AS t1\nWHERE t1.active = TRUE\nORDER BY t1.id DESC\nLIMIT 50",
    );
  });

  it("cross-joins tables that have no connecting edge", () => {
    const tables = [
      table("a", "a", [["x", true]]),
      table("b", "b", [["y", true]]),
    ];
    const sql = generateSql(tables, [], defaults);
    expect(sql).toBe("SELECT t1.x, t2.y\nFROM a AS t1\nCROSS JOIN b AS t2");
  });
});
