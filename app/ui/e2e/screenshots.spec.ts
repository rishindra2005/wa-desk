import { test, expect, type Page } from "@playwright/test";

// Captures each main view in light and dark for visual review (app/ui/screenshots/).

const VIEWS = ["chats", "search", "media", "docs", "tools", "automations", "system", "settings"] as const;

async function prepare(page: Page, view: (typeof VIEWS)[number]) {
  await page.getByTestId(`nav-${view}`).click();
  await expect(page.getByTestId(`view-${view}`)).toBeVisible();
  switch (view) {
    case "chats":
      await page.getByTestId("chat-row").nth(2).click();
      await expect(page.getByTestId("message").first()).toBeVisible();
      break;
    case "search":
      await page.getByTestId("search-input").fill("birthday");
      await expect(page.getByTestId("search-hit").first()).toBeVisible();
      break;
    case "media":
      await expect(page.getByTestId("media-tile").first()).toBeVisible();
      break;
    case "docs":
      await page.getByRole("tab", { name: "PDF" }).click();
      await page.getByTestId("doc-row").filter({ hasText: "Local" }).first().click();
      await expect(page.getByTestId("doc-panel").locator("canvas[data-rendered]").first()).toBeVisible();
      break;
    case "system":
      await expect(page.getByTestId("bridge-state")).toHaveText("Connected");
      break;
    default:
      break;
  }
  await page.mouse.move(1000, 30);
  await page.waitForTimeout(400);
}

for (const scheme of ["light", "dark"] as const) {
  test(`screenshots (${scheme})`, async ({ page }) => {
    test.setTimeout(90_000);
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto("/?quiet");
    await expect(page.getByTestId("chat-row").first()).toBeVisible();
    for (const v of VIEWS) {
      await prepare(page, v);
      await page.screenshot({ path: `screenshots/${v}-${scheme}.png` });
    }
  });
}
