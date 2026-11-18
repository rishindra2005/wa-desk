import { test, expect, type Page } from "@playwright/test";
import type { MockBackend } from "../src/lib/mock/backend";
import type { app as AppState } from "../src/lib/state/app.svelte";

declare global {
  interface Window {
    __waMock?: MockBackend;
    __waApp?: typeof AppState;
  }
}

async function boot(page: Page, query = "quiet&fast") {
  await page.goto(`/?${query}`);
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
  await expect(page.getByTestId("bridge-pill")).toContainText("Connected");
}

async function openChat(page: Page, name: string) {
  await page.getByTestId("chat-filter").fill(name);
  await page.getByTestId("chat-row").filter({ hasText: name }).first().click();
  await expect(page.getByTestId("thread-title")).toContainText(name);
  await expect(page.getByTestId("message").first()).toBeVisible();
}

/** Dispatch a synthetic paste into the composer. */
async function paste(page: Page, spec: { files?: { name: string; type: string }[]; data?: Record<string, string> }) {
  return page.evaluate((s) => {
    const dt = new DataTransfer();
    for (const f of s.files ?? []) dt.items.add(new File([new Uint8Array([137, 80, 78, 71, 1, 2, 3])], f.name, { type: f.type }));
    for (const [k, v] of Object.entries(s.data ?? {})) dt.setData(k, v);
    const ev = new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true });
    document.querySelector<HTMLTextAreaElement>("[data-testid=composer-input]")!.dispatchEvent(ev);
    return ev.defaultPrevented;
  }, spec);
}

async function lastMessages(page: Page, chat: string, n: number) {
  return page.evaluate(
    ([name, count]) => {
      const c = window.__waMock!.chats.find((x) => x.name === name)!;
      return c.messages.slice(-(count as number)).map((m) => ({ content: m.content, file: m.media?.filename ?? null, fromMe: m.is_from_me }));
    },
    [chat, n] as const,
  );
}

test.describe("multi-file attach", () => {
  test("picker adds several files, they can be removed, and all are sent in order with a caption on the first", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.evaluate(() => (window.__waMock!.nextPick = ["/home/u/a.jpg", "/home/u/b.pdf", "/home/u/c.png", "/home/u/d.txt"]));
    await page.getByRole("button", { name: "Attach files" }).click();
    const chips = page.getByTestId("attachment-chip");
    await expect(chips).toHaveCount(4);
    await page.getByRole("button", { name: "Remove d.txt" }).click();
    await expect(chips).toHaveCount(3);
    await page.getByTestId("composer-input").fill("Trip photos");
    await expect(page.getByTestId("attachment-count")).toContainText("caption goes on the first");
    await page.getByTestId("send-btn").click();
    await expect(page.getByTestId("confirm-files")).toContainText("a.jpg");
    await expect(page.getByTestId("confirm-files")).not.toContainText("d.txt");
    await page.getByTestId("confirm-send-btn").click();
    await expect(page.getByTestId("attachment-tray")).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => window.__waMock!.chats.find((c) => c.name === "Karthik Iyer")!.messages.filter((m) => m.is_from_me && /^(a|b|c)\./.test(m.media?.filename ?? "")).length)).toBe(3);
    const sent = await lastMessages(page, "Karthik Iyer", 3);
    expect(sent.map((m) => m.file)).toEqual(["a.jpg", "b.pdf", "c.png"]);
    expect(sent.map((m) => m.content)).toEqual(["Trip photos", "", ""]);
  });

  test("a failing file does not drop the rest and can be retried", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.evaluate(() => (window.__waMock!.nextPick = ["/home/u/one.jpg", "/home/u/two[fail].jpg", "/home/u/three.jpg"]));
    await page.getByRole("button", { name: "Attach files" }).click();
    await expect(page.getByTestId("attachment-chip")).toHaveCount(3);
    await page.getByTestId("send-btn").click();
    await page.getByTestId("confirm-send-btn").click();
    await expect(page.getByTestId("send-progress")).toContainText("Sent 2 of 3");
    await expect(page.getByTestId("send-progress")).toContainText("1 failed");
    await expect(page.getByTestId("retry-send")).toHaveCount(1);
    await expect(page.getByTestId("toast").filter({ hasText: "two[fail].jpg not sent" })).toBeVisible();
    const files = await page.evaluate(() => window.__waMock!.chats.find((c) => c.name === "Karthik Iyer")!.messages.filter((m) => m.is_from_me).slice(-2).map((m) => m.media?.filename));
    expect(files).toEqual(["one.jpg", "three.jpg"]);
  });

  test("dedupes the same path and caps nothing else", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.evaluate(() => (window.__waMock!.nextPick = ["/home/u/a.jpg", "/home/u/a.jpg"]));
    await page.getByRole("button", { name: "Attach files" }).click();
    await expect(page.getByTestId("attachment-chip")).toHaveCount(1);
  });

  test("dropping files attaches them, with a hint while dragging", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.evaluate(() => {
      const dt = new DataTransfer();
      dt.items.add(new File([new Uint8Array([1, 2, 3])], "dropped1.png", { type: "image/png" }));
      dt.items.add(new File([new Uint8Array([4, 5, 6])], "dropped2.pdf", { type: "application/pdf" }));
      (window as unknown as { __dt: DataTransfer }).__dt = dt;
      document.body.dispatchEvent(new DragEvent("dragover", { dataTransfer: dt, bubbles: true, cancelable: true }));
    });
    await expect(page.getByTestId("drop-hint")).toBeVisible();
    await page.evaluate(() => {
      const dt = (window as unknown as { __dt: DataTransfer }).__dt;
      document.body.dispatchEvent(new DragEvent("drop", { dataTransfer: dt, bubbles: true, cancelable: true }));
    });
    await expect(page.getByTestId("drop-hint")).toHaveCount(0);
    await expect(page.getByTestId("attachment-chip")).toHaveCount(2);
    await expect(page.getByTestId("attachment-chip").first()).toContainText("dropped1.png");
  });
});

