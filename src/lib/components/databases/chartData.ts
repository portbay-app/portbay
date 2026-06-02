import type { DbClientRows } from "$lib/types/databases";

export type ChartType = "bar" | "line" | "pie";

export interface ChartConfig {
  type: ChartType;
  labelColumn: string;
  valueColumns: string[];
}

export interface ChartRow {
  label: string;
  [colName: string]: string | number;
}

/** Whether there is enough data to render any chart at all. */
export function canRenderChart(rows: DbClientRows | null): boolean {
  if (!rows) return false;
  return rows.columns.length >= 2 && rows.rows.length > 0;
}

/** All column names where at least one row has a numeric value. */
export function getNumericColumns(rows: DbClientRows): string[] {
  return rows.columns
    .map((col, i) => ({
      name: col.name,
      hasNumeric: rows.rows.some((row) => typeof row[i] === "number"),
    }))
    .filter((c) => c.hasNumeric)
    .map((c) => c.name);
}

/** All column names where at least one row has a string value. */
export function getLabelColumns(rows: DbClientRows): string[] {
  return rows.columns
    .map((col, i) => ({
      name: col.name,
      hasString: rows.rows.some((row) => typeof row[i] === "string"),
    }))
    .filter((c) => c.hasString)
    .map((c) => c.name);
}

/** Build a sensible default config. Returns null when no numeric or label column is available. */
export function buildDefaultConfig(rows: DbClientRows): ChartConfig | null {
  const numericCols = getNumericColumns(rows);
  const labelCols = getLabelColumns(rows);
  if (numericCols.length === 0 || labelCols.length === 0) return null;
  return {
    type: "bar",
    labelColumn: labelCols[0],
    valueColumns: [numericCols[0]],
  };
}

/** Transform DbClientRows into a flat array of objects suitable for layerchart. */
export function transformToChartData(
  rows: DbClientRows,
  config: ChartConfig,
): ChartRow[] {
  const labelIdx = rows.columns.findIndex((c) => c.name === config.labelColumn);
  const valueIdxMap: Record<string, number> = {};
  for (const colName of config.valueColumns) {
    const idx = rows.columns.findIndex((c) => c.name === colName);
    if (idx !== -1) valueIdxMap[colName] = idx;
  }

  return rows.rows.map((row) => {
    const entry: ChartRow = { label: String(row[labelIdx] ?? "") };
    for (const colName of config.valueColumns) {
      const idx = valueIdxMap[colName];
      if (idx === undefined) {
        entry[colName] = 0;
      } else {
        const v = row[idx];
        entry[colName] = typeof v === "number" ? v : Number(v) || 0;
      }
    }
    return entry;
  });
}

export const PALETTE: readonly string[] = [
  "#3b82f6",
  "#10b981",
  "#f59e0b",
  "#ef4444",
  "#8b5cf6",
  "#ec4899",
  "#06b6d4",
  "#f97316",
] as const;

export function paletteColor(i: number): string {
  return PALETTE[i % PALETTE.length];
}
