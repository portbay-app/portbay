// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;

/**
 * In the hosted web-simulator build (`pnpm build:web`, PUBLIC_SIMULATOR=true),
 * install the mock IPC layer here — `load` is awaited before the layout renders
 * and before any store issues IPC, so the dummy roster is in place with no race.
 *
 * The flag is a compile-time constant (vite.config.js `define`), so in the
 * desktop/Tauri build this branch is statically dead and the dynamic import —
 * with the entire simulator module (mock + fixtures) — tree-shakes out.
 */
export const load = async () => {
  if (import.meta.env.PUBLIC_SIMULATOR === "true") {
    const { installSimulator } = await import("$lib/simulator/mockIpc");
    installSimulator();
  }
};
