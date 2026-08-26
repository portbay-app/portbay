# PortBay Accessibility Conformance Report (VPAT® 2.5)

**Based on VPAT® Version 2.5 — WCAG / Section 508 / EN 301 549 edition**

| | |
|---|---|
| **Name of Product/Version** | PortBay (desktop application, v0.1.6) |
| **Report Date** | 2026-07-17 |
| **Product Description** | PortBay is a local development environment manager (Tauri desktop app; SvelteKit single-page front-end rendered in a system WebView). It gives local projects friendly HTTPS hostnames and managed start/stop, and includes agent, database, SSH, capture, and AI-playground surfaces. |
| **Contact Information** | security@portbay.app |
| **Notes** | This is a **partial-conformance, self-assessment** report. It is honest about known gaps and explicitly marks criteria that have not yet been measured. It is **not** a claim of full WCAG 2.1 AA conformance. A remediation backlog is listed at the end. |
| **Evaluation Methods Used** | (1) Static Svelte-compiler accessibility analysis via `svelte-check`, which surfaces the Svelte compiler's built-in `a11y_*` warnings, gated in CI (`scripts/check-a11y.mjs`, `.github/workflows/a11y.yml`). (2) A committed baseline of every in-source `<!-- svelte-ignore a11y_* -->` suppression (`scripts/a11y-baseline.json`), counted and printed on every run. (3) A runtime `@axe-core/playwright` smoke over ~15 primary screens (`tests/sim/a11y-axe-smoke.spec.ts`). (4) Manual source review of ARIA, labels, focus handling, and reduced-motion support. **Not yet performed:** assistive-technology (screen-reader) testing, and the first capture of the runtime axe baseline (`axe.seeded=false`) — several runtime-only criteria below are therefore marked *Not Evaluated*. |

## Applicable Standards / Guidelines

| Standard/Guideline | Included in Report |
|---|---|
| Web Content Accessibility Guidelines 2.1 | Level A (Yes), Level AA (Yes), Level AAA (No) |
| Revised Section 508 (2017), 36 CFR 1194 Appendix A, B, C | Yes (by WCAG reference) |
| EN 301 549 (V3.2.1) | Yes (by WCAG reference) |

## Terms

- **Supports** — The functionality meets the criterion without known defects.
- **Partially Supports** — Some functionality meets the criterion; known gaps exist.
- **Does Not Support** — The majority of the functionality does not meet the criterion.
- **Not Applicable** — The criterion is not relevant to the product.
- **Not Evaluated** — Not yet assessed. Used here only where honest measurement is still pending (e.g. runtime color-contrast, screen-reader behavior). Per VPAT convention this is disclosed rather than guessed.

---

## WCAG 2.1 Report

### Table 1: Success Criteria, Level A

