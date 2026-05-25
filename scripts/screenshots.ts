/**
 * PortBay screenshot pipeline — one reproducible command that renders the real
 * frontend build against the fictional demo roster and writes a consistent,
 * on-brand, light+dark set for the docs site and README.
 *
 *   pnpm screenshots               # all shots, light + dark
 *   SHOT=inspector pnpm screenshots  # only shots whose name matches "inspector"
 *
 * It runs through Playwright (the same runner + static-build serving as the
 * e2e harness — see playwright.screenshots.config.ts, which serves `build/`
 * via sirv). For each shot × theme it opens a retina context, injects the
 * shared simulator mock (`$lib/simulator`) + forces the theme via localStorage
 * *before* the SPA boots, navigates, settles, captures, then composites the
 * capture into a macOS window frame in a second pass (no image-lib dependency).
 *
 * ⛔ DUMMY DATA ONLY: every pixel comes from the fictional roster in
 * src/lib/simulator/fixtures.ts — never a real project, path, domain, or
 * account. That is a review gate.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { test, expect, type Browser } from "@playwright/test";

import { DEMO_FIXTURES } from "../src/lib/simulator/fixtures";
import { installSimulatorIpcBrowser } from "../src/lib/simulator/mockIpc";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(ROOT, "docs-site", "public", "screenshots");

/** Logical viewport for every capture; @2x for retina-crisp output. */
const VIEWPORT = { width: 1440, height: 900 };
const SCALE = 2;

type Theme = "light" | "dark";

interface Shot {
  /** Output basename (light → `name.png`, dark → `name-dark.png`). */
  name: string;
  /** SPA route to capture. */
  route: string;
  /** Window-frame title shown in the macOS title bar. */
  title: string;
  /** Restrict to specific themes (default: both). */
  themes?: Theme[];
}

const SHOTS: Shot[] = [
  { name: "projects", route: "/", title: "PortBay — Projects" },
  { name: "inspector", route: "/inspector", title: "PortBay — HTTP Inspector" },
  { name: "languages", route: "/languages", title: "PortBay — Languages" },
  { name: "databases", route: "/databases", title: "PortBay — Databases" },
  { name: "dns", route: "/dns", title: "PortBay — DNS" },
  { name: "services", route: "/services", title: "PortBay — Services" },
  { name: "domains", route: "/domains", title: "PortBay — Domains" },
];

/** Kill animations/transitions so captures are deterministic. */
const FREEZE_CSS = `*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;transition-duration:0s!important;transition-delay:0s!important;caret-color:transparent!important}`;

/** Capture a single route in a given theme; returns the raw PNG buffer. */
async function capture(
  browser: Browser,
  baseURL: string,
  route: string,
  theme: Theme,
): Promise<Buffer> {
  const context = await browser.newContext({
    viewport: VIEWPORT,
    deviceScaleFactor: SCALE,
  });
  const page = await context.newPage();
  // Force theme + install the dummy-data mock before any app code runs.
  await page.addInitScript((t) => localStorage.setItem("portbay.theme", t), theme);
  await page.addInitScript(installSimulatorIpcBrowser, {
    fixtures: DEMO_FIXTURES,
    options: { autoRunOnStart: false },
  });
  // Warm on the dashboard first: it mounts the sidecar status poll, so the
  // (singleton) sidecars store is healthy before we capture. Then SPA-navigate
  // to the target route — client-side nav preserves store state, so every
  // route shows "All Systems Operational" instead of the cold-start setup nag.
  await page.goto(`${baseURL}/`, { waitUntil: "networkidle" });
  await page.waitForTimeout(300);
  if (route !== "/") {
    await page.locator(`a[href="${route}"]`).first().click();
    await page.waitForURL(`**${route}`);
    await page.waitForLoadState("networkidle");
  }
  await page.addStyleTag({ content: FREEZE_CSS });
  await page.waitForTimeout(500); // fonts + final layout settle
  const buf = await page.screenshot({ type: "png" });
  await context.close();
  return buf;
}

