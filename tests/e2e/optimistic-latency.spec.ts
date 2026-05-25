/**
 * E2E interaction-latency guard for the optimistic lifecycle.
 *
 * Card: "P3 — Speed as a feature: interaction-latency budget + optimistic
 * lifecycle", DoD item 4 — "a WebDriver script clicks Play and asserts the row
 * shows `starting` within the budget."
 *
 * This drives the **real frontend build** in headless Chromium with the Tauri
 * IPC layer mocked, and proves the optimistic flip is *synchronous*: clicking
 * Play paints "Starting…" on the row long before the (mocked, deliberately
 * slow) `start_project` IPC resolves. If a regression made Play await the IPC,
 * "Starting…" would not appear until START_IPC_DELAY_MS and the budget
 * assertion below would fail.
 *
 * The complementary <100 ms structural property (the flip takes no async step)
 * is locked at the unit level by tests/optimistic.test.ts. See
 * docs/PERFORMANCE.md for why this runs against the frontend build rather than
 * the Tauri binary (tauri-driver is Linux/Windows-only; CI is macOS).
 */
import { test, expect, type Page } from "@playwright/test";

/** How long the mocked `start_project` stays pending. Must exceed the budget. */
const START_IPC_DELAY_MS = 5_000;

/**
 * CI-tolerant ceiling for click → "Starting…" painted. The real cost is a
 * frame or two (~16–50 ms); this generous bound absorbs browser + Playwright
 * polling overhead while still failing hard if Play regresses to awaiting the
 * 5 s IPC round-trip.
 */
const PAINT_BUDGET_MS = 1_500;

/** One stopped project — enough to render a row with a working Play button. */
const SEED_PROJECT = {
  id: "demo",
  name: "Demo",
  path: "/Users/dev/Sites/demo",
  type: "node",
  extraPorts: [],
  hostname: "demo.test",
  url: "https://demo.test",
  https: true,
  services: [],
  env: {},
  autoStart: false,
  tags: [],
  status: "stopped",
};

/**
 * Install a minimal Tauri v2 internals shim before the SPA boots. Every
 * `safeInvoke()` routes through `window.__TAURI_INTERNALS__.invoke`, so this is
 * the single seam needed — no production code changes.
 */
async function installTauriMock(page: Page) {
  await page.addInitScript(
    ({ project, startDelay }) => {
      let nextId = 1;
      const sidecar = (name: string) => ({ name, status: "stopped" });
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (window as any).__TAURI_INTERNALS__ = {
        // Window/webview APIs (e.g. the wizard's drag-drop listener) read this
        // synchronously on mount; without it the app throws during boot and the
        // Svelte render flush wedges before the table paints.
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { windowLabel: "main", label: "main" },
        },
        transformCallback(cb: (p: unknown) => void, once: boolean) {
          const id = nextId++;
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          (window as any)[`_${id}`] = (payload: unknown) => {
            if (once) delete (window as any)[`_${id}`];
            return cb?.(payload);
          };
          return id;
        },
        invoke(cmd: string) {
          switch (cmd) {
            case "list_projects":
              return Promise.resolve([project]);
            case "dns_preflight":
              return Promise.resolve({ ready: true });
            case "installed_dev_tools":
              return Promise.resolve([]);
            case "sidecar_status":
              return Promise.resolve({
                processCompose: sidecar("process-compose"),
                caddy: sidecar("caddy"),
                mkcertCa: sidecar("mkcert-ca"),
                dnsmasq: sidecar("dnsmasq"),
                mailpit: sidecar("mailpit"),
                hostsHelper: sidecar("hosts-helper"),
              });
            case "start_project":
            case "force_start_project":
              // Stays pending well past the paint budget — the whole point.
              return new Promise((resolve) =>
                setTimeout(() => resolve(null), startDelay),
              );
            case "plugin:event|listen":
              return Promise.resolve(nextId++);
            case "plugin:event|unlisten":
              return Promise.resolve(null);
            default:
              // list_* → [] (stores that expect arrays don't throw on boot);
              // everything else → null (entitlements & friends fall back).
              return Promise.resolve(cmd.startsWith("list_") ? [] : null);
          }
        },
      };
    },
    { project: SEED_PROJECT, startDelay: START_IPC_DELAY_MS },
  );
}

test('Play paints "Starting…" within the optimistic budget', async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");

  const playButton = page.getByRole("button", { name: "Start Demo" });
  await expect(playButton).toBeVisible();

  const row = page.getByRole("row").filter({ hasText: "Demo" });
  // Pre-condition: the row is not already in a starting state.
  await expect(row.getByText(/Starting/)).toHaveCount(0);

  const t0 = Date.now();
  await playButton.click();

  // The optimistic overlay must paint while the start_project IPC is still
  // pending (it won't resolve for START_IPC_DELAY_MS).
  await expect(row.getByText(/Starting/)).toBeVisible({ timeout: PAINT_BUDGET_MS });
  const elapsed = Date.now() - t0;

  // eslint-disable-next-line no-console
  console.log(
    `click → "Starting…" painted in ${elapsed} ms ` +
      `(start_project IPC mocked to take ${START_IPC_DELAY_MS} ms)`,
  );
  expect(elapsed).toBeLessThan(PAINT_BUDGET_MS);
});
