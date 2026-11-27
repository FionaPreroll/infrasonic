// SPDX-License-Identifier: GPL-3.0-only
// End-to-end test of the web player in a real browser against real servers.
//
//   node crates/web/tests/browser.mjs <player URL> [chromium|firefox|webkit] [--mobile]
//
// Servers come from INFRASONIC_E2E_SERVERS ("navidrome=http://...;gonic=http://..."), as set
// by scripts/e2e-servers.sh; the user is admin/admin. The player must be served from
// 127.0.0.1 or localhost (no service worker there), or from a LAN address over plain HTTP
// (no service worker and no AudioWorklet there).
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const playwright = require("playwright");

const [url, engine = "chromium", ...flags] = process.argv.slice(2);
const mobile = flags.includes("--mobile");
const servers = (process.env.INFRASONIC_E2E_SERVERS || "")
  .split(";")
  .filter(Boolean)
  .map((s) => s.split("="));
if (!url || servers.length === 0) {
  console.error("usage: INFRASONIC_E2E_SERVERS=... node browser.mjs <url> [browser] [--mobile]");
  process.exit(2);
}

const launch = {
  chromium: { args: ["--autoplay-policy=no-user-gesture-required"] },
  firefox: { firefoxUserPrefs: { "media.autoplay.default": 0, "media.autoplay.block-webaudio": false } },
  webkit: {},
}[engine];
const browser = await playwright[engine].launch(launch);
const context = await browser.newContext(
  mobile ? { ...playwright.devices["Pixel 7"] } : { viewport: { width: 1280, height: 860 } },
);

let failed = 0;
const errors = [];

async function step(name, fn) {
  const t0 = Date.now();
  try {
    await fn();
    console.log(`  ok   ${name} (${Date.now() - t0} ms)`);
  } catch (e) {
    failed++;
    console.log(`  FAIL ${name}: ${String(e.message || e).split("\n")[0]}`);
  }
}

/** Seconds shown in the player bar. */
async function position(page) {
  const text = (await page.textContent("#position")) || "0:00";
  return text.split(":").reduce((acc, x) => acc * 60 + Number(x), 0);
}

async function waitPlaying(page, beyond = 1, timeout = 30000) {
  await waitPlayingInner(page, beyond, timeout).catch(async (e) => {
    const state = await page.evaluate(() => ({
      position: document.querySelector("#position")?.textContent,
      button: document.querySelector("#play")?.textContent,
      title: document.querySelector(".player .title")?.textContent,
      toast: document.querySelector(".toast")?.textContent,
    }));
    throw new Error(`not playing: ${JSON.stringify(state)} (${String(e.message).split("\n")[0]})`);
  });
}

async function waitPlayingInner(page, beyond, timeout) {
  await page.waitForFunction(
    (s) => {
      const t = document.querySelector("#position")?.textContent || "0:00";
      const secs = t.split(":").reduce((a, x) => a * 60 + Number(x), 0);
      return secs >= s && document.querySelector("#play")?.textContent.includes("⏸");
    },
    beyond,
    { timeout },
  );
}

async function nowPlaying(page) {
  return (await page.textContent(".player .title")) || "";
}

async function clickNav(page, hash) {
  if (mobile) {
    await page.evaluate((h) => { location.hash = h; }, hash);
  } else {
    await page.click(`.sidebar a[href="${hash}"]`);
  }
}