test.describe("paste into the composer", () => {
  test("an image on the clipboard is written to the outbox and lands in the tray", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    expect(await paste(page, { files: [{ name: "image.png", type: "image/png" }] })).toBe(true);
    await expect(page.getByTestId("attachment-chip")).toHaveCount(1);
    await expect(page.getByTestId("attachment-chip").locator("img")).toBeVisible();
    const staged = await page.evaluate(() => window.__waMock!.staged);
    expect(staged).toHaveLength(1);
    expect(staged[0]!.path).toContain("/outbox/");
    // A second paste adds to the same tray and both are sent.
    await paste(page, { files: [{ name: "shot.png", type: "image/png" }] });
    await expect(page.getByTestId("attachment-chip")).toHaveCount(2);
    await page.getByTestId("send-btn").click();
    await page.getByTestId("confirm-send-btn").click();
    await expect.poll(async () => (await lastMessages(page, "Karthik Iyer", 2)).map((m) => m.file)).toEqual(["image.png", "shot.png"]);
  });

  test("files copied in a file manager (uri-list) are attached by path", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    const prevented = await paste(page, { data: { "text/uri-list": "file:///home/u/My%20Photo.jpg\r\nfile:///home/u/notes.pdf\r\n" } });
    expect(prevented).toBe(true);
    const chips = page.getByTestId("attachment-chip");
    await expect(chips).toHaveCount(2);
    await expect(chips.first()).toContainText("My Photo.jpg");
    expect(await page.evaluate(() => window.__waMock!.staged.length)).toBe(0);
  });

  test("ordinary text paste is left alone", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    expect(await paste(page, { data: { "text/plain": "just some words" } })).toBe(false);
    expect(await paste(page, { data: { "text/plain": "https://example.com/a.png" } })).toBe(false);
    await expect(page.getByTestId("attachment-chip")).toHaveCount(0);
  });

  test("a refused paste shows an error and attaches nothing", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.evaluate(() => (window.__waMock!.readOnly = true));
    await paste(page, { files: [{ name: "image.png", type: "image/png" }] });
    await expect(page.getByTestId("toast").filter({ hasText: "read-only" })).toBeVisible();
    await expect(page.getByTestId("attachment-chip")).toHaveCount(0);
    await expect(page.getByTestId("attachment-busy")).toHaveCount(0);
  });
});

