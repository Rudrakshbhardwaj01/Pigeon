import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  timeout: 120_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:4173",
    // Grant fake camera streams: Playwright's Chromium supports a fake
    // device, so camera tests run without physical hardware.
    contextOptions: {
      permissions: ["camera"],
    },
  },
  webServer: {
    command: "npm run preview",
    port: 4173,
    reuseExistingServer: true,
    timeout: 30_000,
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
  ],
});
