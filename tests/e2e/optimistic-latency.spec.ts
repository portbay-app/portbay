/**
 * E2E interaction-latency guard for the optimistic lifecycle.
 *
 * Card: "P3 — Speed as a feature: interaction-latency budget + optimistic
 * lifecycle", DoD item 4 — "a WebDriver script clicks Play and asserts the row
 * shows `starting` within the budget."
 *
 * This drives the **real frontend build** in headless Chromium with the Tauri
 * IPC layer mocked by the shared simulator (`$lib/simulator`), and proves the
 * optimistic flip is *synchronous*: clicking Play paints "Starting…" on the row
 * long before the (mocked, deliberately slow) `start_project` IPC resolves. If
 * a regression made Play await the IPC, "Starting…" would not appear until
 * START_IPC_DELAY_MS and the budget assertion below would fail.
 *
 * The mock + the fictional demo roster are the same ones the screenshot
 * pipeline and the web simulator use — one source of truth (see
 * `src/lib/simulator/`). The complementary <100 ms structural property is
 * locked at the unit level by tests/optimistic.test.ts. See docs/PERFORMANCE.md
 * for why this runs against the frontend build rather than the Tauri binary
 * (tauri-driver is Linux/Windows-only; CI is macOS).
 */
import { test, expect } from "@playwright/test";

import { DEMO_FIXTURES } from "../../src/lib/simulator/fixtures";
import { installSimulatorIpcBrowser } from "../../src/lib/simulator/mockIpc";

/** How long the mocked `start_project` stays pending. Must exceed the budget. */
const START_IPC_DELAY_MS = 5_000;

/**
 * CI-tolerant ceiling for click → "Starting…" painted. The real cost is a
 * frame or two (~16–50 ms); this generous bound absorbs browser + Playwright
 * polling overhead while still failing hard if Play regresses to awaiting the
 * 5 s IPC round-trip.
 */
const PAINT_BUDGET_MS = 1_500;

test('Play paints "Starting…" within the optimistic budget', async ({ page }) => {
  // Inject the shared simulator mock, but with a deliberately slow start and
  // auto-run disabled, so `start_project` stays pending well past the budget —
  // the whole point. (The simulator/screenshot builds use the snappy defaults.)
  await page.addInitScript(installSimulatorIpcBrowser, {
    fixtures: DEMO_FIXTURES,
    options: { startDelayMs: START_IPC_DELAY_MS, autoRunOnStart: false },
  });
  await page.goto("/");

  // "Dashboard UI" is the roster's stopped project — it renders a Play button.
  const playButton = page.getByRole("button", { name: "Start Dashboard UI" });
  await expect(playButton).toBeVisible();

  const row = page.getByRole("row").filter({ hasText: "Dashboard UI" });
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
