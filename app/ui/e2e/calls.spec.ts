import { test, expect, type Page } from "@playwright/test";
import type { MockBackend } from "../src/lib/mock/backend";

declare global {
  interface Window {
    __waMock?: MockBackend;
  }
}

test.use({ permissions: ["clipboard-read", "clipboard-write"] });

async function boot(page: Page) {
  await page.goto("/?quiet&fast");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
  await expect(page.getByTestId("bridge-pill")).toContainText("Connected");
}

async function openCalls(page: Page) {
  await page.getByTestId("nav-calls").click();
  await expect(page.getByTestId("view-calls")).toBeVisible();
  await expect(page.getByTestId("call-row").first()).toBeVisible();
}

test.describe("calls log", () => {
  test("shows the log with icons, group badges, and filters", async ({ page }) => {
    await boot(page);
    // Missed calls since the last visit show as a badge, which clears on visit.
    await expect(page.getByTestId("nav-badge-calls")).toBeVisible();
    await openCalls(page);
    await expect(page.getByTestId("nav-badge-calls")).toHaveCount(0);
    expect(await page.getByTestId("call-row").count()).toBeGreaterThan(5);
    await expect(page.getByTestId("calls-note")).toContainText("only be placed or answered on your phone");

    await page.getByTestId("calls-filter-missed").click();
    await expect(page.getByTestId("call-row").first()).toBeVisible();
    const dirs = await page.getByTestId("call-row").evaluateAll((els) => els.map((e) => e.getAttribute("data-direction")));
    expect(dirs.length).toBeGreaterThan(0);
    expect(dirs.every((d) => d === "missed" || d === "declined")).toBe(true);
    await expect(page.getByTestId("dir-missed").first()).toBeVisible();

    await page.getByTestId("calls-filter-video").click();
    await expect(page.getByTestId("call-row").first()).toBeVisible();
    const media = await page.getByTestId("call-row").evaluateAll((els) => els.map((e) => e.getAttribute("data-media")));
    expect(media.every((m) => m === "video")).toBe(true);
    await expect(page.getByTestId("media-video").first()).toBeVisible();

    await page.getByTestId("calls-filter-all").click();
    const groupRow = page.getByTestId("call-row").filter({ has: page.getByTestId("call-group-badge") }).first();
    await expect(groupRow).toBeVisible();
    await expect(page.getByTestId("call-duration").first()).toBeVisible();
  });

  test("clicking a call opens that chat; context menu actions work", async ({ page }) => {
    await boot(page);
    await openCalls(page);
    const first = page.getByTestId("call-row").first();
    const name = (await first.getByTestId("call-name").textContent())?.trim() ?? "";
    await first.click();
    await expect(page.getByTestId("view-chats")).toBeVisible();
    await expect(page.getByTestId("thread-title")).toHaveText(name);

    await openCalls(page);
    const row = page.getByTestId("call-row").first();
    await row.click({ button: "right" });
    await expect(page.getByTestId("context-menu")).toBeVisible();
    await page.getByTestId("menu-copy").click();
    await expect(page.getByTestId("toast").first()).toContainText("Number copied");
    await row.click({ button: "right" });
    await page.getByTestId("menu-info").click();
    await expect(page.getByTestId("profile-drawer")).toBeVisible();
    await expect(page.getByTestId("view-chats")).toBeVisible();
  });

  test("thread shows call pills; header call icons only explain; drawer has a Calls tab", async ({ page }) => {
    await boot(page);
    await page.getByTestId("chat-filter").fill("Ananya Rao");
    await page.getByTestId("chat-row").filter({ hasText: "Ananya Rao" }).first().click();
    await expect(page.getByTestId("thread-title")).toHaveText("Ananya Rao");
    await expect(page.getByTestId("message").first()).toBeVisible();
    const pills = page.getByTestId("call-pill");
    await expect(pills.filter({ hasText: /Missed voice call · \d{2}:\d{2}/ })).toHaveCount(1);
    await expect(pills.filter({ hasText: "📹 Video call · 3 min" })).toHaveCount(1);

    const hint = "Calls can only be placed from your phone";
    await expect(page.getByTestId("header-voice-call")).toHaveAttribute("title", hint);
    await expect(page.getByTestId("header-video-call")).toHaveAttribute("title", hint);
    await expect(page.getByTestId("header-voice-call")).toHaveAttribute("aria-disabled", "true");
    expect(await page.getByRole("button", { name: /Start (a )?call/i }).count()).toBe(0);

    await page.getByTestId("thread-header").click();
    await page.getByTestId("ptab-calls").click();
    await expect(page.getByTestId("pcall").first()).toBeVisible();
    expect(await page.getByTestId("pcall").count()).toBeGreaterThanOrEqual(2);
  });
});