/** Composite a raw capture into a macOS window frame on a transparent canvas. */
async function frame(
  browser: Browser,
  shot: Buffer,
  title: string,
  theme: Theme,
): Promise<Buffer> {
  const margin = 64;
  const bar = 30;
  const w = VIEWPORT.width + margin * 2;
  const h = VIEWPORT.height + bar + margin * 2;
  const barBg = theme === "light" ? "#e8e8ea" : "#2b2b2e";
  const titleColor = theme === "light" ? "#5b5b5f" : "#9a9aa0";
  const dataUri = `data:image/png;base64,${shot.toString("base64")}`;
  const html = `<!doctype html><html><head><meta charset="utf-8"><style>
    html,body{margin:0;background:transparent}
    .win{position:absolute;left:${margin}px;top:${margin}px;width:${VIEWPORT.width}px;
      border-radius:12px;overflow:hidden;box-shadow:0 28px 70px rgba(0,0,0,.40),0 6px 20px rgba(0,0,0,.22)}
    .bar{height:${bar}px;display:flex;align-items:center;padding:0 12px;background:${barBg};position:relative}
    .dots{display:flex;gap:8px}
    .dot{width:12px;height:12px;border-radius:50%}
    .r{background:#ff5f57}.y{background:#febc2e}.g{background:#28c840}
    .title{position:absolute;left:0;right:0;text-align:center;font:600 13px -apple-system,system-ui,sans-serif;color:${titleColor};pointer-events:none}
    img{display:block;width:${VIEWPORT.width}px;height:${VIEWPORT.height}px}
  </style></head><body>
    <div class="win">
      <div class="bar"><div class="dots"><span class="dot r"></span><span class="dot y"></span><span class="dot g"></span></div><div class="title">${title}</div></div>
      <img src="${dataUri}"/>
    </div>
  </body></html>`;
  const context = await browser.newContext({
    viewport: { width: w, height: h },
    deviceScaleFactor: SCALE,
  });
  const page = await context.newPage();
  await page.setContent(html, { waitUntil: "networkidle" });
  const buf = await page.screenshot({ type: "png", fullPage: true, omitBackground: true });
  await context.close();
  return buf;
}

test("generate screenshots", async ({ browser, baseURL }) => {
  test.setTimeout(180_000);
  await mkdir(OUT_DIR, { recursive: true });

  const filter = process.env.SHOT;
  const shots = filter ? SHOTS.filter((s) => s.name.includes(filter)) : SHOTS;
  if (shots.length === 0) throw new Error(`no shots match SHOT="${filter}"`);

  for (const shot of shots) {
    for (const theme of shot.themes ?? (["light", "dark"] as Theme[])) {
      const raw = await capture(browser, baseURL!, shot.route, theme);
      const framed = await frame(browser, raw, shot.title, theme);
      const file = join(OUT_DIR, `${shot.name}${theme === "dark" ? "-dark" : ""}.png`);
      await writeFile(file, framed);
      // eslint-disable-next-line no-console
      console.log(`✓ ${shot.route} (${theme}) → ${file.replace(`${ROOT}/`, "")}`);
    }
  }
});

/**
 * Smoke check for the hosted web-simulator build: with PUBLIC_SIMULATOR=true,
 * the SPA must boot against the baked-in dummy roster (no Playwright injection)
 * and the simulated lifecycle must work end-to-end. Run against a `build:web`
 * output: `pnpm build:web && SIMULATOR_SMOKE=1 pnpm exec playwright test \
 * --config playwright.screenshots.config.ts -g boots`. Skipped otherwise.
 */
test("web simulator boots with baked-in data and runs a project", async ({ page, baseURL }) => {
  test.skip(
    process.env.SIMULATOR_SMOKE !== "1",
    "needs a build:web output + SIMULATOR_SMOKE=1",
  );
  await page.goto(`${baseURL}/`, { waitUntil: "networkidle" });
  // Data served with no injection ⇒ the baked-in mock (hooks.client) installed.
  await expect(page.getByText("Acme Storefront")).toBeVisible();
  const row = page.getByRole("row").filter({ hasText: "Dashboard UI" });
  await page.getByRole("button", { name: "Start Dashboard UI" }).click();
  await expect(row.getByText("Starting")).toBeVisible({ timeout: 1_500 });
  // Reaches Running ⇒ the simulated portbay://status event was emitted + received.
  await expect(row.getByText("Running")).toBeVisible({ timeout: 5_000 });
});
