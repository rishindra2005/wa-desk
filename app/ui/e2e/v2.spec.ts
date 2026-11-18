import { test, expect, type Page } from "@playwright/test";
import type { MockBackend } from "../src/lib/mock/backend";
import type { app as AppState } from "../src/lib/state/app.svelte";

declare global {
  interface Window {
    __waMock?: MockBackend;
    __waApp?: typeof AppState;
  }
}

// The system clipboard is shared between workers, so this file runs serially.
test.describe.configure({ mode: "serial" });
test.use({ permissions: ["clipboard-read", "clipboard-write"] });

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

/** Scroll the open chat to a message that has a local photo. */
async function jumpToLocalImage(page: Page, name: string) {
  const t = await page.evaluate((n) => {
    const c = window.__waMock!.chats.find((x) => x.name === n)!;
    const m = [...c.messages].reverse().find((x) => x.media?.kind === "image" && x.media.url);
    return m ? { jid: c.jid, id: m.id } : null;
  }, name);
  expect(t).not.toBeNull();
  await page.evaluate((x) => window.__waApp!.jumpTo(x.jid, x.id), t!);
  await expect(page.locator(`[data-testid="message"][data-id="${t!.id}"]`)).toBeVisible();
}

const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText());

test.describe("profile drawer", () => {
  test("contact: avatar, phone copy, about, tabs, common groups", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.getByTestId("thread-header").click();
    await expect(page.getByTestId("profile-heading")).toHaveText("Contact info");
    await expect(page.getByTestId("profile-name")).toHaveText("Karthik Iyer");
    await expect(page.getByTestId("profile-phone")).toContainText("+91 99001 55667");
    await expect(page.getByTestId("profile-about")).not.toBeEmpty();
    await page.getByTestId("copy-phone").click();
    expect(await clipboard(page)).toBe("+91 99001 55667");
    await expect(page.getByTestId("count-media")).toBeVisible();
    await page.getByTestId("ptab-docs").click();
    await page.getByTestId("ptab-links").click();
    await page.getByTestId("ptab-starred").click();
    // Full-size picture lightbox (only when the contact has a picture).
    const avatar = page.getByTestId("profile-drawer").getByTestId("avatar-btn");
    if (await avatar.count()) {
      await avatar.click();
      await expect(page.getByTestId("lightbox")).toBeVisible();
      await page.keyboard.press("Escape");
    }
    await page.getByTestId("profile-close").click();
    await expect(page.getByTestId("profile-drawer")).toHaveCount(0);
  });

  test("business contact shows business info", async ({ page }) => {
    await boot(page);
    await openChat(page, "Dr. Iyengar");
    await page.getByTestId("thread-header").click();
    await expect(page.getByTestId("profile-business")).toContainText("Health & Medical");
  });

  test("group: participants search, admin badges, participant profile and back", async ({ page }) => {
    await boot(page);
    await openChat(page, "Startup Founders");
    await page.getByTestId("thread-header").click();
    await expect(page.getByTestId("profile-heading")).toHaveText("Group info");
    await expect(page.getByTestId("profile-subtitle")).toContainText("250 participants");
    await expect(page.getByTestId("profile-description")).toContainText("Be kind");
    await expect(page.getByTestId("admin-badge").first()).toBeVisible();
    await expect(page.getByTestId("profile-created")).toContainText("Created");
    await page.getByTestId("participant-search").fill("Kavya");
    const first = page.getByTestId("participant").first();
    await expect(first).toContainText("Kavya");
    await first.click();
    await expect(page.getByTestId("profile-heading")).toHaveText("Contact info");
    await expect(page.getByTestId("profile-name")).toContainText("Kavya");
    await expect(page.getByTestId("common-group").first()).toBeVisible();
    await page.getByTestId("profile-back").click();
    await expect(page.getByTestId("profile-heading")).toHaveText("Group info");
  });

  test("community: sub-groups, parent link and Community › Group subtitle", async ({ page }) => {
    await boot(page);
    await page.getByTestId("tab-community").click();
    const row = page.getByTestId("chat-row").filter({ hasText: "Bengaluru Neighbours" });
    await expect(row).toHaveCount(1);
    await row.click();
    // A community has no messages of its own: opening it shows its groups.
    await expect(page.getByTestId("profile-heading")).toHaveText("Community info");
    await page.getByTestId("thread-header").click();
    await expect(page.getByTestId("profile-heading")).toHaveText("Community info");
    await expect(page.getByTestId("badge-announce")).toBeVisible();
    await expect(page.getByTestId("subgroup")).toHaveCount(3);
    await page.getByTestId("subgroup").filter({ hasText: "Parking" }).click();
    await expect(page.getByTestId("thread-title")).toHaveText("Parking & Gate");
    await expect(page.getByTestId("thread-subtitle")).toContainText("Community › Group");
    await expect(page.getByTestId("profile-parent")).toContainText("Bengaluru Neighbours");
    await page.getByTestId("profile-parent").click();
    await expect(page.getByTestId("profile-heading")).toHaveText("Community info");
  });

  test("channel: subscribers and verified badge", async ({ page }) => {
    await boot(page);
    await page.getByTestId("tab-channel").click();
    await page.getByTestId("chat-row").filter({ hasText: "WhatsApp Tech News" }).click();
    await page.getByTestId("thread-header").click();
    await expect(page.getByTestId("profile-heading")).toHaveText("Channel info");
    await expect(page.getByTestId("profile-subtitle")).toContainText("1,240,000 subscribers");
    await expect(page.getByTestId("verified-badge")).toBeVisible();
    await expect(page.getByTestId("type-channel").first()).toBeVisible();
  });
});

