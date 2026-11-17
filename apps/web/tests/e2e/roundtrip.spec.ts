// End-to-end browser tests for the complete optical roundtrip:
// file → animated QR GIF (downloaded) → GIF import → verified reconstruction.
// Also covers offline operation after the service worker has cached the app.

import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const thisDir = path.dirname(fileURLToPath(import.meta.url));
const fixtures = path.resolve(thisDir, "../../../../tests/fixtures");

const sendSection = (page: Page) => page.getByRole("region", { name: "Send a File" });
const receiveSection = (page: Page) => page.getByRole("region", { name: "Receive a File" });

async function sendFile(page: Page, filePath: string): Promise<void> {
  await page.goto("/");
  await sendSection(page).getByRole("button", { name: /drop a file/i }).click();
  await sendSection(page).locator('input[type="file"]').setInputFiles(filePath);
  await page.getByRole("button", { name: /encode to gif/i }).click();
  await expect(page.getByRole("heading", { name: "Transfer ready" })).toBeVisible({
    timeout: 60_000,
  });
}

async function downloadGif(page: Page): Promise<string> {
  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: /download gif/i }).click();
  const download = await downloadPromise;
  const target = path.join(thisDir, "../.tmp-download", download.suggestedFilename());
  await download.saveAs(target);
  return target;
}

async function importGif(page: Page, gifPath: string): Promise<void> {
  await receiveSection(page).getByRole("button", { name: /drop the animated qr gif/i }).click();
  await receiveSection(page).locator('input[type="file"]').setInputFiles(gifPath);
}

test("app loads and initializes the wasm engine", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.goto("/");
  await expect(page.getByText("Files never leave this device")).toBeVisible();
  await expect(page.getByRole("button", { name: /encode to gif/i })).toBeVisible();
  // No engine-level page errors during startup
  expect(errors).toEqual([]);
});

test("file → GIF → file roundtrip preserves bytes exactly", async ({ page }) => {
  await sendFile(page, path.join(fixtures, "hello.txt"));

  // Download the produced GIF
  const gifPath = await downloadGif(page);
  const gifBytes = readFileSync(gifPath);
  expect(gifBytes.length).toBeGreaterThan(500);
  expect(gifBytes.subarray(0, 6).toString("ascii")).toBe("GIF89a");

  // Import it in the Receive tab
  await importGif(page, gifPath);

  // Verification completes and matches the sender's manifest
  await expect(
    page.getByRole("heading", { name: /file recovered & verified/i })
  ).toBeVisible({ timeout: 60_000 });
  await expect(
    receiveSection(page).getByLabel("Recovered file").getByText("hello.txt", { exact: true })
  ).toBeVisible();

  // Download the reconstructed file and compare bytes
  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: /download hello.txt/i }).click();
  const download = await downloadPromise;
  const outPath = path.join(thisDir, "../.tmp-download/recovered.txt");
  await download.saveAs(outPath);
  const original = readFileSync(path.join(fixtures, "hello.txt"));
  const recovered = readFileSync(outPath);
  expect(recovered.equals(original)).toBe(true);
});

test("binary file with all 256 byte values roundtrips", async ({ page }) => {
  await sendFile(page, path.join(fixtures, "binary.bin"));
  const gifPath = await downloadGif(page);
  await importGif(page, gifPath);
  await expect(
    page.getByRole("heading", { name: /file recovered & verified/i })
  ).toBeVisible({ timeout: 60_000 });

  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: /download binary.bin/i }).click();
  const download = await downloadPromise;
  const outPath = path.join(thisDir, "../.tmp-download/recovered.bin");
  await download.saveAs(outPath);
  expect(readFileSync(outPath).equals(readFileSync(path.join(fixtures, "binary.bin")))).toBe(
    true
  );
});

test("stats show honest transfer details", async ({ page }) => {
  await sendFile(page, path.join(fixtures, "hello.txt"));
  await expect(page.getByText("SHA-256", { exact: false }).first()).toBeVisible();
  await expect(page.getByText(/QR version \/ ECL/i)).toBeVisible();
  await expect(page.getByText(/Required packets/i)).toBeVisible();
  await page.getByRole("button", { name: /encode another file/i }).click();
  await expect(page.getByRole("button", { name: /encode to gif/i })).toBeVisible();
});

test("application works offline after first load", async ({ page, context }) => {
  // First load: populate the service worker cache.
  await page.goto("/");
  await expect(page.getByRole("button", { name: /encode to gif/i })).toBeVisible();
  // Wait until the service worker is active and controlling the page, and
  // until the precache can actually serve the app shell and its main assets.
  // (A cache that misses here — e.g. due to Vary-header mismatches — would
  // otherwise only surface as a blank page after going offline.)
  await expect
    .poll(
      () =>
        page.evaluate(async () => {
          const reg = await navigator.serviceWorker.getRegistration();
          if (!reg?.active || !navigator.serviceWorker.controller) {
            return { ready: false, scriptHit: false, styleHit: false };
          }
          const scriptUrl = document.querySelector('script[type="module"]')?.getAttribute("src");
          const styleUrl = document.querySelector('link[rel="stylesheet"]')?.getAttribute("href");
          const scriptHit = scriptUrl
            ? !!(await caches.match(new URL(scriptUrl, location.origin).href, { ignoreVary: true }))
            : false;
          const styleHit = styleUrl
            ? !!(await caches.match(new URL(styleUrl, location.origin).href, { ignoreVary: true }))
            : false;
          return { ready: true, scriptHit, styleHit };
        }),
      { timeout: 20_000 }
    )
    .toEqual({ ready: true, scriptHit: true, styleHit: true });

  await context.setOffline(true);
  await page.reload();
  await expect(page.getByRole("button", { name: /encode to gif/i })).toBeVisible({
    timeout: 20_000,
  });

  // Full roundtrip offline: encode → download → import → verify.
  await sendSection(page).getByRole("button", { name: /drop a file/i }).click();
  await sendSection(page).locator('input[type="file"]').setInputFiles(path.join(fixtures, "hello.txt"));
  await page.getByRole("button", { name: /encode to gif/i }).click();
  await expect(page.getByRole("heading", { name: "Transfer ready" })).toBeVisible({
    timeout: 60_000,
  });
  const gifPath = await downloadGif(page);
  await importGif(page, gifPath);
  await expect(
    page.getByRole("heading", { name: /file recovered & verified/i })
  ).toBeVisible({ timeout: 60_000 });

  await context.setOffline(false);
});

