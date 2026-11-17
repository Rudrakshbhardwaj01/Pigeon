// Regenerates the README screenshots (docs/img/*.png) from the running
// preview server (npm run preview → http://127.0.0.1:4173).
//
// Usage:
//   npm run preview &        # serve apps/web/dist
//   node scripts/capture-screenshots.mjs
//
// The screenshots always show the real application — never mock-ups.

import { chromium } from "@playwright/test";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outDir = path.join(repoRoot, "docs", "img");
mkdirSync(outDir, { recursive: true });

const BASE = process.env.PIGEON_PREVIEW_URL ?? "http://127.0.0.1:4173";

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
await page.goto(BASE, { waitUntil: "networkidle" });
// Give the WASM engine time to initialize so the page is in its steady state.
await page.waitForTimeout(2500);

async function shot(name) {
  const target = path.join(outDir, name);
  await page.screenshot({ path: target });
  console.log(`saved ${target}`);
}

// 1. Hero / landing.
await shot("hero.png");

// 2. Send section with the encryption UI open: presets, fields, passphrase.
await page.locator("#send").scrollIntoViewIfNeeded();
await page.waitForTimeout(400);
await page
  .getByRole("button", { name: /encryption off/i })
  .click();
await page.getByLabel("Passphrase", { exact: true }).click();
await page.waitForTimeout(400);
await shot("send.png");

// 3. Receive section (GIF import mode), framed from the top of its card.
await page.locator("#receive").evaluate((el) =>
  el.scrollIntoView({ block: "start" })
);
await page.waitForTimeout(400);
await shot("receive.png");

await browser.close();