test.describe("context menus", () => {
  test("chat list menu: right-click, hover button, keyboard, Esc, click-outside", async ({ page }) => {
    await boot(page);
    const row = page.getByTestId("chat-row").first();
    await row.click({ button: "right" });
    const menu = page.getByTestId("context-menu");
    await expect(menu).toBeVisible();
    for (const id of ["open", "read", "info", "search", "export", "jid"]) await expect(page.getByTestId(`menu-${id}`)).toBeVisible();
    await page.getByTestId("menu-jid").click();
    expect(await clipboard(page)).toContain("@");
    await expect(menu).toHaveCount(0);

    await row.hover();
    await row.locator("xpath=..").getByTestId("row-menu").click();
    await expect(menu).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);

    // Keyboard: Shift+F10 opens, arrows move, Enter activates "Contact info".
    await row.focus();
    await page.keyboard.press("Shift+F10");
    await expect(menu).toBeVisible();
    await page.keyboard.press("End"); // last item: Copy JID
    await page.keyboard.press("Enter");
    await expect(menu).toHaveCount(0);
    await expect(page.getByText("JID copied").first()).toBeVisible();

    await row.click({ button: "right" });
    await expect(menu).toBeVisible();
    await page.mouse.click(700, 400);
    await expect(menu).toHaveCount(0);
  });

  test("menu stays inside the viewport near the bottom-right corner", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    await page.getByTestId("message").last().click({ button: "right", position: { x: 20, y: 10 } });
    await page.getByTestId("context-menu").waitFor();
    await page.keyboard.press("Escape");
    await page.mouse.click(1300, 840, { button: "right" });
    await page.evaluate(() => window.__waApp && void 0);
    const msg = page.getByTestId("message").last();
    const box = (await msg.boundingBox())!;
    await msg.click({ button: "right", position: { x: box.width - 6, y: box.height - 4 } });
    const m = page.getByTestId("context-menu");
    await expect(m).toBeVisible();
    const b = (await m.boundingBox())!;
    const vp = page.viewportSize()!;
    expect(b.x).toBeGreaterThanOrEqual(0);
    expect(b.y).toBeGreaterThanOrEqual(0);
    expect(b.x + b.width).toBeLessThanOrEqual(vp.width);
    expect(b.y + b.height).toBeLessThanOrEqual(vp.height);
  });

  test("message menu has the expected actions; thread header menu works", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.getByTestId("message").last().click({ button: "right", position: { x: 20, y: 8 } });
    for (const id of ["reply", "select", "forward", "star", "info"]) await expect(page.getByTestId(`menu-${id}`)).toBeVisible();
    await page.getByTestId("menu-info").click();
    await expect(page.getByTestId("message-info")).toBeVisible();
    await page.keyboard.press("Escape");
    await page.getByTestId("thread-menu").click();
    for (const id of ["info", "search", "select", "export", "refresh"]) await expect(page.getByTestId(`menu-${id}`)).toBeVisible();
    await page.getByTestId("menu-search").click();
    await expect(page.getByTestId("view-search")).toBeVisible();
  });
});

