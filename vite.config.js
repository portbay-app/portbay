import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  // Tailwind 4's Vite plugin must come before sveltekit so it processes
  // CSS imports during dev. Order matters — sveltekit() must be last among
  // these two for proper HMR with .svelte files.
  plugins: [tailwindcss(), sveltekit()],

  // Build-time flag for the hosted web simulator (`pnpm build:web`). Unset in
  // the desktop/Tauri build, so it folds to "" and the simulator mock + dummy
  // fixtures tree-shake out of the bundle entirely. Consumed by the `init`
  // hook in src/hooks.client.ts.
  define: {
    "import.meta.env.PUBLIC_SIMULATOR": JSON.stringify(
      process.env.PUBLIC_SIMULATOR ?? "",
    ),
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    // Tauri's WKWebView will happily replay a previously-fetched ES module
    // from its own HTTP cache, so a code change can keep crashing against a
    // stale compiled module even after Vite serves the fresh one (it bit us
    // with the SSH page's old `host.tags.length` after a refactor). Force the
    // webview to revalidate every dev asset so a reload always runs current
    // code. Cheap on localhost; only applies to `vite dev`.
    headers: {
      "Cache-Control": "no-store",
    },
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
