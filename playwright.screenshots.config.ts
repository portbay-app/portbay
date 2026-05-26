/**
 * Playwright config for the screenshot pipeline (`pnpm screenshots`). Separate
 * from playwright.config.ts so the generator never runs under `pnpm test:e2e`.
 * Serves the static SvelteKit build (`build/`) via sirv — the same model as the
 * e2e harness — and points the generator at it. Run `pnpm build` first.
 */
import { defineConfig, devices } from "@playwright/test";

const PORT = 4319;

export default defineConfig({
  testDir: "scripts",
  testMatch: "screenshots.ts",
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  timeout: 600_000,
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  use: { baseURL: `http://127.0.0.1:${PORT}` },
  webServer: {
    command: `pnpm exec sirv build --single --quiet --host 127.0.0.1 --port ${PORT}`,
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
