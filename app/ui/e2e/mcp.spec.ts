import { test, expect, type Page } from "@playwright/test";

async function boot(page: Page) {
  await page.goto("/?quiet&fast");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
  await page.getByTestId("nav-settings").click();
  await expect(page.getByTestId("mcp-section")).toBeVisible();
}

const mock = (page: Page) => page.evaluate(() => ({ ...window.__waMock!.mcp }));

test.describe("MCP server", () => {
  test("is on by default, explained, and shows its URL and status", async ({ page }) => {
    await boot(page);
    const sw = page.getByTestId("mcp-section").getByRole("switch", { name: "MCP server" });
    await expect(sw).toHaveAttribute("aria-checked", "true");
    await expect(page.getByTestId("mcp-help")).toContainText("Model Context Protocol");
    await expect(page.getByTestId("mcp-help")).toContainText("turn it off");
    await expect(page.getByTestId("mcp-port")).toHaveValue("8771");
    await expect(page.getByTestId("mcp-url")).toHaveText("http://127.0.0.1:8771/mcp");
    await expect(page.getByTestId("mcp-status")).toContainText("Running (pid 4343)");
  });

  test("the toggle applies at once, without saving the form", async ({ page }) => {
    await boot(page);
    const sw = page.getByTestId("mcp-section").getByRole("switch", { name: "MCP server" });
    await sw.click();
    await expect(sw).toHaveAttribute("aria-checked", "false");
    await expect(page.getByTestId("mcp-status")).toContainText("Off");
    expect((await mock(page)).state).toBe("disabled");
    await expect.poll(() => page.evaluate(() => window.__waMock!.settings.mcp_enabled)).toBe(false);
    await expect(page.getByTestId("save-settings")).toBeDisabled(); // nothing left unsaved

    await sw.click();
    await expect(page.getByTestId("mcp-status")).toContainText("Running");
    expect((await mock(page)).state).toBe("running");
  });

  test("changing the port restarts on the new port and updates the URL", async ({ page }) => {
    await boot(page);
    const port = page.getByTestId("mcp-port");
    await port.fill("9100");
    await port.press("Enter");
    await expect(page.getByTestId("mcp-url")).toHaveText("http://127.0.0.1:9100/mcp");
    await expect.poll(() => mock(page).then((m) => m.pid)).toBe(4344);
    await expect.poll(() => page.evaluate(() => window.__waMock!.settings.mcp_port)).toBe(9100);
    await expect(page.getByTestId("save-settings")).toBeDisabled();
  });

  test("an invalid port is flagged and not applied", async ({ page }) => {
    await boot(page);
    const port = page.getByTestId("mcp-port");
    await port.fill("80");
    await port.press("Enter");
    await expect(page.getByRole("alert").filter({ hasText: "1024" })).toBeVisible();
    expect(await page.evaluate(() => window.__waMock!.settings.mcp_port)).toBe(8771);
  });

  test("the URL can be copied", async ({ page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await boot(page);
    await page.getByTestId("mcp-copy").click();
    await expect(page.getByTestId("toast").filter({ hasText: "MCP URL copied" })).toBeVisible();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("http://127.0.0.1:8771/mcp");
  });

  test("the status line follows needs-uv, port-busy and gave-up", async ({ page }) => {
    await boot(page);
    const line = page.getByTestId("mcp-status");
    const url = "http://127.0.0.1:8771/mcp";
    await page.evaluate((u) => { window.__waMock!.mcp = { state: "needs_uv", pid: null, url: u, detail: null, restarts: 0 }; }, url);
    await expect(line).toContainText("Needs uv", { timeout: 6000 });
    await expect(line).toContainText("https://docs.astral.sh/uv/");
    await page.evaluate((u) => { window.__waMock!.mcp = { state: "port_busy", pid: null, url: u, detail: "Port 8771 is in use by another program; pick another port.", restarts: 0 }; }, url);
    await expect(line).toContainText("Port in use", { timeout: 6000 });
    await page.evaluate((u) => { window.__waMock!.mcp = { state: "starting", pid: 9, url: u, detail: "Starting (first run installs dependencies)", restarts: 0 }; }, url);
    await expect(line).toContainText("first run installs dependencies", { timeout: 6000 });
    await page.evaluate((u) => { window.__waMock!.mcp = { state: "gave_up", pid: null, url: u, detail: "exited with exit status: 1", restarts: 8 }; }, url);
    await expect(line).toContainText("Keeps crashing", { timeout: 6000 });
  });

  test("the System view has a one-line MCP status", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-system").click();
    const line = page.getByTestId("mcp-system-line");
    await expect(line).toContainText("MCP server");
    await expect(line).toContainText("Running (pid 4343)");
    await expect(line).toContainText("http://127.0.0.1:8771/mcp");
  });
});
