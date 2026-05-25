/**
 * Lifecycle: start a stopped project and watch it reach "Running", stop it
 * back to "Stopped", and stop every running project at once with Stop-All.
 */
import { test, expect } from "@playwright/test";

import { installSim } from "./_harness";

test.beforeEach(async ({ page }) => {
  await installSim(page);
  await page.goto("/");
});

test("start then stop a project transitions its status", async ({ page }) => {
  // "Dashboard UI" is the roster's stopped project.
  const row = page.getByRole("row").filter({ hasText: "Dashboard UI" });
  await expect(row).toBeVisible();

  await page.getByRole("button", { name: "Start Dashboard UI" }).click();

  // Optimistic flip is immediate; the mocked running event lands shortly after.
  await expect(row.getByText(/Starting|Running/)).toBeVisible();
  await expect(row.getByText("Running")).toBeVisible({ timeout: 5_000 });

  // Now stoppable — stopping returns it to "Stopped".
  await page.getByRole("button", { name: "Stop Dashboard UI" }).click();
  await expect(row.getByText("Stopped")).toBeVisible();
});

test("stop-all stops every running project", async ({ page }) => {
  const acme = page.getByRole("row").filter({ hasText: "Acme Storefront" });
  const billing = page.getByRole("row").filter({ hasText: "Billing API" });

  // Both start out running in the fixture roster.
  await expect(acme.getByText("Running")).toBeVisible();
  await expect(billing.getByText("Running")).toBeVisible();

  await page.getByRole("button", { name: "Stop all running projects" }).click();
  await page.getByRole("button", { name: "Confirm stop all" }).click();

  await expect(acme.getByText("Stopped")).toBeVisible();
  await expect(billing.getByText("Stopped")).toBeVisible();
});