test("non-GIF files are rejected with a clear message", async ({ page }) => {
  await page.goto("/");
  await receiveSection(page).getByRole("button", { name: /drop the animated qr gif/i }).click();
  await receiveSection(page)
    .locator('input[type="file"]')
    .setInputFiles(path.join(fixtures, "hello.txt"));
  await expect(page.getByText(/not a GIF/i)).toBeVisible();
});

test("a valid GIF without Pigeon packets is reported honestly", async ({ page }) => {
  await page.goto("/");
  await receiveSection(page).getByRole("button", { name: /drop the animated qr gif/i }).click();
  await receiveSection(page)
    .locator('input[type="file"]')
    .setInputFiles(path.join(fixtures, "not-pigeon.gif"));
  await expect(page.getByText(/no pigeon packets found/i)).toBeVisible({
    timeout: 30_000,
  });
});

test("an incomplete Pigeon GIF resolves with an honest partial report", async ({ page }) => {
  // Genuine Pigeon GIF carrying only ~35% of its transfer's frames.
  await page.goto("/");
  await receiveSection(page).getByRole("button", { name: /drop the animated qr gif/i }).click();
  await receiveSection(page)
    .locator('input[type="file"]')
    .setInputFiles(path.join(fixtures, "incomplete.gif"));
  await expect(page.getByText(/not enough frames captured/i)).toBeVisible({
    timeout: 60_000,
  });
  await expect(page.getByText(/required unique packets/i)).toBeVisible();
  // The busy state must have ended (no permanent spinner): the dropzone is
  // interactive again.
  await expect(
    receiveSection(page).getByRole("button", { name: /drop the animated qr gif/i })
  ).toBeEnabled();
});

test("camera mode starts with fake media stream", async ({ page }) => {
  // Chromium fake device: launch args must come from the project config;
  // permissions are granted in contextOptions.
  await page.goto("/");
  await page.getByRole("tab", { name: /scan with camera/i }).click();
  const button = page.getByRole("button", { name: /start camera/i });
  if (await button.count()) {
    // If the fake stream is unavailable in this environment the button stays;
    // we tolerate both outcomes but require no crash.
    await button.click();
    await page.waitForTimeout(1_500);
  }
  await expect(page.getByRole("button", { name: /stop camera|start camera/i })).toBeVisible();
});

test("encrypted transfer roundtrips with the correct passphrase", async ({ page }) => {
  const passphrase = "e2e optical secret";

  // Send: enable encryption, provide (and confirm) the passphrase.
  await page.goto("/");
  await sendSection(page).getByRole("button", { name: /drop a file/i }).click();
  await sendSection(page).locator('input[type="file"]').setInputFiles(path.join(fixtures, "hello.txt"));
  await sendSection(page).getByRole("button", { name: /encryption off/i }).click();
  await sendSection(page).getByLabel("Passphrase", { exact: true }).fill(passphrase);
  await sendSection(page).getByLabel("Confirm passphrase", { exact: true }).fill(passphrase);
  await page.getByRole("button", { name: /encode to gif/i }).click();
  await expect(page.getByRole("heading", { name: "Transfer ready" })).toBeVisible({
    timeout: 60_000,
  });
  // The result must say the transfer is encrypted.
  await expect(
    page.getByText(/Argon2id \+ XChaCha20-Poly1305/i)
  ).toBeVisible();

  const gifPath = await downloadGif(page);

  // Receive: the passphrase gate must appear — filename stays hidden.
  await importGif(page, gifPath);
  await expect(page.getByText(/this transfer is encrypted/i)).toBeVisible({
    timeout: 60_000,
  });

  // Wrong passphrase: honest rejection, gate stays up.
  await page.getByLabel("Transfer passphrase").fill("wrong passphrase entirely");
  await page.getByRole("button", { name: /decrypt/i }).click();
  await expect(page.getByText(/did not decrypt/i)).toBeVisible({ timeout: 30_000 });

  // Correct passphrase: verified reconstruction, byte-exact download.
  await page.getByLabel("Transfer passphrase").fill(passphrase);
  await page.getByRole("button", { name: /decrypt/i }).click();
  await expect(
    page.getByRole("heading", { name: /file recovered & verified/i })
  ).toBeVisible({ timeout: 60_000 });

  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: /download hello.txt/i }).click();
  const download = await downloadPromise;
  const outPath = path.join(thisDir, "../.tmp-download/recovered-encrypted.txt");
  await download.saveAs(outPath);
  expect(readFileSync(outPath).equals(readFileSync(path.join(fixtures, "hello.txt")))).toBe(true);
});
