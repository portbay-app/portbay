/**
 * Log viewer modal state. The project detail panel opens it for one
 * project; the /logs route opens it from a list.
 */

function createLogViewerStore() {
  let openId = $state<string | null>(null);
  return {
    get id() {
      return openId;
    },
    show(id: string) {
      openId = id;
    },
    hide() {
      openId = null;
    },
  };
}

export const logViewer = createLogViewerStore();
