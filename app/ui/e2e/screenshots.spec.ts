import { test, expect, type Page } from "@playwright/test";

// Captures each main view in light and dark for visual review (app/ui/screenshots/).

const VIEWS = ["chats", "calls", "search", "media", "docs", "tools", "automations", "system", "settings"] as const;

async function prepare(page: Page, view: (typeof VIEWS)[number]) {
  await page.getByTestId(`nav-${view}`).click();
  await expect(page.getByTestId(`view-${view}`)).toBeVisible();
  switch (view) {
    case "chats":
      await page.getByTestId("chat-row").nth(2).click();
      await expect(page.getByTestId("message").first()).toBeVisible();
      break;
    case "calls":
      await expect(page.getByTestId("call-row").first()).toBeVisible();
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

async function openChat(page: Page, name: string) {
  await page.getByTestId("chat-filter").fill(name);
  await page.getByTestId("chat-row").filter({ hasText: name }).first().click();
  await expect(page.getByTestId("thread-title")).toContainText(name);
  await expect(page.getByTestId("message").first()).toBeVisible();
}

for (const scheme of ["light", "dark"] as const) {
  test(`screenshots (${scheme})`, async ({ page }) => {
    test.setTimeout(120_000);
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto("/?quiet");
    await expect(page.getByTestId("chat-row").first()).toBeVisible();
    for (const v of VIEWS) {
      await prepare(page, v);
      await page.screenshot({ path: `screenshots/${v}-${scheme}.png` });
    }

    // Ringing call banner over the chats view.
    await page.getByTestId("nav-chats").click();
    await page.evaluate(() => {
      const m = (window as unknown as { __waMock: { ring: (o: object) => unknown; chats: { name: string; jid: string }[] } }).__waMock;
      m.ring({ chatJid: m.chats.find((x) => x.name === "Ananya Rao")?.jid, media: "video" });
    });
    await expect(page.getByTestId("call-banner")).toBeVisible();
    await page.waitForTimeout(500);
    await page.screenshot({ path: `screenshots/call-banner-${scheme}.png` });
    await page.getByTestId("dismiss-call").click();

    // Profile drawer: contact, then group.
    await page.getByTestId("nav-chats").click();
    await openChat(page, "Ananya Rao");
    await page.getByTestId("thread-header").click();
    await expect(page.getByTestId("profile-name")).toHaveText("Ananya Rao");
    await page.waitForTimeout(400);
    await page.screenshot({ path: `screenshots/profile-contact-${scheme}.png` });

    await openChat(page, "Startup Founders");
    await expect(page.getByTestId("profile-subtitle")).toContainText("250 participants");
    await page.waitForTimeout(400);
    await page.screenshot({ path: `screenshots/profile-group-${scheme}.png` });
    await page.getByTestId("profile-close").click();

    // Archived view.
    await page.getByTestId("nav-chats").click();
    await page.getByTestId("chat-filter").fill("");
    await page.getByTestId("archived-row").click();
    await expect(page.getByTestId("archived-title")).toBeVisible();
    await expect(page.getByTestId("chat-row").first()).toBeVisible();
    await page.mouse.move(1000, 30);
    await page.waitForTimeout(300);
    await page.screenshot({ path: `screenshots/archived-${scheme}.png` });
    await page.getByTestId("archived-back").click();

    // Context menu on a message.
    await openChat(page, "Ananya Rao");
    await page.getByTestId("message").last().click({ button: "right", position: { x: 40, y: 10 } });
    await expect(page.getByTestId("context-menu")).toBeVisible();
    await page.screenshot({ path: `screenshots/context-menu-${scheme}.png` });
    await page.keyboard.press("Escape");

    // Selection mode.
    await page.getByTestId("message").last().click({ button: "right", position: { x: 40, y: 10 } });
    await page.getByTestId("menu-select").click();
    await page.getByTestId("message").nth(-2).click();
    await expect(page.getByTestId("selection-bar")).toBeVisible();
    await page.mouse.move(1000, 30);
    await page.screenshot({ path: `screenshots/selection-${scheme}.png` });
    await page.getByTestId("sel-cancel").click();

    // Photo albums in a group, then the combined download dialog.
    await openChat(page, "Family");
    await expect(page.getByTestId("album")).toHaveCount(2);
    await page.mouse.move(1000, 30);
    await page.waitForTimeout(400);
    await page.screenshot({ path: `screenshots/album-${scheme}.png` });
    const bubble = page.locator('[data-testid="message"][data-id="ALBUM-B3"]');
    await bubble.hover();
    await bubble.getByTestId("album-download-all").click();
    await expect(page.getByTestId("export-dialog")).toBeVisible();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `screenshots/download-dialog-${scheme}.png` });
  });
}
