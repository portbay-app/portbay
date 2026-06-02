/**
 * terminalHighlightEngine — paints {@link ./terminalHighlight} rule matches onto
 * a live xterm.js terminal as decorations.
 *
 * Cheap by construction: it only ever scans the **visible viewport** (not the
 * whole scrollback), repaints are coalesced to one per animation frame, and a
 * decoration is reused across frames as long as the same match stays on screen
 * — only genuinely new spans allocate, only scrolled-away ones are disposed. So
 * idle output costs nothing and scrolling costs one diff pass.
 *
 * Column accuracy: string match offsets are mapped to real terminal cells via
 * the buffer's per-cell API, so wide glyphs (CJK/emoji, width 2) and combining
 * marks don't shift a highlight off its text.
 *
 * Each match is a DOM overlay behind the text (`layer: 'bottom'`), styled per
 * render mode, so it stays renderer-agnostic (no webgl/canvas dependency).
 */
import type {
  IBufferCell,
  IBufferLine,
  IDecoration,
  IDisposable,
  IMarker,
  Terminal,
} from "@xterm/xterm";

import {
  matchLine,
  type CompiledRule,
  type HighlightRenderMode,
} from "$lib/ssh/terminalHighlight";

const BACKGROUND_ALPHA = 0.32;

interface Painted {
  decoration: IDecoration;
  marker: IMarker;
}

export interface HighlightEngine {
  /** Recompute the visible viewport (call after the rule set changes). */
  refresh: () => void;
  dispose: () => void;
}

/** `#RGB` / `#RRGGBB` → `rgba(r, g, b, alpha)`, falling back to the input. */
function toRgba(hex: string, alpha: number): string {
  let h = hex.trim().replace(/^#/, "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  if (!/^[0-9a-fA-F]{6}$/.test(h)) return hex;
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Style an overlay element for a render mode. */
function styleOverlay(el: HTMLElement, color: string, mode: HighlightRenderMode) {
  el.style.pointerEvents = "none";
  el.style.boxSizing = "border-box";
  if (mode === "underline") {
    el.style.borderBottom = `2px solid ${color}`;
  } else if (mode === "outline") {
    el.style.border = `1px solid ${color}`;
    el.style.borderRadius = "2px";
  } else {
    el.style.backgroundColor = toRgba(color, BACKGROUND_ALPHA);
    el.style.borderRadius = "2px";
  }
}

/**
 * Read one buffer row as a string plus a map from string index → starting cell
 * column. `colForIndex` has one trailing sentinel entry (the column just past
 * the final character) so a match's end column is `colForIndex[end]`. Trailing
 * blank cells are trimmed so patterns don't match padding.
 */
function readRow(
  line: IBufferLine,
  cols: number,
  reuse: { cell: IBufferCell | undefined },
): { text: string; colForIndex: number[] } {
  let text = "";
  const colForIndex: number[] = [];
  let col = 0;
  while (col < cols) {
    // Reuse one cell object across the whole pass to avoid per-cell allocation.
    const cell = line.getCell(col, reuse.cell);
    if (!cell) {
      col += 1;
      continue;
    }
    reuse.cell = cell;
    const width = cell.getWidth();
    if (width === 0) {
      // Combining mark already folded into the preceding base cell.
      col += 1;
      continue;
    }
    const chars = cell.getChars();
    const s = chars.length > 0 ? chars : " ";
    for (let k = 0; k < s.length; k++) colForIndex.push(col);
    text += s;
    col += width;
  }
  colForIndex.push(col); // sentinel: column after the last character

  // Trim trailing whitespace (and its column entries, keeping the sentinel).
  let end = text.length;
  while (end > 0 && text.charCodeAt(end - 1) === 32) end--;
  return { text: text.slice(0, end), colForIndex: colForIndex.slice(0, end + 1) };
}

/**
 * Attach a highlight engine to `term`. `getRules` is read on every repaint so
 * the caller can swap rules live without re-attaching. Returns a handle to
 * refresh (on rule change) and dispose (on teardown).
 */
export function attachHighlightEngine(
  term: Terminal,
  getRules: () => CompiledRule[],
): HighlightEngine {
  const active = new Map<string, Painted>();
  let frame = 0;
  let disposed = false;

  function repaint() {
    if (disposed) return;
    const rules = getRules();
    const buffer = term.buffer.active;
    const top = buffer.viewportY;
    const bottom = Math.min(buffer.length - 1, top + term.rows - 1);
    const desired = new Set<string>();

    if (rules.length > 0) {
      const reuse: { cell: IBufferCell | undefined } = { cell: undefined };
      for (let abs = top; abs <= bottom; abs++) {
        const line = buffer.getLine(abs);
        if (!line) continue;
        const { text, colForIndex } = readRow(line, term.cols, reuse);
        if (!text) continue;

        for (const m of matchLine(text, rules)) {
          const startCol = colForIndex[m.start];
          const endCol = colForIndex[m.end];
          if (startCol === undefined || endCol === undefined) continue;
          const width = endCol - startCol;
          if (width <= 0) continue;

          const key = `${abs}:${m.ruleId}:${startCol}:${width}:${m.renderMode}`;
          desired.add(key);
          if (active.has(key)) continue;

          // Markers anchor relative to the cursor line at creation, then track
          // their buffer line as it scrolls; offset can be negative (scrollback).
          const offset = abs - (buffer.baseY + buffer.cursorY);
          const marker = term.registerMarker(offset);
          if (!marker) continue;
          const decoration = term.registerDecoration({
            marker,
            x: startCol,
            width,
            layer: "bottom",
          });
          if (!decoration) {
            marker.dispose();
            continue;
          }
          const color = m.color;
          const mode = m.renderMode;
          decoration.onRender((el) => styleOverlay(el, color, mode));
          active.set(key, { decoration, marker });
        }
      }
    }

    // Drop decorations for matches that scrolled off or no longer apply.
    for (const [key, painted] of active) {
      if (desired.has(key)) continue;
      painted.decoration.dispose();
      painted.marker.dispose();
      active.delete(key);
    }
  }

  function schedule() {
    if (disposed || frame !== 0) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      repaint();
    });
  }

  const subscriptions: IDisposable[] = [
    term.onRender(() => schedule()),
    term.onScroll(() => schedule()),
  ];

  // Paint whatever is already on screen at attach time.
  schedule();

  return {
    refresh: schedule,
    dispose() {
      disposed = true;
      if (frame !== 0) cancelAnimationFrame(frame);
      for (const painted of active.values()) {
        painted.decoration.dispose();
        painted.marker.dispose();
      }
      active.clear();
      for (const sub of subscriptions) sub.dispose();
    },
  };
}
