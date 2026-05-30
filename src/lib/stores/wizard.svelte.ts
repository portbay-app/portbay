/**
 * Add Project wizard — open/close state. Owned outside the wizard so the
 * TopBar's plus button and the projects-table empty state can both
 * trigger the same instance.
 *
 * `requestAdd()` is the gated entry point every "+ Add project" affordance
 * should call: it opens the wizard when the current tier allows another
 * project, and otherwise opens the sign-in / upgrade sheet instead.
 */

import { entitlements } from "./entitlements.svelte";
import { account } from "./account.svelte";
import { projects } from "./projects.svelte";
import { trackEvent } from "$lib/telemetry";

/**
 * `project` — the full Add-project flow (folder → detect → web-server fields).
 * `board`   — a board-only project: a folder + name, registered as a `custom`
 *             kind with no port/start command, so the Tasks page can spin up a
 *             Kanban board that isn't a runnable web project. Both share one
 *             wizard instance (and one project-cap gate — a board is a project).
 */
export type WizardMode = "project" | "board";

function createWizardStore() {
  let open = $state<boolean>(false);
  let mode = $state<WizardMode>("project");
  return {
    get isOpen() {
      return open;
    },
    get mode() {
      return mode;
    },
    show() {
      mode = "project";
      open = true;
    },
    hide() {
      open = false;
    },
    toggle() {
      open = !open;
    },
    /**
     * Gated open. Within the project cap → open the wizard. At the cap →
     * open the account sheet with the right intent (sign up at the anonymous
     * cap, upgrade to Pro at the free cap) instead of the wizard.
     */
    requestAdd() {
      this.requestOpen("project");
    },
    /** Gated open in board-only mode — the Tasks page "New board" affordance. */
    requestBoard() {
      this.requestOpen("board");
    },
    requestOpen(next: WizardMode) {
      const count = projects.value.length;
      if (entitlements.canAddProject(count)) {
        mode = next;
        open = true;
        return;
      }
      trackEvent("project_limit_reached");
      account.open({ intent: entitlements.upgradePromptAt(count) ?? "pro" });
    },
  };
}

export const addProjectWizard = createWizardStore();
