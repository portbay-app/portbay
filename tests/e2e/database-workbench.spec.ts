import { test, expect } from "@playwright/test";

import { DEMO_FIXTURES } from "../../src/lib/simulator/fixtures";
import { installSimulatorIpcBrowser } from "../../src/lib/simulator/mockIpc";

test("embedded database workbench browses tables and JSON cells", async ({ page }) => {
  await page.addInitScript(installSimulatorIpcBrowser, {
    fixtures: DEMO_FIXTURES,
  });
  await page.goto("/databases");

  await page.getByRole("button", { name: /^quill-sqlite\b/ }).click();
  await page.getByRole("button", { name: "documents 3" }).click();
  await expect(page.getByRole("heading", { name: "documents" })).toBeVisible();

  await expect(page.getByRole("columnheader", { name: /metadata/ })).toBeVisible();
  const metadataCell = page.getByRole("gridcell", {
    name: /"status":"draft"/,
  });
  await metadataCell.getByRole("button", { name: /"status":"draft"/ }).click();
  await expect(metadataCell.getByRole("textbox")).toHaveValue(
    '{"status":"draft","tags":["release","docs"]}',
  );

  await page.keyboard.press("Escape");
  await expect(
    metadataCell.getByRole("button", { name: /"status":"draft"/ }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Chart" }).click();
  await expect(page.getByRole("button", { name: "Chart" })).toHaveClass(
    /bg-accent/,
  );

  await page.getByRole("button", { name: "Schema diagram" }).click();
  await expect(page.getByText("Schema Diagram", { exact: true })).toBeVisible();
  await expect(page.getByText("2 tables", { exact: true })).toBeVisible();
  await expect(page.getByRole("application")).toBeVisible();

  await page.getByRole("button", { name: "Open new tab" }).click();
  await page.getByRole("button", { name: "Visual query builder" }).click();
  await expect(page.getByRole("tab", { name: "Query Builder" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});
