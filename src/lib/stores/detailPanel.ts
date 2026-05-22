/**
 * Detail panel open state. The projects table opens the panel on row
 * click (or Enter while a row is selected). Panel lives at root layout.
 */

function createDetailPanelStore() {
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
    toggle(id: string) {
      openId = openId === id ? null : id;
    },
  };
}

export const projectDetailPanel = createDetailPanelStore();
