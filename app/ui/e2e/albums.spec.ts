import { test, expect, type Page } from "@playwright/test";
import type { MockBackend } from "../src/lib/mock/backend";

declare global {
  interface Window {
    __waMock?: MockBackend;
  }
}

async function boot(page: Page) {
  await page.goto("/?quiet&fast");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
}

async function openFamily(page: Page) {
  await page.getByTestId("chat-filter").fill("Family");
  await page.getByTestId("chat-row").filter({ hasText: "Family" }).first().click();
  await expect(page.getByTestId("thread-title")).toContainText("Family");
  // Both seeded albums sit at the end of the thread.
  await expect(page.getByTestId("album")).toHaveCount(2);
}

const albums = (page: Page) => page.getByTestId("album");
const lastExport = (page: Page) => page.evaluate(() => window.__waMock!.exports.at(-1) ?? null);

test.describe("albums", () => {
  test("consecutive photos from one sender render as grouped albums", async ({ page }) => {
    await boot(page);
    await openFamily(page);
    const big = albums(page).first();
    await expect(big).toHaveAttribute("data-count", "5");
    await expect(big).toHaveAttribute("data-layout", "grid");
    await expect(big.getByTestId("album-tile")).toHaveCount(4);
    await expect(big.getByTestId("album-more")).toHaveText("+1");
    const trio = albums(page).nth(1);
    await expect(trio).toHaveAttribute("data-count", "3");
    await expect(trio).toHaveAttribute("data-layout", "trio");
    await expect(trio.getByTestId("album-tile")).toHaveCount(3);
    await expect(trio.getByTestId("album-more")).toHaveCount(0);

    // One bubble per album: the caption, one time stamp, no separate item bubbles.
    const bubble = page.locator('[data-testid="message"][data-id="ALBUM-A5"]');
    await expect(bubble).toHaveAttribute("data-album-ids", "ALBUM-A1 ALBUM-A2 ALBUM-A3 ALBUM-A4 ALBUM-A5");
    await expect(bubble.getByTestId("message-text")).toContainText("Weekend at the lake");
    await expect(bubble.locator("span[title]").filter({ hasText: /\d:\d\d/ })).toHaveCount(1);
    await expect(page.locator('[data-testid="message"][data-id="ALBUM-A1"]')).toHaveCount(0);
    // The thread is pinned to the end: the last album is fully in view.
    const lastBox = await page.locator('[data-testid="message"][data-id="ALBUM-B3"]').boundingBox();
    const logBox = await page.getByRole("log").boundingBox();
    expect(lastBox!.y + lastBox!.height).toBeLessThanOrEqual(logBox!.y + logBox!.height + 1);
    // My own album shows delivery ticks once.
    await expect(page.locator('[data-testid="message"][data-id="ALBUM-B3"]').getByTestId("tick")).toHaveCount(1);
  });

  test("a tile not yet downloaded shows a download placeholder", async ({ page }) => {
    await boot(page);
    await openFamily(page);
    const tile = albums(page).first().locator('[data-id="ALBUM-A2"]');
    await expect(tile).toHaveAttribute("data-missing", "true");
    await tile.click();
    await expect(tile).toHaveAttribute("data-missing", "false");
  });

  test("lightbox opens at the tile and steps through the album", async ({ page }) => {
    await boot(page);
    await openFamily(page);
    const lb = page.getByTestId("lightbox");
    await albums(page).first().locator('[data-id="ALBUM-A1"]').click();
    await expect(lb).toBeVisible();
    // Four of five are on disk (A2 is not): stepping covers only the album.
    await expect(lb).toContainText("1 / 4");
    await page.keyboard.press("ArrowRight");
    await expect(lb).toContainText("2 / 4");
    await page.keyboard.press("Escape");
    await expect(lb).toBeHidden();

    // The "+1" tile is the fourth item; the hidden fifth is one step on, with its caption.
    await albums(page).first().locator('[data-id="ALBUM-A4"]').click();
    await expect(lb).toContainText("3 / 4");
    await page.keyboard.press("ArrowRight");
    await expect(lb).toContainText("4 / 4");
    await expect(lb).toContainText("Weekend at the lake");
    await page.keyboard.press("Escape");
    await expect(lb).toBeHidden();
  });

  test("per-item and album actions stay reachable", async ({ page }) => {
    await boot(page);
    await openFamily(page);
    // Right-click one tile: the usual per-message menu.
    await albums(page).nth(1).locator('[data-id="ALBUM-B2"]').click({ button: "right" });
    await expect(page.getByTestId("menu-reply")).toBeVisible();
    await expect(page.getByTestId("menu-star")).toBeVisible();
    await page.keyboard.press("Escape");
    // The bubble's "more" menu offers album-wide actions.
    const bubble = page.locator('[data-testid="message"][data-id="ALBUM-B3"]');
    await bubble.hover();
    await bubble.getByTestId("msg-more").click();
    await expect(page.getByTestId("menu-download-all")).toContainText("Download all (3)");
    await page.getByTestId("menu-star").click();
    await expect(bubble.getByTestId("starred-icon")).toBeVisible();
    // Reply and react work on the album (via its last message).
    await bubble.hover();
    await bubble.getByTestId("reply-btn").click();
    await expect(page.getByTestId("reply-chip")).toBeVisible();
    await bubble.hover();
    await bubble.getByTestId("react-btn").click();
    await page.getByRole("menuitem", { name: "React 👍" }).click();
    await expect(bubble.getByTestId("reactions")).toContainText("👍");
  });

  test("selection mode picks a whole album and the combined download saves as ZIP", async ({ page }) => {
    await boot(page);
    await openFamily(page);
    await page.getByTestId("thread-menu").click();
    await page.getByTestId("menu-select").click();
    await page.locator('[data-testid="message"][data-id="ALBUM-A5"]').click({ position: { x: 4, y: 4 } });
    await expect(page.getByTestId("sel-count")).toHaveText("5 selected");
    await page.locator('[data-testid="message"][data-id="ALBUM-B3"]').click({ position: { x: 4, y: 4 } });
    await expect(page.getByTestId("sel-count")).toHaveText("8 selected");
    await page.getByTestId("sel-download").click();
    const dlg = page.getByTestId("export-dialog");
    await expect(dlg.getByTestId("export-zip")).toBeVisible();
    await expect(dlg.getByTestId("export-folder")).toBeVisible();
    await dlg.getByTestId("export-zip").click();
    await expect(page.getByTestId("export-result")).toHaveText("Saved 8, failed 0");
    const e = await lastExport(page);
    expect(e?.req.mode).toBe("zip");
    expect(e?.req.dest).toMatch(/\.zip$/);
    expect(e?.req.items).toHaveLength(8);
    await page.getByTestId("export-close").click();
    await expect(dlg).toBeHidden();
  });

  test("Download all on an album saves the files to a folder", async ({ page }) => {
    await boot(page);
    await openFamily(page);
    const bubble = page.locator('[data-testid="message"][data-id="ALBUM-B3"]');
    await bubble.hover();
    await bubble.getByTestId("album-download-all").click();
    await expect(page.getByRole("dialog", { name: "Download 3 files" })).toBeVisible();
    await page.getByTestId("export-folder").click();
    await expect(page.getByTestId("export-result")).toHaveText("Saved 3, failed 0");
    const e = await lastExport(page);
    expect(e?.req.mode).toBe("folder");
    expect(e?.req.dest).toBe("/home/user/Pictures/Camera Exports");
    expect(e?.files).toHaveLength(3);
  });

  test("a missing item is fetched first, or reported as failed in read-only mode", async ({ page }) => {
    await boot(page);
    await openFamily(page);
    const first = page.locator('[data-testid="message"][data-id="ALBUM-A5"]');
    await page.evaluate(() => (window.__waMock!.readOnly = true));
    await first.hover();
    await first.getByTestId("album-download-all").click();
    await page.getByTestId("export-folder").click();
    await expect(page.getByTestId("export-result")).toHaveText("Saved 4, failed 1");
    await expect(page.getByTestId("export-failures")).toContainText("read-only");
    await page.getByTestId("export-close").click();

    await page.evaluate(() => (window.__waMock!.readOnly = false));
    await first.hover();
    await first.getByTestId("album-download-all").click();
    await page.getByTestId("export-zip").click();
    await expect(page.getByTestId("export-result")).toHaveText("Saved 5, failed 0");
  });
});