test.describe("delivery ticks and sending", () => {
  test("own messages show ticks; chat list preview shows ticks", async ({ page }) => {
    await boot(page);
    await expect(page.getByTestId("chat-row").getByTestId("tick").first()).toBeVisible();
    await openChat(page, "Karthik Iyer");
    const ticks = page.getByTestId("message").getByTestId("tick");
    await expect(ticks.first()).toBeVisible();
    const statuses = await ticks.evaluateAll((els) => [...new Set(els.map((e) => e.getAttribute("data-status")))]);
    expect(statuses.some((s) => s === "read" || s === "delivered" || s === "played" || s === "sent")).toBe(true);
    const read = page.locator('[data-testid="tick"][data-status="read"]').first();
    if (await read.count()) await expect(read).toHaveAttribute("data-tone", "blue");
  });

  test("clock while sending, then ticks progress live via db:changed", async ({ page }) => {
    await boot(page, "quiet");
    await openChat(page, "Karthik Iyer");
    await page.getByTestId("composer-input").fill("tick test ✓✓");
    await page.getByTestId("composer-input").press("Enter");
    const msg = page.getByTestId("message").filter({ hasText: "tick test ✓✓" });
    await expect(msg.locator('[data-status="pending"]')).toBeVisible();
    await expect(msg.locator('[data-status="sent"]')).toBeVisible();
    await expect(msg.locator('[data-status="delivered"]')).toBeVisible();
    await expect(msg.locator('[data-status="read"]')).toBeVisible({ timeout: 8000 });
    await expect(msg.getByTestId("tick")).toHaveAttribute("title", "Read");
  });

  test("failed send shows a red ! with retry", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.getByTestId("composer-input").fill("will fail [fail]");
    await page.getByTestId("composer-input").press("Enter");
    const msg = page.getByTestId("message").filter({ hasText: "will fail" });
    await expect(msg.locator('[data-status="failed"]')).toBeVisible();
    await expect(msg.getByTestId("retry-send")).toBeVisible();
    await msg.getByTestId("retry-send").click();
    await expect(msg.locator('[data-status="failed"]')).toBeVisible();
  });

  test("drafts persist per chat", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.getByTestId("composer-input").fill("half-written…");
    await openChat(page, "Ananya Rao");
    await expect(page.getByTestId("composer-input")).toHaveValue("");
    await openChat(page, "Karthik Iyer");
    await expect(page.getByTestId("composer-input")).toHaveValue("half-written…");
    await page.reload();
    await expect(page.getByTestId("chat-row").first()).toBeVisible();
    await openChat(page, "Karthik Iyer");
    await expect(page.getByTestId("composer-input")).toHaveValue("half-written…");
  });
});

test.describe("unread divider", () => {
  test("shows N unread messages when opening an unread chat, and Jump to latest after scrolling up", async ({ page }) => {
    await boot(page);
    const unread = await page.evaluate(() => window.__waMock!.chats.find((c) => c.unread && c.chat_type === "direct")!.name);
    await openChat(page, unread);
    await expect(page.getByTestId("unread-divider")).toContainText(/\d+ unread messages?/);
    await page.getByRole("log", { name: "Messages" }).evaluate((el) => (el.scrollTop = 0));
    await expect(page.getByTestId("jump-latest")).toBeVisible();
    await page.getByTestId("jump-latest").click();
    await expect(page.getByTestId("jump-latest")).toHaveCount(0);
  });
});

test.describe("avatars", () => {
  test("pictures fade in; chats without one fall back to initials / type icons", async ({ page }) => {
    await boot(page);
    await openChat(page, "Dad");
    const header = page.getByTestId("thread-header").getByTestId("avatar");
    await expect(header).toHaveAttribute("data-state", "fallback");
    await expect(header.getByTestId("avatar-initials")).toHaveText("D");
    await openChat(page, "Ananya Rao");
    await expect(page.getByTestId("thread-header").getByTestId("avatar")).toHaveAttribute("data-state", "image");
    await openChat(page, "Startup Founders");
    await expect(page.getByTestId("thread-header").getByTestId("avatar")).toHaveAttribute("data-state", /image|fallback/);
  });
});

