/**
 * Cloudflare-tunnel modal state — which project's tunnel detail is
 * currently open. The modal mounts at the layout root and reads this
 * to decide whether to render.
 */

function createTunnelModalStore() {
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

export const tunnelModal = createTunnelModalStore();
