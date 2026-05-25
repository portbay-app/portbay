/**
 * Playwright config for PortBay's interaction-latency E2E (card: "Speed as a
 * feature"). It serves the **real frontend build** (`build/`, the same SPA the
 * Tauri webview loads) with a static SPA server and drives it in headless
 * Chromium. The Tauri IPC layer is mocked inside the test via an init script —
 * see tests/e2e/optimistic-latency.spec.ts.
 *
 * Why not drive the actual Tauri binary: `tauri-driver` supports Linux +
 * Windows only (there is no WKWebView WebDriver on macOS), and both CI and the
 * dev machine are macOS. The optimistic flip this guards is pure frontend, so
 * the built SPA in a real browser exercises exactly the guarded code path.
 */
import { defineConfig, devices } from "@playwright/test";

const PORT = 4173;

export default defineConfig({
  testDir: "tests/e2e",
  testMatch: "**/*.spec.ts",
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["github"], ["list"]] : "list",
  timeout: 30_000,
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    // `build/` is produced by `pnpm build` (the `pretest:e2e` step). `--single`
    // gives the SvelteKit SPA its index.html fallback for client-side routing.
    command: `pnpm exec sirv build --single --quiet --host 127.0.0.1 --port ${PORT}`,
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
