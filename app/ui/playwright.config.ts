import { defineConfig, devices } from "@playwright/test";

// E2E against the Vite dev server with the in-memory mock backend.
export default defineConfig({
  testDir: "e2e",
  timeout: 30_000,
  expect: { timeout: 8_000 },
  fullyParallel: true,
  workers: 4,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:1420",
    viewport: { width: 1320, height: 860 },
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1320, height: 860 } } }],
  webServer: {
    command: "npx vite --port 1420 --strictPort --host 127.0.0.1",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