test.describe("selection", () => {
  test("range select, copy, forward limit and per-target results", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    const msgs = page.getByTestId("message");
    await msgs.nth(-6).click({ button: "right", position: { x: 20, y: 8 } });
    await page.getByTestId("menu-select").click();
    await expect(page.getByTestId("selection-bar")).toBeVisible();
    await expect(page.getByTestId("sel-count")).toHaveText("1 selected");
    await msgs.nth(-2).click({ modifiers: ["Shift"], position: { x: 20, y: 8 } });
    await expect(page.getByTestId("sel-count")).toHaveText("5 selected");
    await msgs.nth(-3).click(); // toggle one off
    await expect(page.getByTestId("sel-count")).toHaveText("4 selected");

    await page.getByTestId("sel-copy").click();
    await expect.poll(() => clipboard(page)).toMatch(/^\[\d{2}\//);
    const text = await clipboard(page);
    expect(text.split("\n")).toHaveLength(4);
    expect(text).toMatch(/^\[\d{2}\/\d{2}\/\d{4}, \d{2}:\d{2}\] /);

    await page.getByTestId("sel-forward").click();
    const dlg = page.getByTestId("forward-dialog");
    await expect(dlg).toBeVisible();
    const boxes = dlg.getByTestId("forward-target").locator("input[type=checkbox]");
    for (let i = 0; i < 6; i++) await boxes.nth(i).click({ force: true });
    await expect(page.getByTestId("forward-count")).toContainText("5 of 5");
    await expect(boxes.nth(5)).not.toBeChecked();
    // Uncheck all, then pick one normal chat and the admin-only community.
    for (let i = 0; i < 5; i++) await boxes.nth(i).click({ force: true });
    await page.getByTestId("forward-search").fill("Karthik");
    await expect(dlg.getByTestId("forward-target").first()).toContainText("Karthik");
    await dlg.getByTestId("forward-target").first().locator("input").check({ force: true });
    await page.getByTestId("forward-search").fill("Bengaluru Neighbours");
    await expect(dlg.getByTestId("forward-target").first()).toContainText("Bengaluru Neighbours");
    await dlg.getByTestId("forward-target").first().locator("input").check({ force: true });
    await page.getByTestId("forward-send").click();
    const results = page.getByTestId("forward-result");
    await expect(results).toHaveCount(2);
    await expect(page.locator('[data-testid="forward-result"][data-ok="true"]')).toHaveCount(1);
    await expect(page.locator('[data-testid="forward-result"][data-ok="false"]')).toHaveCount(1);
    await page.getByRole("button", { name: "Done" }).click();
  });

  test("Ctrl+A selects all loaded messages and Esc cancels", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    await page.getByTestId("message").last().click({ position: { x: 20, y: 8 } });
    await page.keyboard.press("Control+a");
    await expect(page.getByTestId("selection-bar")).toBeVisible();
    const n = Number((await page.getByTestId("sel-count").innerText()).split(" ")[0]);
    expect(n).toBeGreaterThanOrEqual(80);
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("selection-bar")).toHaveCount(0);
  });

  test("message text is selectable (no user-select: none)", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    const p = page.getByTestId("message-text").last();
    expect(await p.evaluate((el) => getComputedStyle(el).userSelect)).not.toBe("none");
    await p.evaluate((el) => {
      const r = document.createRange();
      r.selectNodeContents(el);
      const s = getSelection()!;
      s.removeAllRanges();
      s.addRange(r);
    });
    expect((await page.evaluate(() => getSelection()!.toString())).length).toBeGreaterThan(2);
  });
});

test.describe("stars", () => {
  test("star and unstar, starred view and drawer tab", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    const msg = page.getByTestId("message").filter({ hasNot: page.getByTestId("deleted"), hasNotText: "★" }).last();
    const before = await page.evaluate(() => window.__waMock!.listStarred(null, null, 500).items.length);
    await msg.click({ button: "right", position: { x: 20, y: 8 } });
    const star = page.getByTestId("menu-star");
    const label = (await star.innerText()).trim();
    await star.click();
    const count = () => page.evaluate(() => window.__waMock!.listStarred(null, null, 500).items.length);
    const want = label === "Star" ? before + 1 : before - 1;
    await expect.poll(count).toBe(want);
    const after = want;
    if (label === "Star") await expect(msg.getByTestId("starred-icon")).toBeVisible();

    await page.getByTestId("nav-search").click();
    await page.getByTestId("mode-starred").click();
    await expect(page.getByTestId("starred-hit").first()).toBeVisible();
    await expect(page.getByTestId("starred-count")).toContainText(`${after} starred`);
    await page.getByTestId("starred-hit").first().click();
    await expect(page.getByTestId("view-chats")).toBeVisible();

    await page.getByTestId("message").last().click({ button: "right", position: { x: 20, y: 8 } });
    await page.getByTestId("menu-star").click();
  });
});

