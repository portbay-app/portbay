/**
 * Vitest config for PortBay's frontend unit tests.
 *
 * Deliberately minimal and SvelteKit-free: the tests target pure TypeScript
 * modules (e.g. the optimistic-lifecycle core), so we only need the `$lib`
 * path alias — not the Svelte compiler or SvelteKit's virtual modules. Keeping
 * it standalone means `pnpm test` is fast and has no dev-server dependency.
 */
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
    },
  },
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
  },
});