| Criterion | Conformance Level | Remarks and Explanations |
|---|---|---|
| **1.1.1 Non-text Content** (A) | Partially Supports | 66 of 69 `<img>` elements carry `alt` text and icon-only controls broadly use `aria-label` (165 components). One redundant-alt case is baselined (`a11y_img_redundant_alt` ×1). Not every decorative/functional icon has been individually audited. |
| **1.2.1 Audio-only and Video-only (Prerecorded)** (A) | Not Applicable | The product ships no prerecorded audio-only or video-only content. The STT/TTS playgrounds operate on user-supplied or user-generated media. |
| **1.2.2 Captions (Prerecorded)** (A) | Partially Supports | The STT playground renders an audio `<media>` element without a `<track kind="captions">` (`a11y_media_has_caption` ×1, baselined). The audio is user-supplied test input, not product content, but the gap is disclosed. |
| **1.2.3 Audio Description or Media Alternative (Prerecorded)** (A) | Not Applicable | No prerecorded synchronized media is shipped. |
| **1.3.1 Info and Relationships** (A) | Partially Supports | Semantic HTML, `<label>` (70 components), and ARIA (165 components) are used widely. However, some interactive behaviors are attached to static/non-interactive elements (see 4.1.2), which weakens programmatic structure in those spots. |
| **1.3.2 Meaningful Sequence** (A) | Supports | DOM order follows visual order; no CSS-reordering that breaks reading sequence was found in review. Not exhaustively AT-verified. |
| **1.3.3 Sensory Characteristics** (A) | Supports | Instructions do not rely solely on shape, size, or location; status is conveyed with text plus icons. |
| **1.4.1 Use of Color** (A) | Supports | Color is not the sole information channel: status marks carry text labels, and user preferences add color-independent status cues and persistent link underlines (`AccessibilityPanel`). |
| **1.4.2 Audio Control** (A) | Not Applicable | No audio plays automatically for more than 3 seconds; TTS playback is user-initiated. |
| **2.1.1 Keyboard** (A) | Partially Supports | **Known gap.** Eight sites attach click/interaction handlers to static or non-interactive elements without a keyboard handler (`a11y_click_events_have_key_events` ×3, `a11y_no_static_element_interactions` ×3, `a11y_no_noninteractive_element_interactions` ×2 — all baselined). Most of the app is keyboard-operable; these specific controls are not fully keyboard-reachable. |
| **2.1.2 No Keyboard Trap** (A) | Partially Supports | 34 components use `role="dialog"`; modal focus handling exists but has not been assistive-technology-verified for trap-free entry/exit. |
| **2.1.4 Character Key Shortcuts** (A) | Not Evaluated | The app has global hotkeys and a voice wake word; single-character shortcut remap/disable behavior has not been audited against this criterion. |
| **2.2.1 Timing Adjustable** (A) | Not Applicable | No content time limits are imposed on the user. |
| **2.2.2 Pause, Stop, Hide** (A) | Supports | `prefers-reduced-motion` is honored app-wide (`src/app.css` and 9+ components) and a **Reduce motion** user preference minimizes animation, shimmer, and smooth scrolling. |
| **2.3.1 Three Flashes or Below Threshold** (A) | Supports | No flashing content exceeding the threshold is present. |
| **2.4.1 Bypass Blocks** (A) | Does Not Support | **Known gap.** No "skip to main content" link was found. The SPA relies on a persistent sidebar and ARIA landmarks; an explicit bypass mechanism is on the remediation backlog. |
| **2.4.2 Page Titled** (A) | Partially Supports | The application window is titled; per-route document titles within the SPA are not consistently set for every screen. |
| **2.4.3 Focus Order** (A) | Partially Supports | Focus order is generally logical; one non-interactive element carries a `tabindex` (`a11y_no_noninteractive_tabindex` ×1, the sidebar resize handle, baselined). |
| **2.4.4 Link Purpose (In Context)** (A) | Supports | Links and link-buttons carry descriptive text or `aria-label`. |
| **2.5.1 Pointer Gestures** (A) | Partially Supports | The agent board uses drag-and-drop (`svelte-dnd-action`); a full keyboard-equivalent for reordering has not been verified. |
| **2.5.2 Pointer Cancellation** (A) | Supports | Actions fire on pointer-up (click), allowing cancellation by moving off-target. |
| **2.5.3 Label in Name** (A) | Partially Supports | Visible control text is generally included in the accessible name; not exhaustively audited across all 304 components. |
| **2.5.4 Motion Actuation** (A) | Not Applicable | No functionality is operated by device or user motion. |
| **3.1.1 Language of Page** (A) | Supports | `<html lang="en">` is set (`src/app.html`). |
| **3.2.1 On Focus** (A) | Partially Supports | No unexpected context change occurs on focus. However, `autofocus` is used in 7 dialog/prompt surfaces (`a11y_autofocus` ×7, baselined); this is generally acceptable in modal contexts but is flagged for review. |
| **3.2.2 On Input** (A) | Supports | Changing a control value does not trigger an unexpected context change. |
| **3.3.1 Error Identification** (A) | Partially Supports | Forms surface validation errors in text; programmatic association of every error with its field has not been exhaustively verified. |
| **3.3.2 Labels or Instructions** (A) | Supports | Labels and `aria-label`/placeholder instructions are provided for inputs broadly. |
| **4.1.1 Parsing** (A) | Supports | The Svelte compiler enforces well-formed markup at build time. (Obsolete/removed in WCAG 2.2; retained here for 2.1 completeness.) |
| **4.1.2 Name, Role, Value** (A) | Partially Supports | **Known gap.** Interactive behaviors on static elements lack an explicit ARIA `role` in the baselined sites (`a11y_no_static_element_interactions`, `a11y_no_noninteractive_element_interactions`). Elsewhere roles/names/states are set via native semantics and ARIA. |

