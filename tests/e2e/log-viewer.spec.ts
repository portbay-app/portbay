/**
 * Log viewer: open it from a project's row menu, confirm log lines load from
 * the fixture, and that searching surfaces a match counter.
 */
import { test, expect } from "@playwright/test";

import { installSim } from "./_harness";

test.beforeEach(async ({ page }) => {
  await installSim(page);
  await page.goto("/");
});

test("opens from the row menu and search finds matches", async ({ page }) => {
  // Open Billing API's log viewer from its "More actions" menu.
  await page
    .getByRole("button", { name: "More actions for Billing API" })
    .click();
  await page.getByRole("menuitem", { name: "View logs" }).click();

  const viewer = page.getByRole("dialog", { name: "Log viewer" });
  await expect(viewer).toBeVisible();

  // Lines from the billing-api fixture are present.
  await expect(viewer.getByText("Route cache loaded")).toBeVisible();

  // Searching a term in the log surfaces the N/M match counter.
  await viewer.locator("#logviewer-search").fill("invoices");
  await expect(viewer.getByText(/^\d+\/\d+$/)).toBeVisible();
});
