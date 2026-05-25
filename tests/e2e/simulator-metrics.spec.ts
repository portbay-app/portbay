import { test, expect } from "@playwright/test";

import { DEMO_FIXTURES } from "../../src/lib/simulator/fixtures";
import { installSimulatorIpcBrowser } from "../../src/lib/simulator/mockIpc";

test("simulator renders live sidebar system metrics", async ({ page }) => {
  await page.addInitScript(installSimulatorIpcBrowser, {
    fixtures: DEMO_FIXTURES,
  });
  await page.goto("/");

  const meters = page.locator("#sidebar-system-meters");
  await expect(meters).toBeVisible();
  await expect(meters.getByText("CPU")).toBeVisible();
  await expect(meters.getByText("Memory")).toBeVisible();
  await expect(meters.getByText("Disk")).toBeVisible();

  await expect(meters.getByText(/^\d+%$/)).toBeVisible();
  await expect(meters.getByText(/^\d+\.\d GB$/)).toBeVisible();
  await expect(meters.getByText(/^\d+ GB$/)).toBeVisible();

  const cpuBefore = await meters.getByText(/^\d+%$/).first().textContent();
  await expect
    .poll(async () => meters.getByText(/^\d+%$/).first().textContent())
    .not.toBe(cpuBefore);
});