### Table 2: Success Criteria, Level AA

| Criterion | Conformance Level | Remarks and Explanations |
|---|---|---|
| **1.2.4 Captions (Live)** (AA) | Not Applicable | No live synchronized media is presented. |
| **1.2.5 Audio Description (Prerecorded)** (AA) | Not Applicable | No prerecorded synchronized media is shipped. |
| **1.3.4 Orientation** (AA) | Supports | Content is not locked to a single orientation; layout is responsive. |
| **1.3.5 Identify Input Purpose** (AA) | Partially Supports | `autocomplete` attributes on common personal-data fields have not been comprehensively audited. |
| **1.4.3 Contrast (Minimum)** (AA) | Not Evaluated | Automated contrast measurement is pending the first runtime axe baseline capture (`axe.seeded=false`). A **High contrast** user preference is available and design tokens target both light and dark themes, but measured 4.5:1/3:1 conformance is not yet asserted. |
| **1.4.4 Resize Text** (AA) | Supports | A **Text size** preference (Normal / Large / Larger) scales interface text; layout uses relative units and reflows. |
| **1.4.5 Images of Text** (AA) | Supports | UI text is real text, not images of text. |
| **1.4.10 Reflow** (AA) | Partially Supports | Responsive breakpoints exist (e.g. `max-[640px]` stacking); full reflow at 320 CSS px without loss has not been comprehensively verified. |
| **1.4.11 Non-text Contrast** (AA) | Not Evaluated | Focus rings, borders, and control boundaries exist (24 components style `:focus-visible`), but 3:1 non-text contrast has not yet been measured (pending axe baseline). |
| **1.4.12 Text Spacing** (AA) | Partially Supports | No content is clipped by fixed line-heights in review; not exhaustively tested with user text-spacing overrides. |
| **1.4.13 Content on Hover or Focus** (AA) | Partially Supports | Hover/focus tooltips and cards exist; dismissible/hoverable/persistent behavior has not been fully audited. |
| **2.4.5 Multiple Ways** (AA) | Supports | Screens are reachable via persistent sidebar navigation and in-app search/command surfaces. |
| **2.4.6 Headings and Labels** (AA) | Supports | Descriptive headings and labels are used throughout the panels and forms. |
| **2.4.7 Focus Visible** (AA) | Supports | A visible focus indicator is provided (`:focus-visible` styling), and a **Focus indicator → Strong** preference draws a larger focus ring for keyboard navigation. |
| **3.1.2 Language of Parts** (AA) | Not Applicable | The interface is single-language (English). |
| **3.2.3 Consistent Navigation** (AA) | Supports | The primary sidebar navigation is consistent across screens. |
| **3.2.4 Consistent Identification** (AA) | Supports | Repeated components (status marks, action menus) are identified consistently. |
| **3.3.3 Error Suggestion** (AA) | Partially Supports | Some forms suggest corrections; coverage across all inputs has not been verified. |
| **3.3.4 Error Prevention (Legal, Financial, Data)** (AA) | Supports | Destructive actions (delete project, card, SSH host, etc.) require an explicit in-app confirmation dialog. |
| **4.1.3 Status Messages** (AA) | Partially Supports | 17 components use `aria-live`/`role="status"`/`role="alert"`; live-region coverage for all asynchronous status changes (start/stop, provisioning) is not comprehensive. |

