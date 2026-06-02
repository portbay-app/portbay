/**
 * Open state for the SFTP file-manager overlay. Opened per SSH connection from
 * the SSH page; the overlay itself lives once in the (connections) layout.
 */

type Target = { connectionId: string; label: string };

function createFileBrowserStore() {
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

export const fileBrowser = createFileBrowserStore();
