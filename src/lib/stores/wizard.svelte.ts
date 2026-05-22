/**
 * Add Project wizard — open/close state. Owned outside the wizard so the
 * TopBar's plus button and the projects-table empty state can both
 * trigger the same instance.
 */

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
  };
}

export const addProjectWizard = createWizardStore();