---

## Section 508 and EN 301 549

Because Revised Section 508 (Chapters 4 & 5 software) and EN 301 549 (Chapter 9 Web / Chapter 11 Software) incorporate the WCAG 2.1 Level A and AA success criteria by reference, the conformance results in Tables 1 and 2 above apply directly. The product-specific hardware and closed-functionality provisions of those standards are **Not Applicable** — PortBay is open-platform software that runs on the user's own general-purpose computer and does not restrict the user's assistive technology.

## Conformance Summary (honest self-assessment)

Across the 50 evaluated WCAG 2.1 A/AA success criteria:

- **Supports:** 20
- **Partially Supports:** 18
- **Does Not Support:** 1 (2.4.1 Bypass Blocks — no skip link)
- **Not Applicable:** 8
- **Not Evaluated:** 3 (2.1.4 Character Key Shortcuts; 1.4.3 Contrast; 1.4.11 Non-text Contrast — the two contrast criteria are pending the first runtime axe measurement)

**PortBay does not currently claim full WCAG 2.1 Level AA conformance.** The most material gaps are keyboard operability of a small set of controls (2.1.1 / 4.1.2), the absence of a bypass-blocks mechanism (2.4.1), and unmeasured color/non-text contrast (1.4.3 / 1.4.11).

## Known Gaps and Remediation Backlog (future work)

The following are tracked as separate remediation work, not part of the COMP-1 infrastructure card. Each corresponds to a baselined entry in `scripts/a11y-baseline.json` or a *Not Evaluated* row above:

1. **Keyboard operability (2.1.1 / 4.1.2)** — Add keyboard handlers and appropriate `role`/`tabindex` (or convert to native `<button>`/`<a>`) for the baselined static-element interaction sites: `MacPermissionDialog`, `ThirdPartyLicensesDialog`, `agent/Markdown`, `SidebarResizeHandle`, `PresenceWindow`, and `tasks/Column`.
2. **Bypass blocks (2.4.1)** — Add a "Skip to main content" link and/or verified ARIA landmark structure.
3. **Contrast measurement (1.4.3 / 1.4.11)** — Capture the runtime axe baseline (`A11Y_AXE_UPDATE=1 SIM_HEADLESS=1 SIM_SLOWMO=0 pnpm a11y:axe`), review reported contrast violations, and remediate or accept-with-reason. This flips `axe.seeded` to `true` and turns the smoke into an enforcing net-new-critical gate.
4. **Captions (1.2.2)** — Provide a caption/transcript affordance for the STT playground media element, or confirm N/A and remove the suppression.
5. **`autofocus` review (3.2.1)** — Confirm each of the 7 `autofocus` uses is a modal/prompt where initial focus is expected; otherwise remove.
6. **Assistive-technology pass** — Perform screen-reader testing (VoiceOver / NVDA) to validate 2.1.2, 2.4.3, and 4.1.3 beyond static analysis, and update the *Not Evaluated* rows.

## How accessibility is enforced in CI

- **`scripts/check-a11y.mjs`** — runs `svelte-check` and fails the build on any net-new Svelte a11y warning or any new/grown `<!-- svelte-ignore a11y_* -->` suppression, measured against `scripts/a11y-baseline.json`. Green on today's tree; you cannot silently add a new violation or a new suppression.
- **`tests/sim/a11y-axe-smoke.spec.ts`** — an `@axe-core/playwright` smoke over ~15 primary screens; critical violations fail once the baseline is seeded, serious/moderate/minor are recorded as backlog.
- **`.github/workflows/a11y.yml`** — the `a11y` CI lane that runs the lint gate and the axe smoke on every push to `main` and every PR.

This report is maintained alongside the code. Update it whenever the baseline changes materially or a remediation item lands.