for (const [name, server] of servers) {
  console.log(`${engine}${mobile ? " (mobile)" : ""} against ${name}`);
  const page = await context.newPage();
  page.on("pageerror", (e) => errors.push(`${name}: pageerror: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(`${name}: console: ${m.text()}`);
  });
  page.on("dialog", (d) => d.accept());

  await step("log in", async () => {
    await page.goto(url + "#/login");
    await page.fill("#url", server);
    await page.fill("#user", "admin");
    await page.fill("#password", "admin");
    await page.click("#login");
    await page.waitForSelector(".grid .card", { timeout: 20000 });
  });

  await step("password is not stored", async () => {
    const stored = JSON.parse(await page.evaluate(() => localStorage.getItem("infrasonic.profiles")));
    if (!stored?.list?.length) throw new Error("no profile saved");
    for (const p of stored.list) {
      for (const [k, v] of Object.entries(p)) {
        if (k !== "username" && v === "admin") throw new Error(`password stored as ${k}`);
      }
    }
  });

  await step("album list tabs and genre", async () => {
    await page.click('.tabs a[href="#/albums/alphabeticalByName"]');
    await page.waitForFunction(() => location.hash === "#/albums/alphabeticalByName");
    // The grid of the previous tab may still show for a moment.
    await page.waitForFunction(() => {
      const names = [...document.querySelectorAll(".grid .card .name")].map((e) => e.textContent);
      const sorted = [...names].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
      return names.length >= 3 && JSON.stringify(names) === JSON.stringify(sorted);
    }, null, { timeout: 10000 });
  });

  await step("album page plays through the engine", async () => {
    await page.click('.grid .card .name:text("Sine Studies")');
    await page.waitForSelector("table.songs tr");
    await page.click(".actions-row button.primary");
    await waitPlaying(page, 2);
    const title = await nowPlaying(page);
    if (!title.includes("Low A")) throw new Error(`playing "${title}"`);
  });

  await step("pause, resume and seek", async () => {
    await page.keyboard.press("Space");
    await page.waitForFunction(() => document.querySelector("#play").textContent.includes("▶"));
    const p0 = await position(page);
    await page.waitForTimeout(1200);
    if ((await position(page)) !== p0) throw new Error("position moved while paused");
    await page.keyboard.press("Space");
    await waitPlaying(page, p0);
  });

  await step("next song (gapless queue)", async () => {
    await page.keyboard.press("Shift+ArrowRight");
    await page.waitForFunction(() => document.querySelector(".player .title")?.textContent.includes("Middle A"));
    const p = await position(page);
    if (p > 2) throw new Error(`new song starts at ${p} s`);
    await waitPlaying(page, 1);
  });

  await step("every format plays through the engine", async () => {
    await page.evaluate(() => { location.hash = "#/search/Song"; });
    await page.waitForFunction(() => document.querySelectorAll("table.songs tr").length >= 4);
    const titles = await page.$$eval("table.songs .title > a", (e) => e.map((x) => x.textContent));
    const formats = titles.filter((t) => / Song$/.test(t));
    if (formats.length !== 4) throw new Error(`songs: ${titles}`);
    await page.click(`table.songs .title > a:text-is("${titles[0]}")`);
    for (let i = 0; i < titles.length; i++) {
      await page.waitForFunction((t) => document.querySelector(".player .title")?.textContent === t, titles[i]);
      await waitPlaying(page, 1);
      if (i + 1 < titles.length) await page.keyboard.press("Shift+ArrowRight");
    }
  });

  await step("queue shows the album and moves songs", async () => {
    await page.evaluate(() => { location.hash = "#/albums/alphabeticalByName"; });
    await page.click('.grid .card .name:text("Sine Studies")');
    await page.waitForSelector("table.songs tr");
    await page.click("table.songs tr:nth-child(2) .title > a");
    await page.waitForFunction(() => document.querySelector(".player .title")?.textContent.includes("Middle A"));
    await clickNav(page, "#/queue");
    await page.waitForSelector("table.songs tr");
    const rows = await page.$$eval("table.songs tr .title > a", (els) => els.map((e) => e.textContent));
    if (rows.length !== 3) throw new Error(`queue has ${rows.length} rows`);
    await page.click("table.songs tr:nth-child(3) button[title]:has-text('↑')");
    await page.waitForFunction(() =>
      document.querySelectorAll("table.songs tr .title > a")[1]?.textContent.includes("High A"));
  });

  await step("star the playing song (l) and see it in favorites", async () => {
    await page.keyboard.press("l");
    await page.waitForSelector(".player .star.on");
    await clickNav(page, "#/favorites");
    await page.waitForSelector("table.songs tr");
    const t = await page.textContent("table.songs");
    if (!t.includes("Middle A")) throw new Error("starred song missing");
    await page.keyboard.press("l");
    await page.waitForSelector(".player .star:not(.on)");
  });

  await step("artist page", async () => {
    await page.click(".player .artist");
    await page.waitForSelector(".head h1");
    const h = await page.textContent(".head h1");
    if (!h.includes("Infrasonic Test Ensemble")) throw new Error(`artist page shows ${h}`);
    await page.waitForSelector(".grid .card");
  });

  await step("search", async () => {
    if (mobile) {
      await page.evaluate(() => { location.hash = "#/search/Vorbis"; });
    } else {
      await page.fill("#search", "Vorbis");
      await page.press("#search", "Enter");
    }
    await page.waitForSelector("table.songs tr");
    if (!(await page.textContent("table.songs")).includes("Vorbis Song")) throw new Error("no hit");
  });

  await step("playlist: create, add songs, reorder, delete", async () => {
    await clickNav(page, "#/playlists");
    const pl = `e2e ${engine} ${Date.now()}`;
    await page.fill("form.row input", pl);
    await page.click("form.row button[type=submit]");
    await page.waitForSelector(`.card .name:text("${pl}")`);
    await page.evaluate(() => { location.hash = "#/search/Song"; });
    await page.waitForSelector("table.songs tr");
    for (const n of [1, 2]) {
      await page.click(`table.songs tr:nth-child(${n}) button[title]:has-text("≣")`);
      await page.click(`.dialog .pick button:text("${pl}")`);
      await page.waitForSelector(".dialog", { state: "detached" });
    }
    await clickNav(page, "#/playlists");
    await page.click(`.card .name:text("${pl}")`);
    await page.waitForSelector("table.songs tr:nth-child(2)");
    const before = await page.$$eval("table.songs .title > a", (e) => e.map((x) => x.textContent));
    if (!mobile) {
      await page.dragAndDrop("table.songs tr:nth-child(2)", "table.songs tr:nth-child(1)");
      await page.waitForFunction((first) =>
        document.querySelector("table.songs .title > a")?.textContent === first, before[1]);
    }
    await page.click('button[title]:has-text("🗑")');
    await page.waitForFunction(() => location.hash === "#/playlists");
  });

  await step("lyrics page", async () => {
    await clickNav(page, "#/lyrics");
    await page.waitForSelector("main h1");
    await page.waitForFunction(() => !document.querySelector("main").textContent.includes("…"), null, { timeout: 15000 });
  });

  await step("internet radio: add and remove a station", async () => {
    await clickNav(page, "#/radio");
    const n = `e2e station ${engine}`;
    await page.fill("form.stack input >> nth=0", n);
    await page.fill("form.stack input >> nth=1", "https://radio.example.org/stream");
    await page.click("form.stack button[type=submit]");
    const row = page.locator("table.songs tr", { hasText: n });
    await row.waitFor();
    await row.locator('button:has-text("✕")').click();
    await row.waitFor({ state: "detached" });
  });

  if (name === "navidrome") {
    await step("share an album", async () => {
      await page.evaluate(() => { location.hash = "#/albums/alphabeticalByName"; });
      await page.click('.grid .card .name:text("Sine Studies")');
      await page.click('.head button[title]:has-text("🔗")');
      await page.waitForSelector(".toast");
      await clickNav(page, "#/shares");
      const row = page.locator("table.songs tr", { hasText: "Sine Studies" });
      await row.waitFor();
      await row.locator('button:has-text("✕")').click();
      await row.waitFor({ state: "detached" });
    });
  }

  if (!(await page.evaluate(() => isSecureContext))) {
    // Plain HTTP in a home network: no AudioWorklet, so the page must have fallen back.
    await step("plain HTTP falls back to the audio element", async () => {
      await clickNav(page, "#/settings");
      await page.waitForSelector(".hint.warn");
      const t = await page.textContent("main");
      if (!t.includes("audio element")) throw new Error("engine still active");
    });
  }

  await step("browser backend (audio element) plays", async () => {
    await clickNav(page, "#/settings");
    await page.selectOption("#backend", "element");
    await page.evaluate(() => { location.hash = "#/albums/alphabeticalByName"; });
    await page.click('.grid .card .name:text("Black Album")');
    await page.waitForSelector("table.songs tr");
    await page.click(".actions-row button.primary");
    await waitPlaying(page, 2);
    await clickNav(page, "#/settings");
    await page.waitForFunction(() => document.querySelector("main").textContent.includes("audio element") ||
      document.querySelector("main").textContent.includes("Audio-Element"));
    await page.selectOption("#backend", "auto");
  });

  await step("German texts", async () => {
    await page.evaluate(() => { location.hash = "#/settings"; });
    await page.selectOption("#language", "de");
    await page.waitForFunction(() => document.querySelector("main h1")?.textContent === "Einstellungen", null, { timeout: 20000 });
    await page.selectOption("#language", "en");
    await page.waitForFunction(() => document.querySelector("main h1")?.textContent === "Settings", null, { timeout: 20000 });
  });

  await step("reload keeps the login", async () => {
    await page.reload();
    await page.waitForSelector(".layout");
  });

  await step("log out forgets the server", async () => {
    await page.evaluate(() => { location.hash = "#/settings"; });
    await page.click("text=Log out and forget this server");
    await page.waitForSelector("#login");
    const stored = await page.evaluate(() => localStorage.getItem("infrasonic.profiles"));
    if (stored.includes(server)) throw new Error("profile still stored");
  });
  await page.close();
}

await browser.close();
const relevant = errors.filter((e) => !/favicon|ERR_ABORTED/.test(e));
if (relevant.length) {
  console.log("browser errors:\n  " + relevant.slice(0, 20).join("\n  "));
}
console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
