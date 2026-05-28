/**
 * licenseDialog — drives the single <AboutLicenseDialog> mounted at the layout
 * root. The "About License" surface shows the full Community-vs-Pro matrix, the
 * two honest ways to get Pro, and the open-source honesty note. Opened from the
 * Account settings card and the user menu; the SignInSheet stays the focused
 * upgrade CTA, this is the "what is it / how does it work" detail view.
 */

import { trackEvent } from "$lib/telemetry";

function createLicenseDialogStore() {
  let open = $state(false);

  return {
    get isOpen() {
      return open;
    },
    open() {
      open = true;
      trackEvent("upgrade_dialog_viewed");
    },
    close() {
      open = false;
    },
  };
}

export const licenseDialog = createLicenseDialogStore();
