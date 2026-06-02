/**
 * Open state for the remote deploy / run panel. Opened per SSH connection from
 * the SSH page; the panel itself lives once in the (connections) layout.
 */

type Target = { connectionId: string; label: string };

function createDeployPanelStore() {
  let target = $state<Target | null>(null);
  return {
    get target() {
      return target;
    },
    get isOpen() {
      return target !== null;
    },
    open(connectionId: string, label: string) {
      target = { connectionId, label };
    },
    close() {
      target = null;
    },
  };
}

export const deployPanel = createDeployPanelStore();
