/**
 * Svelte action: nudge a popover back inside the viewport.
 *
 * Context menus and dropdowns open anchored to a click or trigger; near a
 * window edge they would otherwise clip off-screen. On mount this measures the
 * rendered element and applies a corrective `translate()` so the whole panel
 * stays visible (with a small margin) — translation works for both `fixed` and
 * `absolute` positioning, whatever the offset parent. Callers should `{#key}`
 * (or conditionally mount) the element on their open/position state so a
 * re-open re-runs the action.
 */
const PAD = 8;

export function clampToViewport(el: HTMLElement) {
  const r = el.getBoundingClientRect();
  let dx = 0;
  let dy = 0;
  if (r.right > window.innerWidth - PAD) dx = window.innerWidth - PAD - r.right;
  if (r.left + dx < PAD) dx = PAD - r.left;
  if (r.bottom > window.innerHeight - PAD) dy = window.innerHeight - PAD - r.bottom;
  if (r.top + dy < PAD) dy = PAD - r.top;
  if (dx !== 0 || dy !== 0) el.style.transform = `translate(${dx}px, ${dy}px)`;
}
