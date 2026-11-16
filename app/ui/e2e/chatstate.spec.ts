import { test, expect, type Page } from "@playwright/test";

async function boot(page: Page) {
  await page.goto("/?quiet&fast");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
}

const row = (page: Page, name: string) => page.getByTestId("chat-row").filter({ has: page.getByTestId("chat-name").getByText(name, { exact: true }) });

async function mockNames(page: Page, pick: (c: { name: string; archived: boolean; pinned: boolean; muted_until: number | null }) => boolean) {
  return page.evaluate((src) => {
    const f = new Function("c", `return (${src})(c)`) as (c: unknown) => boolean;
    return window.__waMock!.chats.filter((c) => f(c)).map((c) => c.name);
  }, pick.toString());
}

test.describe("archived / pinned / muted chats", () => {
  test("archived row opens the archive, unarchive moves the chat back", async ({ page }) => {
    await boot(page);
    const archived = await mockNames(page, (c) => c.archived);
    expect(archived.length).toBeGreaterThan(0);
    await expect(page.getByTestId("archived-count")).toHaveText(String(archived.length));
    await expect(row(page, archived[0]!)).toHaveCount(0);

    await page.getByTestId("archived-row").click();
    await expect(page.getByTestId("archived-title")).toBeVisible();
    await expect(page.getByTestId("chat-row")).toHaveCount(archived.length);
    await row(page, archived[0]!).click({ button: "right" });
    await expect(page.getByTestId("menu-archive")).toHaveText(/Unarchive/);
    await page.getByTestId("menu-archive").click();
    await expect(row(page, archived[0]!)).toHaveCount(0);
    await page.getByTestId("archived-back").click();
    await expect(page.getByTestId("archived-count")).toHaveText(String(archived.length - 1));
    await expect(row(page, archived[0]!)).toHaveCount(1);
  });

  test("archived row disappears when the archive is emptied", async ({ page }) => {
    await boot(page);
    const archived = await mockNames(page, (c) => c.archived);
    await page.getByTestId("archived-row").click();
    for (let i = 0; i < archived.length; i++) {
      await page.getByTestId("chat-row").first().click({ button: "right" });
      await page.getByTestId("menu-archive").click();
      // The last one leaves the (now empty) archive and returns to the main list.
      if (i < archived.length - 1) await expect(page.getByTestId("chat-row")).toHaveCount(archived.length - i - 1);
    }
    await expect(page.getByTestId("archived-title")).toHaveCount(0);
    await expect(page.getByTestId("archived-row")).toHaveCount(0);
    await expect(page.getByTestId("chat-row").first()).toBeVisible();
  });

  test("pinned chats sort first; the fourth pin shows the limit toast", async ({ page }) => {
    await boot(page);
    await expect(page.getByTestId("pinned-icon")).toHaveCount(1);
    const names = await page.getByTestId("chat-name").allTextContents();
    const a = names[6]!, b = names[7]!, c = names[8]!;
    for (const n of [a, b]) {
      await row(page, n).click({ button: "right" });
      await page.getByTestId("menu-pin").click();
    }
    await expect(page.getByTestId("pinned-icon")).toHaveCount(3);
    const after = await page.getByTestId("chat-name").allTextContents();
    expect(new Set(after.slice(0, 3))).toEqual(new Set([names[0], a, b]));

    await row(page, c).click({ button: "right" });
    await page.getByTestId("menu-pin").click();
    await expect(page.getByTestId("toast").filter({ hasText: "You can only pin up to 3 chats" })).toBeVisible();
    await expect(page.getByTestId("pinned-icon")).toHaveCount(3);
    expect((await page.getByTestId("chat-name").allTextContents()).slice(0, 3)).not.toContain(c);
  });

  test("mute from the thread menu shows the muted icon, unmute removes it; expired mute shows none", async ({ page }) => {
    await boot(page);
    const muted = await mockNames(page, (c) => c.muted_until === -1);
    const expired = await mockNames(page, (c) => c.muted_until !== null && c.muted_until > 0 && c.muted_until < Date.now() / 1000);
    expect(expired.length).toBeGreaterThan(0);
    await expect(row(page, muted[0]!).getByTestId("muted-icon")).toHaveCount(1);
    await expect(row(page, expired[0]!).getByTestId("muted-icon")).toHaveCount(0);

    const target = (await page.getByTestId("chat-name").allTextContents())[3]!;
    await row(page, target).click();
    await expect(page.getByTestId("thread-title")).toHaveText(target);
    await page.getByTestId("thread-menu").click();
    await page.getByTestId("menu-mute").click();
    await expect(page.getByTestId("menu-mute-8h")).toBeVisible();
    await page.keyboard.press("ArrowLeft");
    await expect(page.getByTestId("menu-mute")).toBeVisible();
    await page.getByTestId("menu-mute").click();
    await page.getByTestId("menu-mute-1w").click();
    await expect(row(page, target).getByTestId("muted-icon")).toHaveCount(1);

    await row(page, target).click({ button: "right" });
    await page.getByTestId("menu-unmute").click();
    await expect(row(page, target).getByTestId("muted-icon")).toHaveCount(0);
  });

  test("menu is keyboard operable", async ({ page }) => {
    await boot(page);
    const target = (await page.getByTestId("chat-name").allTextContents())[4]!;
    await row(page, target).focus();
    await page.keyboard.press("Shift+F10");
    await expect(page.getByTestId("context-menu")).toBeVisible();
    await page.getByTestId("context-menu").press("ArrowDown");
    while (!(await page.getByTestId("menu-pin").evaluate((el) => el.classList.contains("bg-surface-2")))) {
      await page.getByTestId("context-menu").press("ArrowDown");
    }
    await page.keyboard.press("Enter");
    await expect(row(page, target).getByTestId("pinned-icon")).toHaveCount(1);
  });
});