test.describe("media view multi-select", () => {
  test("downloads the picked files as a ZIP or into a folder", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-media").click();
    await expect(page.getByTestId("media-tile").first()).toBeVisible();
    await page.getByTestId("media-select-toggle").click();
    await page.getByTestId("media-tile").nth(0).getByTestId("media-pick").click();
    await page.getByTestId("media-tile").nth(1).getByTestId("media-pick").click();
    await expect(page.getByTestId("media-sel-count")).toHaveText("2 selected");
    await page.getByTestId("media-download-selected").click();
    await expect(page.getByTestId("export-folder")).toBeVisible();
    await page.getByTestId("export-folder").click();
    await expect(page.getByTestId("export-result")).toContainText("Saved 2, failed 0");
    const e = await lastExport(page);
    expect(e?.req.mode).toBe("folder");
    expect(e?.req.items).toHaveLength(2);
  });

  test("ZIP mode works from the gallery too", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-media").click();
    await page.getByTestId("media-select-toggle").click();
    await page.getByTestId("media-tile").nth(2).getByTestId("media-pick").click();
    await page.getByTestId("media-download-selected").click();
    await page.getByTestId("export-zip").click();
    await expect(page.getByTestId("export-result")).toContainText("Saved 1, failed 0");
    expect((await lastExport(page))?.req.mode).toBe("zip");
  });
});