test.describe("incoming call banner", () => {
  test("rings, declines through reject_call and disappears", async ({ page }) => {
    await boot(page);
    const call = await page.evaluate(() => {
      const m = window.__waMock!;
      const c = m.ring({ chatJid: m.chats.find((x) => x.name === "Ananya Rao")!.jid, media: "video" });
      return { id: c.call_id, caller: c.caller_jid };
    });
    const banner = page.getByTestId("call-banner");
    await expect(banner).toBeVisible();
    await expect(banner.getByTestId("call-banner-name")).toHaveText("Ananya Rao");
    await expect(banner).toContainText("Video call");
    await expect(banner.getByTestId("call-banner-hint")).toHaveText("Answer on your phone");
    await banner.getByTestId("decline-call").click();
    await expect(banner).toBeHidden();
    const rejected = await page.evaluate(() => window.__waMock!.rejected);
    expect(rejected).toEqual([{ callId: call.id, callerJid: call.caller }]);
    // The rejected call is now in the log and in the missed filter.
    await page.getByTestId("nav-calls").click();
    await page.getByTestId("calls-filter-missed").click();
    await expect(page.locator(`[data-testid="call-row"][data-call-id="${call.id}"]`)).toHaveAttribute("data-result", "rejected");
  });

  test("disappears when the call is answered elsewhere, and can be dismissed", async ({ page }) => {
    await boot(page);
    const id = await page.evaluate(() => window.__waMock!.ring({ media: "voice" }).call_id);
    await expect(page.getByTestId("call-banner")).toBeVisible();
    await page.evaluate((x) => window.__waMock!.finishCall(x, "answered", 5), id);
    await expect(page.getByTestId("call-banner")).toBeHidden();

    await page.evaluate(() => window.__waMock!.ring({ media: "voice" }));
    await expect(page.getByTestId("call-banner")).toBeVisible();
    await page.getByTestId("dismiss-call").click();
    await expect(page.getByTestId("call-banner")).toBeHidden();
  });

  test("group calls say so; decline is disabled with a reason in read-only mode", async ({ page }) => {
    await boot(page);
    await page.evaluate(() => {
      const m = window.__waMock!;
      m.readOnly = true;
      m.ring({ chatJid: m.chats.find((x) => x.chat_type === "group")!.jid, media: "voice" });
    });
    const banner = page.getByTestId("call-banner");
    await expect(banner).toBeVisible();
    await expect(banner.getByTestId("call-banner-label")).toContainText("Group voice call");
    await banner.getByTestId("decline-call").click();
    await expect(banner.getByTestId("call-banner-error")).toContainText("read-only");
    await expect(banner.getByTestId("decline-call")).toBeDisabled();
    await expect(banner.getByTestId("decline-call")).toHaveAttribute("title", /read-only/);
  });

  test("a missed call raises the badge", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-calls").click();
    await expect(page.getByTestId("view-calls")).toBeVisible();
    await page.getByTestId("nav-chats").click();
    await expect(page.getByTestId("nav-badge-calls")).toHaveCount(0);
    await page.waitForTimeout(2200); // the badge counts calls newer than the visit, in whole seconds (the mock clock may lag by up to 1 s)
    await page.evaluate(() => {
      const c = window.__waMock!.ring();
      window.__waMock!.finishCall(c.call_id, "missed");
    });
    await expect(page.getByTestId("nav-badge-calls")).toHaveText("1");
  });
});