test.describe("export", () => {
  test("export dialog: format, date range, include media, result with open/reveal", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    await page.getByTestId("thread-menu").click();
    await page.getByTestId("menu-export").click();
    const dlg = page.getByTestId("export-dialog");
    await expect(dlg).toBeVisible();
    await page.getByTestId("export-format-txt").click();
    await expect(page.getByTestId("export-media")).toBeDisabled();
    await page.getByTestId("export-format-html").click();
    await page.getByTestId("export-media").check();
    await page.getByTestId("export-from").fill("2026-01-01");
    await page.getByTestId("export-run").click();
    await expect(page.getByTestId("export-result")).toContainText("/exports/");
    await page.getByTestId("export-open").click();
    await page.getByTestId("export-reveal").click();
    await page.getByRole("button", { name: "Done" }).click();
    await expect(dlg).toHaveCount(0);
  });
});

test.describe("convert", () => {
  test("lightbox Convert shows progress and the output with Open/Reveal", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    await jumpToLocalImage(page, "Ananya Rao");
    await page.getByTestId("media-image").last().click();
    await expect(page.getByTestId("lightbox")).toBeVisible();
    await page.getByTestId("convert-btn").click();
    await page.getByTestId("convert-more").click();
    await page.getByTestId("convert-start").click();
    const card = page.getByTestId("convert-panel").getByTestId("job-card");
    await expect(card).toBeVisible();
    await expect(card).toHaveAttribute("data-status", /queued|running|succeeded/);
    await expect(card).toHaveAttribute("data-status", "succeeded", { timeout: 15_000 });
    await expect(card.getByTestId("job-output")).toBeVisible();
    await card.getByRole("button", { name: "Open" }).click();
    await card.getByRole("button", { name: "Reveal" }).click();
  });

  test("document preview offers kind-specific conversions", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-docs").click();
    await page.getByRole("tab", { name: "PDF" }).click();
    await page.getByTestId("doc-row").filter({ hasText: "Local" }).first().click();
    await page.getByTestId("convert-btn").click();
    await expect(page.getByTestId("convert-opt-pdf_compress")).toBeVisible();
    await expect(page.getByTestId("convert-opt-pdf_to_images")).toBeVisible();
    await expect(page.getByTestId("convert-opt-office_to_pdf")).toHaveCount(0);
  });
});

test.describe("names and chat types", () => {
  test("community tab, placeholder names, refresh names from the list menu", async ({ page }) => {
    await boot(page);
    await page.getByTestId("tab-group").click();
    await expect(page.getByTestId("chat-name").filter({ hasText: "Unnamed group" })).toHaveCount(2);
    await expect(page.getByTestId("chat-parent").first()).toContainText("Bengaluru Neighbours");
    await page.getByTestId("list-menu").click();
    await page.getByTestId("menu-refresh-names").click();
    await expect(page.getByText("Fixed 2 chat names")).toBeVisible();
    await expect(page.getByTestId("chat-name").filter({ hasText: "Neighbourhood Watch" })).toHaveCount(1);
    await expect(page.getByTestId("chat-name").filter({ hasText: "Unnamed group" })).toHaveCount(0);
    await page.getByTestId("tab-broadcast").click();
    await expect(page.getByTestId("chat-row")).toHaveCount(1);
    await expect(page.getByTestId("type-broadcast")).toBeVisible();
  });

  test("System view refresh names reports the count", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-system").click();
    await page.getByTestId("system-refresh-names").click();
    await expect(page.getByTestId("refresh-result")).toContainText("Fixed 2 names");
    await page.getByTestId("system-refresh-names").click();
    await expect(page.getByTestId("refresh-result")).toContainText("already up to date");
  });

  test("unknown LID senders never show raw digits", async ({ page }) => {
    await boot(page);
    const text = await page.evaluate(() => {
      const c = window.__waMock!.chats.find((x) => x.messages.some((m) => m.sender_jid?.endsWith("@lid")))!;
      return c.name;
    });
    await openChat(page, text);
    await expect(page.getByTestId("thread")).not.toContainText(/\b\d{12,}\b/);
  });
});

