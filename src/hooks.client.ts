import type { ClientInit } from "@sveltejs/kit";

/**
 * In the hosted web-simulator build (`pnpm build:web`, PUBLIC_SIMULATOR=true),
 * install the mock IPC layer here.
 *
 * `init` is the one client hook SvelteKit *awaits before the app boots* — i.e.
 * before any route `load`, component mount, or store `start()` runs. That
 * ordering guarantee is the whole point: the mock previously installed inside
 * `+layout.ts`'s `load`, but the `await import()` of the simulator chunk
 * resolved a frame or two *after* the layout mounted, so the first IPC calls
 * (store `listen()`, `getCurrentWebview()`, `project_icon`) hit an undefined
 * `window.__TAURI_INTERNALS__` and threw. Installing in `init` closes that race.
 *
 * `PUBLIC_SIMULATOR` is a compile-time constant (vite.config.js `define`), so in
 * the desktop/Tauri build this branch is statically dead and the dynamic import
 * — the entire simulator module (mock + fixtures) — tree-shakes out.
 */
export const init: ClientInit = async () => {
  if (import.meta.env.PUBLIC_SIMULATOR === "true") {
    const { installSimulator } = await import("$lib/simulator/mockIpc");
    installSimulator();
  }
};
