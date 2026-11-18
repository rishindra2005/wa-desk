import { test, expect, type Page } from "@playwright/test";

async function boot(page: Page) {
  await page.goto("/?quiet&fast");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
}

test.describe("desktop notifications", () => {
  test("clicking a notification opens its chat and marks it read", async ({ page }) => {
    await boot(page);
    const { jid, name } = await page.evaluate(() => {
      const c = window.__waMock!.chats.find((x) => x.unread && x.chat_type === "direct")!;
      return { jid: c.jid, name: c.name };
    });
    // Elsewhere in the app when the notification is clicked.
    await page.getByTestId("nav-settings").click();
    await page.evaluate((j) => window.__waMock!.clickNotification(j), jid);
    await expect(page.getByTestId("thread-title")).toHaveText(name);
    await expect.poll(() => page.evaluate(() => window.__waMock!.markedRead)).toContain(jid);
    const row = page.getByTestId("chat-row").filter({ has: page.getByTestId("chat-name").getByText(name, { exact: true }) });
    await expect(row.getByTestId("unread-dot")).toHaveCount(0);
  });

  test("a new message in the open chat of a focused window is marked read", async ({ page }) => {
    await boot(page);
    const { jid, name } = await page.evaluate(() => {
      const c = window.__waMock!.chats.find((x) => !x.unread && x.chat_type === "direct")!;
      return { jid: c.jid, name: c.name };
    });
    await page.getByTestId("chat-row").filter({ has: page.getByTestId("chat-name").getByText(name, { exact: true }) }).click();
    await expect(page.getByTestId("thread-title")).toHaveText(name);
    const before = await page.evaluate((j) => window.__waMock!.markedRead.filter((x) => x === j).length, jid);
    await page.evaluate((j) => window.__waMock!.incoming(j, "are you there?"), jid);
    await expect
      .poll(() => page.evaluate((j) => window.__waMock!.markedRead.filter((x) => x === j).length, jid))
      .toBeGreaterThan(before);
    await expect.poll(() => page.evaluate((j) => window.__waMock!.chats.find((c) => c.jid === j)!.unread, jid)).toBe(false);
  });
});