test.describe("pdf and previews", () => {
  test("PDF text layer is selectable and copyable", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-docs").click();
    await page.getByRole("tab", { name: "PDF" }).click();
    await page.getByTestId("doc-row").filter({ hasText: "Local" }).first().click();
    const panel = page.getByTestId("doc-panel");
    await expect(panel.locator("canvas[data-rendered]").first()).toBeVisible();
    const layer = panel.locator('[data-testid="pdf-text-layer"][data-ready="true"]').first();
    await expect(layer).toBeVisible();
    await expect(layer).toContainText("wa-desk sample document");
    expect(await layer.locator("span").first().evaluate((el) => getComputedStyle(el).userSelect)).toBe("text");
    const selected = await layer.evaluate((el) => {
      const r = document.createRange();
      r.selectNodeContents(el);
      const s = getSelection()!;
      s.removeAllRanges();
      s.addRange(r);
      return s.toString();
    });
    expect(selected).toContain("wa-desk sample document");
  });

  test("forward paging after jumping to an old message", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    const target = await page.evaluate(() => {
      const c = window.__waMock!.chats.find((x) => x.name === "Ananya Rao")!;
      return { jid: c.jid, id: c.messages[200]!.id, total: c.messages.length };
    });
    await page.evaluate((t) => window.__waApp!.jumpTo(t.jid, t.id), target);
    const thread = page.getByTestId("thread");
    await expect(thread).toHaveAttribute("data-detached", "true");
    await expect(page.locator(`[data-testid="message"][data-id="${target.id}"]`)).toBeVisible();
    const log = page.getByRole("log", { name: "Messages" });
    await expect
      .poll(
        async () => {
          await log.evaluate((el) => (el.scrollTop = el.scrollHeight));
          return thread.getAttribute("data-detached");
        },
        { timeout: 20_000, intervals: [300] },
      )
      .toBe("false");
    expect(Number(await thread.getAttribute("data-loaded-count"))).toBeGreaterThan(200);
    await log.evaluate((el) => (el.scrollTop = el.scrollHeight));
    const lastId = await page.evaluate(() => {
      const c = window.__waMock!.chats.find((x) => x.name === "Ananya Rao")!;
      return c.messages[c.messages.length - 1]!.id;
    });
    await expect(page.locator(`[data-testid="message"][data-id="${lastId}"]`)).toBeVisible();
  });

  test("document and image previews open in under a second", async ({ page }) => {
    await boot(page);
    const found = await page.evaluate(() => {
      for (const c of window.__waMock!.chats) {
        const m = c.messages.find((x) => x.media?.kind === "document" && x.media.filename?.endsWith(".pdf") && x.media.url);
        if (m) return { jid: c.jid, id: m.id, name: c.name };
      }
      return null;
    });
    expect(found).not.toBeNull();
    await openChat(page, found!.name);
    await page.evaluate((t) => window.__waApp!.jumpTo(t.jid, t.id), found!);
    const card = page.locator(`[data-testid="message"][data-id="${found!.id}"]`);
    await expect(card).toBeVisible();
    // Spinner/thumbnail must show within 150 ms of the click, the first page within 1 s.
    const t0 = Date.now();
    await card.getByTestId("doc-preview").click();
    await expect(page.getByTestId("doc-preview-body")).toBeVisible();
    await expect(page.locator("canvas[data-rendered]").first()).toBeVisible({ timeout: 1000 });
    const elapsed = Date.now() - t0;
    console.log(`pdf preview opened in ${elapsed} ms`);
    expect(elapsed).toBeLessThan(1000);
    await page.keyboard.press("Escape");

    await openChat(page, "Ananya Rao");
    await jumpToLocalImage(page, "Ananya Rao");
    const img = page.getByTestId("media-image").last();
    const t1 = Date.now();
    await img.click();
    await expect(page.getByTestId("lightbox-image")).toBeVisible();
    await page.waitForFunction(() => {
      const i = document.querySelector<HTMLImageElement>('[data-testid="lightbox-image"]');
      return !!i && i.complete && i.naturalWidth > 0;
    });
    const imgElapsed = Date.now() - t1;
    console.log(`image preview opened in ${imgElapsed} ms`);
    expect(imgElapsed).toBeLessThan(1000);
  });
});
