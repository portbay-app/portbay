/**
 * Add-project wizard: open it, type a folder path, run detection (which fills
 * the standard fields), commit, and confirm the new row appears in the table.
 *
 * The native folder dialog (`Browse…`) can't be driven from a browser, so the
 * spec uses the wizard's path text input + `Detect` — the same code path the
 * dialog feeds into.
 */
import { test, expect } from "@playwright/test";

import { installSim } from "./_harness";

test.beforeEach(async ({ page }) => {
  await installSim(page);
  await page.goto("/");
});

test("adding a project through the wizard adds a row", async ({ page }) => {
  await page.getByRole("button", { name: "Add project" }).click();

  const wizard = page.getByRole("complementary", { name: "Add Project" });
  await expect(wizard).toBeVisible();

  // Type a folder path and run detection.
  await wizard
    .getByPlaceholder("/path/to/your/project")
    .fill("/Users/dev/Sites/checkout-service");
  await wizard.getByRole("button", { name: "Detect" }).click();

  // Detection fills the standard fields (id derived from the folder name).
  await expect(wizard.locator("#wizard-name")).toHaveValue("Checkout Service");

  // Commit — the footer "Add" button.
  await wizard.getByRole("button", { name: "Add", exact: true }).click();

  // The new project shows up as a table row.
  await expect(
    page.getByRole("cell").filter({ hasText: "Checkout Service" }).first(),
  ).toBeVisible();
});
