/**
 * `use:trapFocus` — contains keyboard focus within an open overlay and
 * restores it to the previously-focused element on close (WCAG 2.4.3).
 *
 * On mount it moves focus into the node (first focusable, or the node
 * itself) unless focus is already inside — so a modal that autofocuses
 * its own input isn't overridden. Tab / Shift+Tab wrap at the ends.
 * Attach to the dialog/panel container; Escape-to-close stays the
 * component's own concern.
 */
const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function trapFocus(node: HTMLElement) {
  const previouslyFocused = document.activeElement as HTMLElement | null;

  function focusables(): HTMLElement[] {
    return Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (el) => el.offsetParent !== null || el === document.activeElement,
    );
  }

  // Move focus in, unless the component already placed it (e.g. an input).
  queueMicrotask(() => {
    if (!node.contains(document.activeElement)) {
      (focusables()[0] ?? node).focus();
    }
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== "Tab") return;
    const f = focusables();
    if (f.length === 0) {
      e.preventDefault();
      node.focus();
      return;
    }
    const first = f[0];
    const last = f[f.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || !node.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  node.addEventListener("keydown", onKeydown);

  return {
    destroy() {
      node.removeEventListener("keydown", onKeydown);
      previouslyFocused?.focus?.();
    },
  };
}
