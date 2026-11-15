import { test, expect } from "@playwright/test";

// Every view must render without page errors or console errors/warnings.
test("views render without console errors or warnings", async ({ page }) => {
  const problems: string[] = [];
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warning") problems.push(`${m.type()}: ${m.text()}`);
  });
  await page.goto("/?quiet");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
  for (const v of ["chats", "search", "media", "docs", "tools", "automations", "system", "settings"]) {
    await page.getByTestId(`nav-${v}`).click();
    await expect(page.getByTestId(`view-${v}`)).toBeVisible();
    await page.waitForTimeout(300);
    expect(problems, `after opening ${v}`).toEqual([]);
  }
});
