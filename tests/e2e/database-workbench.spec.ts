import { test, expect } from "@playwright/test";

import { DEMO_FIXTURES } from "../../src/lib/simulator/fixtures";
import { installSimulatorIpcBrowser } from "../../src/lib/simulator/mockIpc";

test("embedded database workbench browses tables and JSON cells", async ({ page }) => {
  await page.addInitScript(installSimulatorIpcBrowser, {
    fixtures: DEMO_FIXTURES,
  });
  await page.goto("/databases");

  await expect(page.getByRole("heading", { name: "Data Workbench" })).toBeVisible();
  await page.getByRole("button", { name: "quill-sqlite" }).click();
  await page.getByRole("button", { name: "documents 3" }).click();

  await expect(page.getByRole("columnheader", { name: /metadata/ })).toBeVisible();
  await page.getByRole("button", { name: /"status":"draft"/ }).click();
  await expect(page.getByText("JSON")).toBeVisible();
  await expect(page.getByText("status", { exact: true })).toBeVisible();
  await expect(page.getByText('"draft"', { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "ERD" }).click();
  await expect(page.getByLabel("Database schema diagram")).toBeVisible();

  await page.getByRole("button", { name: "Builder" }).click();
  await expect(page.getByRole("button", { name: "Build query" })).toBeVisible();

  await page.getByRole("button", { name: "Visual Explain" }).click();
  await expect(page.getByLabel("Visual explain plan")).toBeVisible();

  await page.getByRole("button", { name: "Chart" }).click();
  await expect(page.getByText("id", { exact: true })).toBeVisible();
});
