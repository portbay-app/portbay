// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;

// The web-simulator's mock IPC layer is installed in `src/hooks.client.ts`'s
// `init` hook, which SvelteKit awaits *before* this layout mounts — so the
// dummy roster is in place before any store issues IPC (no race).
