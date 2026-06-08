/**
 * Writing Tools — preset selection transforms for the card editor.
 *
 * Apple-Writing-Tools-style one-click actions over a text selection, but
 * dev-shaped: a card spec is a task brief headed for an AI coding agent (or
 * dictated dev notes on the way to becoming one), not correspondence — so
 * Apple's tone presets (Friendly/Professional) and prose Summary are out
 * (user decision 2026-06-06), and an agent-readiness action is in. Same
 * trust model as Smart Dictation throughout: every failure keeps the user's
 * text, ⌘Z reverts, nothing leaves the machine.
 *
 * Two mechanisms, both existing probed pipelines (see
 * `DictationRewriter.transformSelection`):
 *   • `edit`    — canned instruction through `dictation_edit` (EDIT_RULES),
 *                 the Voice Edit Mode path minus the voice;
 *   • `rewrite` — the selection through `dictation_rewrite` with a context
 *                 override (BASE_RULES + SMART_EXAMPLES + context tail) —
 *                 used where the smart-rewrite prompt's no-invention
 *                 machinery beats a bare instruction.
 *
 * Every action below was probed offline against the on-device 3B
 * (2026-06-06, /tmp/wt-probe kit — full log in the kanban card) with
 * realistic dev-note samples: instruction applied, no invented facts,
 * technical tokens (paths, flags, branch names) verbatim, within the growth
 * cap, already-clean text degrades to "Kept original". The wording is
 * LOAD-BEARING on the 3B — re-probe after ANY edit.
 *
 * Probed and DROPPED — don't re-add without re-probing:
 *   • Table: deterministic truncation at the markdown header row, even with
 *     maxTokens at the ceiling.
 *   • Task brief ("one clear actionable task"): the compression pressure
 *     drops ~one clause per dense ramble on BOTH paths (v1 lost the "janik
 *     said" attribution, v3 lost the pc.rs hypothesis; the rewrite-path
 *     todo_task tail lost both and cross-leaked a fact between providers).
 *     Rewrite/Concise/Agent prompt keep every fact on the same samples.
 *   • Bug report: when the selection lacks expected behavior or repro, the
 *     3B fills the sections anyway — negation-derived expectations, invented
 *     repro procedures, even an environment template ("version: Not
 *     specified"). Clean-looking invented text, on both paths, across three
 *     instruction wordings.
 *
 * Known accepted limits: Proofread may drop profanity and re-case lowercase
 * tool names ("dnsmasq" → "DNSmasq") — persisted across three wordings; the
 * vocabulary mechanism corrects the casing when the term is on the surface.
 * Key points can mangle a compressed plain word ("reconciler" →
 * "Recoliner") — an explicit spell-exactly instruction does not fix it.
 */
import type { IconName } from "$lib/components/atoms/Icon.svelte";

import type { RewriteContext } from "./types";

/** How a preset runs: a canned instruction through the edit path, or the
 * selection through the smart-rewrite path with a context override. */
export type WritingToolAction =
  | { kind: "edit"; instruction: string }
  | { kind: "rewrite"; context: RewriteContext };

export interface WritingToolPreset {
  id: string;
  label: string;
  icon: IconName;
  action: WritingToolAction;
}

/** Grouping: cleanup first, then dev formatting. */
export const WRITING_TOOL_GROUPS: WritingToolPreset[][] = [
  [
    {
      id: "proofread",
      label: "Proofread",
      icon: "spell-check",
      action: {
        kind: "edit",
        instruction:
          "Fix grammar, spelling, punctuation, and capitalization only. Do not " +
          "remove, add, or replace any words. Keep technical names exactly as " +
          "written, including their casing.",
      },
    },
    {
      id: "rewrite",
      label: "Rewrite",
      icon: "pen-line",
      action: {
        kind: "edit",
        instruction: "Rewrite this text so it reads clearly and well.",
      },
    },
    {
      id: "concise",
      label: "Concise",
      icon: "chevrons-down-up",
      action: {
        kind: "edit",
        instruction:
          "Make this text more concise. Keep every fact, name, number, and " +
          "technical reference.",
      },
    },
  ],
  [
    {
      // The board's headline action: make the selection dispatch-ready.
      // Runs the REWRITE path with the agent_prompt context — probed: keeps
      // every fact on instruction-shaped notes, and on purely descriptive
      // text it restates the problem precisely instead of inventing fixes
      // (the bare edit-instruction variant invented remediation steps —
      // "set the port back to 53053" — across two wordings).
      id: "agentprompt",
      label: "Agent prompt",
      icon: "bot",
      action: { kind: "rewrite", context: "agent_prompt" },
    },
    {
      id: "keypoints",
      label: "Key points",
      icon: "list",
      action: {
        kind: "edit",
        instruction:
          "Summarize this text as a short list of key points, one '- ' bullet " +
          "per line.",
      },
    },
    {
      id: "list",
      label: "Turn into list",
      icon: "list-ordered",
      action: {
        kind: "edit",
        instruction:
          "Break this text into a list of its distinct points, one '- ' item " +
          "per line. Keep the original wording of each point.",
      },
    },
  ],
];

/** Minimum selection length for the affordance to appear — anything shorter
 * is cheaper to retype than to transform (mirrors `dictation_edit`'s own
 * instruction gate). */
export const WRITING_TOOLS_MIN_SELECTION = 3;
