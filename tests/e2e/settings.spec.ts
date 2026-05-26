/**
 * Settings: the density preference persists across a reload (it's written to
 * localStorage and reapplied to <body data-density> on boot).
 */
import { test, expect } from "@playwright/test";

import { installSim } from "./_harness";

test.beforeEach(async ({ page }) => {
  await installSim(page);
  await page.goto("/settings");
});

test("density preference persists across reload", async ({ page }) => {
  // Density is a Segmented control (buttons with aria-pressed); default is
  // "comfortable".
  const compact = page.getByRole("button", { name: "Compact" });
  await compact.click();

  await expect(compact).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("body")).toHaveAttribute("data-density", "compact");

  await page.reload();

  // The choice survives the reload (read back from localStorage on boot).
  await expect(page.locator("body")).toHaveAttribute("data-density", "compact");
  await expect(page.getByRole("button", { name: "Compact" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});
