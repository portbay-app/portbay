/**
 * Error envelope: when an IPC command fails, `safeInvoke` surfaces a toast
 * (role="alert") that the user can dismiss. The harness injects a failure via
 * the simulator's `failCommands` option.
 */
import { test, expect } from "@playwright/test";

import { installSim } from "./_harness";

test("a failed command shows a dismissible error toast", async ({ page }) => {
  await installSim(page, { failCommands: ["stop_project"] });
  await page.goto("/");

  // Billing API runs in the fixture — stopping it triggers the injected failure.
  await page.getByRole("button", { name: "Stop Billing API" }).click();

  // safeInvoke surfaces the failure as a toast. Its "Dismiss" button is unique
  // to the toast — the inline row error uses "Dismiss inline error" — so it's
  // an unambiguous proxy for "the toast is showing".
  const dismiss = page.getByRole("button", { name: "Dismiss", exact: true });
  await expect(dismiss).toBeVisible();
  await dismiss.click();
  await expect(dismiss).toHaveCount(0);
});
