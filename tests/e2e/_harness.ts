/**
 * Shared e2e harness — installs the simulator's mock Tauri IPC into the page
 * before the SPA boots, so the real frontend build runs against the canonical
 * demo roster with no backend.
 *
 * This is the same mock the screenshot pipeline and web simulator use
 * (`src/lib/simulator/`), driven here through Playwright's `addInitScript`.
 * See `playwright.config.ts` for why these specs run against the frontend
 * build rather than the Tauri binary (tauri-driver is Linux/Windows-only; CI
 * and dev are macOS).
 *
 * `installSimulatorIpcBrowser` is self-contained (it closes over nothing from
 * module scope) so Playwright can serialize it with `.toString()`; the
 * fixtures + options ride along as its single serializable argument.
 */
import type { Page } from "@playwright/test";

import { DEMO_FIXTURES } from "../../src/lib/simulator/fixtures";
import {
  installSimulatorIpcBrowser,
  type SimulatorOptions,
} from "../../src/lib/simulator/mockIpc";

/** Inject the simulator mock IPC. Call before `page.goto`. */
export async function installSim(
  page: Page,
  options?: SimulatorOptions,
): Promise<void> {
  await page.addInitScript(installSimulatorIpcBrowser, {
    fixtures: DEMO_FIXTURES,
    options,
  });
}
