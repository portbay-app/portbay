/**
 * Detail panel: open the slide-over from a row (select + Enter), rename the
 * project, save, and confirm the new name lands in the table.
 */
import { test, expect } from "@playwright/test";

import { installSim } from "./_harness";

test.beforeEach(async ({ page }) => {
  await installSim(page);
  await page.goto("/");
});

test("editing the name in the detail panel updates the table", async ({
  page,
}) => {
  // Open the detail slide-over from the row's "More actions" menu.
  await page
    .getByRole("button", { name: "More actions for Dashboard UI" })
    .click();
  await page.getByRole("menuitem", { name: "Edit project" }).click();

  const panel = page.getByRole("complementary", { name: "Project detail" });
  await expect(panel).toBeVisible();

  const nameInput = panel.locator("#detail-name");
  await expect(nameInput).toHaveValue("Dashboard UI");

  // Editing marks the form dirty, which reveals Save.
  await nameInput.fill("Renamed Dashboard");
  await panel.getByRole("button", { name: /Save/ }).click();

  // The renamed project is reflected in the table.
  await expect(
    page.getByRole("cell").filter({ hasText: "Renamed Dashboard" }).first(),
  ).toBeVisible();
});
