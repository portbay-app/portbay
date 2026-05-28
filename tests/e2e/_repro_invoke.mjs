import { chromium } from "@playwright/test";
import { spawn } from "node:child_process";

const srv = spawn("pnpm", ["exec", "sirv", "build", "--single", "--host", "127.0.0.1", "--port", "4521"], { cwd: process.cwd() });
let port = 4521, base = "";
srv.stdout.on("data", (d) => {
  const s = d.toString();
  const m = s.match(/Local:\s+http:\/\/127\.0\.0\.1:(\d+)/);
  if (m) { port = Number(m[1]); base = `http://127.0.0.1:${port}`; }
});
// wait for ready
for (let i = 0; i < 40 && !base; i++) await new Promise(r => setTimeout(r, 200));
if (!base) base = `http://127.0.0.1:${port}`;
await new Promise(r => setTimeout(r, 500));

const b = await chromium.launch();
const p = await b.newPage();
const errs = [];
p.on("pageerror", (e) => errs.push("PAGEERROR: " + (e.stack || e.message)));
p.on("console", (m) => { if (m.type()==="error") errs.push("CONSOLE.ERROR: " + m.text()); });
await p.goto(base + "/", { waitUntil: "networkidle", timeout: 15000 }).catch(e=>errs.push("GOTO: "+e.message));
await p.waitForTimeout(2500);
const diag = await p.evaluate(() => ({ hasInternals: typeof window.__TAURI_INTERNALS__ })).catch(e => ({evalErr: e.message}));
console.log("BASE:", base);
console.log("DIAG:", JSON.stringify(diag));
console.log("---- ERRORS ----");
console.log(errs.length ? errs.join("\n\n") : "(none)");
await b.close();
srv.kill();
process.exit(0);
