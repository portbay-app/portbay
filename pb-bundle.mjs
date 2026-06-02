import { chromium } from '@playwright/test';
const browser = await chromium.launch();
const page = await browser.newPage();
const urls = [];
page.on('response', r => { const u=r.url(); if(/\/_app\/.*\.js$/.test(u)) urls.push(u); });
try { await page.goto('https://try.portbay.app/', { waitUntil:'domcontentloaded', timeout:30000 }); } catch(e){ console.log('nav',e.message.split('\n')[0]); }
await page.waitForTimeout(4000);
let install=false, sim=false;
for (const u of urls) {
  try { const txt = await page.evaluate(async(x)=>{ const r=await fetch(x); return await r.text(); }, u);
    if (/__TAURI_INTERNALS__\s*=/.test(txt)) install=true;
    if (/installSimulator/.test(txt)) sim=true;
  } catch(e){}
}
console.log('chunks:'+urls.length, 'mockAssign:'+install, 'installSimulatorSymbol:'+sim);
await browser.close();
