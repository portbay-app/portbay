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

function createWizardStore() {
  let open = $state<boolean>(false);
  return {
    get isOpen() {
      return open;
    },
    show() {
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
      const count = projects.value.length;
      if (entitlements.canAddProject(count)) {
        open = true;
        return;
      }
      trackEvent("project_limit_reached");
      account.open({ intent: entitlements.upgradePromptAt(count) ?? "pro" });
    },
  };
}

export const addProjectWizard = createWizardStore();
