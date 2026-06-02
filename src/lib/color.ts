/**
 * Column-accent legibility.
 *
 * A column accent is stored as one hex value, but it has to read against two
 * very different surfaces: the near-black dark theme and the near-white light
 * theme. A color picked to pop in dark mode (say white) is invisible as text in
 * light mode, and vice-versa. `legibleAccent` keeps the chosen hue and
 * saturation but clamps lightness into a band that stays readable on the active
 * theme's surface — so one stored color renders correctly in both.
 */
import type { ResolvedTheme } from "$lib/stores/theme.svelte";

// Lightness bands (0–1) that keep an accent legible as a small bold label or
// glyph on each theme's surface: the light surface needs a darker accent, the
// dark surface a lighter one.
const MAX_L_LIGHT = 0.42;
const MIN_L_DARK = 0.62;

/** Adjust a stored hex accent so it stays readable on the active theme. */
export function legibleAccent(hex: string, theme: ResolvedTheme): string {
  const rgb = parseHex(hex);
  if (!rgb) return hex;
  const [h, s, l] = rgbToHsl(rgb);
  const clamped =
    theme === "light" ? Math.min(l, MAX_L_LIGHT) : Math.max(l, MIN_L_DARK);
  if (clamped === l) return hex; // already in band — keep the exact input
  return hslToHex(h, s, clamped);
}

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  let h = m[1];
  if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
  const n = parseInt(h, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function rgbToHsl([r, g, b]: [number, number, number]): [number, number, number] {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  let h = 0;
  let s = 0;
  if (d !== 0) {
    s = d / (1 - Math.abs(2 * l - 1));
    switch (max) {
      case r:
        h = (((g - b) / d) % 6 + 6) % 6;
        break;
      case g:
        h = (b - r) / d + 2;
        break;
      default:
        h = (r - g) / d + 4;
        break;
    }
    h *= 60;
  }
  return [h, s, l];
}

function hslToHex(h: number, s: number, l: number): string {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  const to = (v: number) =>
    Math.round((v + m) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${to(r)}${to(g)}${to(b)}`;
}
