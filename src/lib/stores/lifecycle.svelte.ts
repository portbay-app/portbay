/**
 * lifecycle — local install-age tracking that drives the in-app feedback /
 * review nudges (the universal counterpart to the backend's lifecycle emails;
 * this reaches anonymous users too, who have no email on file).
 *
 * State lives in localStorage (stable across launches, no Rust/registry
 * coupling): the install timestamp and a "done" flag per prompt so each fires
 * at most once and is never shown again after action or dismissal.
 *
 *   - ~24h after install → a one-line feedback ask.
 *   - ~7 days after install → a GitHub-star / review ask.
 *
 * Prompts are sequential: the 24h ask is auto-retired once the 1-week ask
 * becomes due, so a late first run shows only the most relevant one.
 */

import { browser } from "$app/environment";

const KEY_INSTALLED = "portbay.installedAt";
const KEY_FEEDBACK = "portbay.lifecycle.feedback24h";
const KEY_REVIEW = "portbay.lifecycle.review1week";

const DAY_MS = 24 * 60 * 60 * 1000;

export type LifecyclePrompt = "feedback24h" | "review1week";

function readMs(key: string): number | null {
  const raw = localStorage.getItem(key);
  if (!raw) return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
}

function createLifecycleStore() {
  let due = $state<LifecyclePrompt | null>(null);

  /** Compute which prompt (if any) is due. Call once on app start. */
  function evaluate(): void {
    if (!browser) return;

    let installedAt = readMs(KEY_INSTALLED);
    if (installedAt === null) {
      installedAt = Date.now();
      localStorage.setItem(KEY_INSTALLED, String(installedAt));
    }

    const age = Date.now() - installedAt;
    const feedbackDone = localStorage.getItem(KEY_FEEDBACK) === "1";
    const reviewDone = localStorage.getItem(KEY_REVIEW) === "1";

    if (age >= 7 * DAY_MS && !reviewDone) {
      // Past a week: the review ask supersedes the (now-stale) 24h ask.
      if (!feedbackDone) localStorage.setItem(KEY_FEEDBACK, "1");
      due = "review1week";
    } else if (age >= DAY_MS && age < 7 * DAY_MS && !feedbackDone) {
      due = "feedback24h";
    } else {
      due = null;
    }
  }

  /** Mark the current prompt handled (acted on or dismissed) and clear it. */
  function complete(which: LifecyclePrompt): void {
    if (browser) {
      localStorage.setItem(which === "feedback24h" ? KEY_FEEDBACK : KEY_REVIEW, "1");
    }
    if (due === which) due = null;
  }

  return {
    get due(): LifecyclePrompt | null {
      return due;
    },
    evaluate,
    complete,
  };
}

export const lifecycle = createLifecycleStore();