test.describe("copy image", () => {
  async function jumpToLocalImage(page: Page, name: string) {
    const t = await page.evaluate((n) => {
      const c = window.__waMock!.chats.find((x) => x.name === n)!;
      const m = [...c.messages].reverse().find((x) => x.media?.kind === "image" && x.media.url);
      return m ? { jid: c.jid, id: m.id } : null;
    }, name);
    expect(t).not.toBeNull();
    await page.evaluate((x) => window.__waApp!.jumpTo(x.jid, x.id), t!);
    await expect(page.locator(`[data-testid="message"][data-id="${t!.id}"]`)).toBeVisible();
    return t!;
  }

  test("the context menu copies the image itself", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    const t = await jumpToLocalImage(page, "Ananya Rao");
    await page.locator(`[data-testid="message"][data-id="${t.id}"]`).click({ button: "right", position: { x: 20, y: 20 } });
    await page.getByTestId("menu-copy-image").click();
    await expect(page.getByTestId("toast").filter({ hasText: "Image copied" })).toBeVisible();
    expect(await page.evaluate(() => window.__waMock!.copiedImages)).toEqual([{ chatJid: t.jid, messageId: t.id }]);
  });

  test("the lightbox has a Copy image button", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    const t = await jumpToLocalImage(page, "Ananya Rao");
    await page.getByTestId("media-image").last().click();
    await expect(page.getByTestId("lightbox")).toBeVisible();
    await page.getByTestId("lightbox-copy").click();
    await expect(page.getByTestId("toast").filter({ hasText: "Image copied" })).toBeVisible();
    expect((await page.evaluate(() => window.__waMock!.copiedImages)).at(-1)?.chatJid).toBe(t.jid);
  });

  test("non-downloaded and non-image messages have no Copy image entry", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    await page.getByTestId("message").last().click({ button: "right", position: { x: 20, y: 10 } });
    await expect(page.getByTestId("context-menu")).toBeVisible();
    await expect(page.getByTestId("menu-copy-image")).toHaveCount(0);
  });
});

test.describe("app zoom", () => {
  test("Ctrl+= / Ctrl++ / numpad / Ctrl+- / Ctrl+0 step through browser-like levels with an indicator", async ({ page }) => {
    await boot(page);
    const zooms = () => page.evaluate(() => window.__waMock!.zooms);
    const ind = page.getByTestId("zoom-indicator");
    await page.keyboard.press("Control+Equal");
    await expect(ind).toHaveText("110%");
    await page.keyboard.press("Control+Shift+Equal");
    await expect(ind).toHaveText("125%");
    await page.keyboard.press("Control+NumpadAdd");
    await expect(ind).toHaveText("150%");
    await page.keyboard.press("Control+Minus");
    await expect(ind).toHaveText("125%");
    await page.keyboard.press("Control+NumpadSubtract");
    await expect(ind).toHaveText("110%");
    await page.keyboard.press("Control+0");
    await expect(ind).toHaveText("100%");
    await expect.poll(zooms).toEqual([110, 125, 150, 125, 110, 100]);
    // The indicator is transient.
    await expect(ind).toBeHidden({ timeout: 4000 });
  });

  test("clamps at 200% and 50%", async ({ page }) => {
    await boot(page);
    for (let i = 0; i < 12; i++) await page.keyboard.press("Control+Equal");
    await expect.poll(() => page.evaluate(() => window.__waMock!.settings.zoom_percent)).toBe(200);
    for (let i = 0; i < 20; i++) await page.keyboard.press("Control+Minus");
    await expect.poll(() => page.evaluate(() => window.__waMock!.settings.zoom_percent)).toBe(50);
    await expect(page.getByTestId("zoom-indicator")).toHaveText("50%");
  });

  test("works while typing in the composer without editing the text, and Ctrl+wheel zooms", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    const input = page.getByTestId("composer-input");
    await input.fill("hello");
    await input.press("Control+Equal");
    await input.press("Control+Minus");
    await expect(input).toHaveValue("hello");
    await page.keyboard.press("Control+0");
    await page.mouse.move(600, 400);
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, -100);
    await page.keyboard.up("Control");
    await expect(page.getByTestId("zoom-indicator")).toHaveText("110%");
  });

  test("does not clash with the existing shortcuts", async ({ page }) => {
    await boot(page);
    await page.keyboard.press("Control+k");
    await expect(page.getByTestId("palette")).toBeVisible();
    await page.keyboard.press("Escape");
    await page.keyboard.press("Alt+4");
    await expect(page.getByTestId("view-media")).toBeVisible();
    expect(await page.evaluate(() => window.__waMock!.zooms)).toEqual([]);
  });

  test("plain +/-/0 still zoom the picture inside the lightbox, not the app", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-media").click();
    await page.getByRole("tab", { name: "Downloaded" }).click();
    await page.getByTestId("media-tile").first().getByRole("button").first().click();
    const lb = page.getByTestId("lightbox");
    await expect(lb).toBeVisible();
    const zoom = lb.getByTestId("zoom-level");
    if (await zoom.isVisible()) {
      await page.keyboard.press("=");
      await expect(zoom).toHaveText("125%");
      await page.keyboard.press("Control+Equal");
      await expect(page.getByTestId("zoom-indicator")).toHaveText("110%");
      await expect(zoom).toHaveText("125%");
    }
  });
});
