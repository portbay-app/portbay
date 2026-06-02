/**
 * SQL generation for the visual query builder.
 *
 * The builder canvas is the source of truth: each table node carries its
 * selected columns + aggregates, and each edge is a JOIN between two columns.
 * This module turns that graph (plus the WHERE / ORDER BY / LIMIT settings)
 * into a SELECT statement. It is deliberately pure + dependency-free so it can
 * be unit-tested and reused.
 */

export type Aggregate = "" | "COUNT" | "COUNT_DISTINCT" | "SUM" | "AVG" | "MIN" | "MAX";
export type JoinType = "INNER" | "LEFT" | "RIGHT" | "FULL";

export interface BuilderColumn {
  name: string;
  selected: boolean;
  aggregate: Aggregate;
}

export interface BuilderTable {
  /** Stable node id (also the table key schema.name). */
  id: string;
  name: string;
  columns: BuilderColumn[];
}

export interface BuilderJoin {
  sourceId: string;
  sourceColumn: string;
  targetId: string;
  targetColumn: string;
  type: JoinType;
}

export interface BuilderSettings {
  where: string;
  orderBy: { expr: string; dir: "ASC" | "DESC" }[];
  limit: number | null;
  distinct: boolean;
}

const AGG_LABEL: Record<Exclude<Aggregate, "">, string> = {
  COUNT: "COUNT",
  COUNT_DISTINCT: "COUNT DISTINCT",
  SUM: "SUM",
  AVG: "AVG",
  MIN: "MIN",
  MAX: "MAX",
};

export const AGGREGATES: { value: Aggregate; label: string }[] = [
  { value: "", label: "—" },
  ...(Object.entries(AGG_LABEL) as [Exclude<Aggregate, "">, string][]).map(([value, label]) => ({
    value,
    label,
  })),
];

export const JOIN_TYPES: JoinType[] = ["INNER", "LEFT", "RIGHT", "FULL"];

/** Alias for a table by its position in the node list (t1, t2, …). */
function aliasFor(index: number): string {
  return `t${index + 1}`;
}

function aggregateExpr(agg: Aggregate, ref: string): string {
  switch (agg) {
    case "COUNT":
      return `COUNT(${ref})`;
    case "COUNT_DISTINCT":
      return `COUNT(DISTINCT ${ref})`;
    case "SUM":
      return `SUM(${ref})`;
    case "AVG":
      return `AVG(${ref})`;
    case "MIN":
      return `MIN(${ref})`;
    case "MAX":
      return `MAX(${ref})`;
    default:
      return ref;
  }
}

/**
 * Build a SELECT statement from the builder graph. Returns an empty string when
 * there are no tables yet.
 */
export function generateSql(
  tables: BuilderTable[],
  joins: BuilderJoin[],
  settings: BuilderSettings,
): string {
  if (tables.length === 0) return "";

  const aliasById = new Map<string, string>();
  tables.forEach((t, i) => aliasById.set(t.id, aliasFor(i)));

  // ─── SELECT list ───
  const selectParts: string[] = [];
  let hasAggregate = false;
  const groupByRefs: string[] = [];

  for (const t of tables) {
    const alias = aliasById.get(t.id)!;
    for (const col of t.columns) {
      if (!col.selected) continue;
      const ref = `${alias}.${col.name}`;
      if (col.aggregate) {
        hasAggregate = true;
        selectParts.push(aggregateExpr(col.aggregate, ref));
      } else {
        selectParts.push(ref);
        groupByRefs.push(ref);
      }
    }
  }
  const selectList = selectParts.length > 0 ? selectParts.join(", ") : "*";

  // ─── FROM + JOINs ───
  // First table seeds FROM; JOINs pull the rest in as their edges connect them.
  const [first, ...rest] = tables;
  const included = new Set<string>([first.id]);
  const fromParts = [`FROM ${first.name} AS ${aliasById.get(first.id)}`];

  const pendingJoins = [...joins];
  let progress = true;
  while (progress && included.size < tables.length) {
    progress = false;
    for (let i = 0; i < pendingJoins.length; i += 1) {
      const j = pendingJoins[i];
      const srcIn = included.has(j.sourceId);
      const tgtIn = included.has(j.targetId);
      if (srcIn === tgtIn) continue; // both in, or neither — skip for now
      const newId = srcIn ? j.targetId : j.sourceId;
      const newTable = tables.find((t) => t.id === newId);
      if (!newTable) {
        pendingJoins.splice(i, 1);
        i -= 1;
        continue;
      }
      const newAlias = aliasById.get(newId)!;
      const srcAlias = aliasById.get(j.sourceId)!;
      const tgtAlias = aliasById.get(j.targetId)!;
      fromParts.push(
        `${j.type} JOIN ${newTable.name} AS ${newAlias} ` +
          `ON ${srcAlias}.${j.sourceColumn} = ${tgtAlias}.${j.targetColumn}`,
      );
      included.add(newId);
      pendingJoins.splice(i, 1);
      progress = true;
      i -= 1;
    }
  }

  // Any table not reachable via a JOIN edge is cross-joined.
  for (const t of rest) {
    if (included.has(t.id)) continue;
    fromParts.push(`CROSS JOIN ${t.name} AS ${aliasById.get(t.id)}`);
    included.add(t.id);
  }

  // ─── Clauses ───
  const clauses: string[] = [];
  clauses.push(`SELECT ${settings.distinct ? "DISTINCT " : ""}${selectList}`);
  clauses.push(fromParts.join("\n"));

  const where = settings.where.trim();
  if (where) clauses.push(`WHERE ${where}`);

  if (hasAggregate && groupByRefs.length > 0) {
    clauses.push(`GROUP BY ${groupByRefs.join(", ")}`);
  }

  const order = settings.orderBy.filter((o) => o.expr.trim());
  if (order.length > 0) {
    clauses.push(`ORDER BY ${order.map((o) => `${o.expr} ${o.dir}`).join(", ")}`);
  }

  if (settings.limit != null && settings.limit > 0) {
    clauses.push(`LIMIT ${Math.floor(settings.limit)}`);
  }

  return clauses.join("\n");
}
