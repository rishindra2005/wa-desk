import { test, expect, type Page } from "@playwright/test";

async function boot(page: Page) {
  await page.goto("/?quiet&fast");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
}

test.describe("bundled bridge", () => {
  test("the bridge mode setting is shown, explained and saved", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-settings").click();
    const sel = page.getByTestId("settings-bridge-mode");
    await expect(sel).toHaveValue("auto");
    await expect(page.getByTestId("bridge-mode-help")).toContainText("already running");
    await sel.selectOption("external");
    await expect(page.getByTestId("bridge-mode-help")).toContainText("never starts a bridge");
    await sel.selectOption("bundled");
    await expect(page.getByTestId("bridge-mode-help")).toContainText("port is already taken");
    await page.getByTestId("save-settings").click();
    await expect(page.getByTestId("toast").filter({ hasText: "Settings saved" })).toBeVisible();
    await expect.poll(() => page.evaluate(() => window.__waMock!.settings.bridge_mode)).toBe("bundled");
  });

  test("the System card shows what the supervisor is doing", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-system").click();
    const line = page.getByTestId("bridge-supervisor");
    await expect(line).toContainText("Bundled bridge running (pid 4242)");

    await page.evaluate(() => {
      window.__waMock!.supervisor = { state: "not_installed", pid: null, detail: null, restarts: 0 };
    });
    await expect(line).toContainText("No bundled bridge in this install", { timeout: 6000 });

    await page.evaluate(() => {
      window.__waMock!.supervisor = { state: "gave_up", pid: null, detail: "exited with exit status: 1", restarts: 8 };
    });
    await expect(line).toContainText("keeps crashing", { timeout: 6000 });
    await expect(line).toContainText("exit status: 1");

    await page.evaluate(() => {
      window.__waMock!.supervisor = { state: "external", pid: null, detail: null, restarts: 0 };
    });
    await expect(line).toContainText("bridge you run yourself", { timeout: 6000 });
  });
});
