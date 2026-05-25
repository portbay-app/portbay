/**
 * Smoke: the app boots, the primary navigation renders, the projects table
 * renders the demo roster, and sidebar links route between sections.
 */
import { test, expect } from "@playwright/test";

import { installSim } from "./_harness";

test.beforeEach(async ({ page }) => {
  await installSim(page);
  await page.goto("/");
});

test("boots with sidebar nav and the projects table", async ({ page }) => {
  // Primary navigation with its key destinations.
  await expect(page.getByRole("navigation")).toBeVisible();
  await expect(page.getByRole("link", { name: "Projects" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Settings" })).toBeVisible();

  // The projects table renders the canonical roster.
  await expect(page.getByRole("table")).toBeVisible();
  await expect(page.getByText("Acme Storefront")).toBeVisible();
  await expect(page.getByText("Billing API")).toBeVisible();
  await expect(page.getByText("Dashboard UI")).toBeVisible();

  // All six fixture projects produce a row (header + 6 bodies).
  expect(await page.getByRole("row").count()).toBeGreaterThanOrEqual(6);
});

test("sidebar links navigate between sections", async ({ page }) => {
  await page.getByRole("link", { name: "Settings" }).click();
  await expect(page).toHaveURL(/\/settings$/);

  await page.getByRole("link", { name: "Databases" }).click();
  await expect(page).toHaveURL(/\/databases$/);

  await page.getByRole("link", { name: "Projects" }).click();
  await expect(page).toHaveURL(/\/$/);
});
