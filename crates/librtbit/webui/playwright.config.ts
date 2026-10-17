import { defineConfig, devices } from "@playwright/test";

const chromiumExecutablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  expect: { timeout: 10_000 },
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://127.0.0.1:3032",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    launchOptions: chromiumExecutablePath
      ? { executablePath: chromiumExecutablePath }
      : undefined,
  },
  webServer: [
    {
      command: "npm run dev:mock -- --host 127.0.0.1",
      url: "http://127.0.0.1:3032/mock.html",
      reuseExistingServer: !process.env.CI,
      timeout: 30_000,
    },
    // The real app shell, with its backend calls routed by the wiring spec
    {
      command: "npm run dev -- --host 127.0.0.1",
      url: "http://127.0.0.1:3031/",
      reuseExistingServer: !process.env.CI,
      timeout: 30_000,
    },
  ],
});
