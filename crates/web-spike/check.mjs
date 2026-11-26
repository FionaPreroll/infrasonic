// SPDX-License-Identifier: GPL-3.0-only
// Opens the spike page in headless Chromium and checks every format decoded correctly.
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const url = process.argv[2];
const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
const page = await browser.newPage();
const logs = [];
page.on("console", (m) => logs.push(`${m.type()}: ${m.text()}`));
page.on("pageerror", (e) => logs.push(`pageerror: ${e}`));
await page.goto(url);
await page.waitForFunction(() => window.spikeResults, null, { timeout: 60000 });
const { rate, results } = await page.evaluate(() => window.spikeResults);
await page.click("#play");
await page.waitForTimeout(500);
const state = await page.evaluate(() => document.querySelector("#play") && "clicked");
await browser.close();

console.log(`output rate ${rate} Hz`);
for (const r of results) {
  console.log(r.error
    ? `${r.hint}: ERROR ${r.error}`
    : `${r.hint}: ${r.seconds.toFixed(3)} s from ${r.sourceRate} Hz, peak ${r.peak.toFixed(3)}, ` +
      `decode ${r.decodeMs.toFixed(0)} ms, with resampling ${r.ms.toFixed(0)} ms ` +
      `(${(r.seconds * 1000 / r.ms).toFixed(0)}x real time) ${r.ok ? "ok" : "WRONG"}`);
}
if (logs.length) console.log(logs.join("\n"));
if (!results.every((r) => r.ok) || state !== "clicked") process.exit(1);
