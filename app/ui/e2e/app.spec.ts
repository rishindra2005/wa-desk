import { test, expect, type Page } from "@playwright/test";

async function boot(page: Page) {
  await page.goto("/?quiet&fast");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
  await expect(page.getByTestId("bridge-pill")).toContainText("Connected");
}

async function openChat(page: Page, name: string) {
  await page.getByTestId("chat-filter").fill(name);
  await page.getByTestId("chat-row").filter({ hasText: name }).first().click();
  await expect(page.getByTestId("thread-title")).toHaveText(name);
  await expect(page.getByTestId("message").first()).toBeVisible();
}

test.describe("chats", () => {
  test("switches between chats", async ({ page }) => {
    await boot(page);
    const rows = page.getByTestId("chat-row");
    await rows.nth(0).click();
    await expect(page.getByTestId("thread-title")).toHaveText("Ananya Rao");
    await rows.nth(1).click();
    await expect(page.getByTestId("thread-title")).toHaveText("Mom ❤️");
    await expect(rows.nth(1).getByTestId("unread-dot")).toHaveCount(0);
  });

  test("scrolling up loads older messages", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    const thread = page.getByTestId("thread");
    await expect(thread).toHaveAttribute("data-loaded-count", "80");
    const log = page.getByRole("log", { name: "Messages" });
    // The oldest message of the first page (what was at the top before loading more).
    const firstId = await page.evaluate(() => {
      const c = window.__waMock?.chats.find((x) => x.name === "Ananya Rao");
      return c ? c.messages[c.messages.length - 80]?.id : null;
    });
    await log.evaluate((el) => (el.scrollTop = 0));
    await expect.poll(async () => Number(await thread.getAttribute("data-loaded-count"))).toBeGreaterThan(80);
    // Scroll position is kept: the previous top message is still rendered near the viewport.
    const prev = page.locator(`[data-testid="message"][data-id="${firstId}"]`);
    await expect(prev).toBeVisible();
    await expect(prev).toBeInViewport();
    const top = await log.evaluate((el) => el.scrollTop);
    expect(top).toBeGreaterThan(200);
  });

  test("sends a message that appears in the thread", async ({ page }) => {
    await boot(page);
    await openChat(page, "Karthik Iyer");
    await page.getByTestId("composer-input").fill("Hello from Playwright ✨");
    await page.getByTestId("composer-input").press("Enter");
    await expect(page.getByTestId("message").filter({ hasText: "Hello from Playwright ✨" })).toBeVisible();
    await expect(page.getByTestId("composer-input")).toHaveValue("");
  });

  test("group sends ask for confirmation", async ({ page }) => {
    await boot(page);
    await openChat(page, "Book Club 📚");
    await page.getByTestId("composer-input").fill("Next meetup Friday?");
    await page.getByTestId("send-btn").click();
    await expect(page.getByTestId("confirm-send")).toBeVisible();
    await page.getByTestId("confirm-send-btn").click();
    await expect(page.getByTestId("message").filter({ hasText: "Next meetup Friday?" })).toBeVisible();
  });

  test("reacts to a message and replies to it", async ({ page }) => {
    await boot(page);
    await openChat(page, "Dad");
    const msg = page.getByTestId("message").filter({ hasNot: page.getByTestId("deleted") }).last();
    await msg.hover();
    await msg.getByTestId("react-btn").click();
    await page.getByTestId("react-picker").getByRole("menuitem", { name: "React 🙏" }).click();
    await expect(msg.getByTestId("reactions")).toContainText("🙏");
    await msg.hover();
    await msg.getByTestId("reply-btn").click();
    await expect(page.getByTestId("reply-chip")).toBeVisible();
    await page.getByTestId("composer-input").fill("replying!");
    await page.getByTestId("composer-input").press("Enter");
    const sent = page.getByTestId("message").filter({ hasText: "replying!" });
    await expect(sent.getByTestId("quoted")).toBeVisible();
  });

  test("clicking a quoted reply jumps to the original", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    const quoted = page.getByTestId("quoted").first();
    await expect(quoted).toBeVisible();
    await quoted.click();
    await expect(page.locator('[data-testid="message"] .flash')).toBeVisible();
  });
});

test.describe("search", () => {
  test("finds Devanagari text with highlights and jumps to the message", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-search").click();
    await page.getByTestId("search-input").fill("मिलते");
    const hit = page.getByTestId("search-hit").first();
    await expect(hit).toBeVisible();
    await expect(hit.locator("mark.hl")).toHaveText("मिलते");
    await expect(page.getByTestId("search-count")).toContainText("result");
    await hit.click();
    await expect(page.getByTestId("view-chats")).toBeVisible();
    await expect(page.locator('[data-testid="message"] .flash')).toContainText("मिलते");
  });

  test("filters by sender side and chat", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-search").click();
    await page.getByTestId("search-input").fill("birthday");
    await expect(page.getByTestId("search-hit").first()).toBeVisible();
    const all = await page.getByTestId("search-count").textContent();
    await page.getByRole("tab", { name: "Me", exact: true }).click();
    await expect(page.getByTestId("search-count")).not.toHaveText(all ?? "");
    const picker = page.getByTestId("search-chats");
    await picker.getByRole("combobox").fill("Family");
    await page.getByRole("option").filter({ hasText: "Family" }).first().click();
    await expect(page.getByTestId("search-hit").first()).toBeVisible();
    await expect
      .poll(async () => (await page.getByTestId("search-hit").allTextContents()).every((t) => t.includes("Family")))
      .toBe(true);
  });
});

test.describe("media & docs", () => {
  test("opens the PDF viewer and renders a page", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-docs").click();
    await page.getByRole("tab", { name: "PDF" }).click();
    await page.getByTestId("doc-row").filter({ hasText: "Local" }).first().click();
    const viewer = page.getByTestId("pdf-viewer");
    await expect(viewer.locator("canvas[data-rendered]").first()).toBeVisible();
    const zoom = await viewer.getByTestId("pdf-zoom").textContent();
    await viewer.getByRole("button", { name: "Zoom in" }).click();
    await expect(viewer.getByTestId("pdf-zoom")).not.toHaveText(zoom ?? "");
    await expect(viewer.getByText("/ 3")).toBeVisible();
  });

  test("downloads a missing document", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-docs").click();
    await page.getByTestId("docs-filter").fill("Menu");
    const row = page.getByTestId("doc-row").filter({ hasText: "Missing" }).first();
    await row.click();
    await page.getByTestId("doc-panel").getByRole("button", { name: "Download" }).click();
    await expect(page.getByTestId("doc-panel").getByRole("button", { name: "Open externally" })).toBeVisible();
  });

  test("media lightbox with keyboard navigation and zoom", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-media").click();
    await page.getByRole("tab", { name: "Downloaded" }).click();
    await page.getByTestId("media-tile").first().getByRole("button").first().click();
    const lb = page.getByTestId("lightbox");
    await expect(lb).toBeVisible();
    await expect(lb.getByText(/^1 \/ \d+$/)).toBeVisible();
    await page.keyboard.press("ArrowRight");
    await expect(lb.getByText(/^2 \/ \d+$/)).toBeVisible();
    await page.keyboard.press("ArrowLeft");
    const zoom = lb.getByTestId("zoom-level");
    if (await zoom.isVisible()) {
      await lb.getByRole("button", { name: "Zoom in" }).click();
      await expect(zoom).toHaveText("125%");
    }
    await page.keyboard.press("Escape");
    await expect(lb).toBeHidden();
  });
});

test.describe("system", () => {
  test("starts and cancels a job", async ({ page }) => {
    await boot(page);
    await page.keyboard.press("Alt+8");
    await expect(page.getByTestId("view-system")).toBeVisible();
    await page.getByTestId("job-action-reindex").click();
    const card = page.locator('[data-testid="job-card"][data-status="running"], [data-testid="job-card"][data-status="queued"]').first();
    await expect(card).toBeVisible();
    await card.getByTestId("job-cancel").click();
    await expect(page.locator('[data-testid="job-card"][data-status="cancelled"]').first()).toContainText("Rebuild search index");
  });

  test("backfill job runs to completion with live progress", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-system").click();
    await page.getByTestId("job-action-backfill").click();
    await page.getByTestId("backfill-days").fill("60");
    await page.getByTestId("start-backfill").click();
    await expect(page.getByRole("progressbar", { name: /Backfill media \(60 days\)/ })).toBeVisible();
    await expect(page.locator('[data-testid="job-card"][data-status="succeeded"]').filter({ hasText: "60 days" })).toBeVisible({ timeout: 15_000 });
  });

  test("bridge reconnect shows a pairing QR then connects", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-system").click();
    await page.getByTestId("reconnect").click();
    await expect(page.getByTestId("pairing").getByRole("img", { name: "Pairing QR code" })).toBeVisible();
    await expect(page.getByTestId("pairing")).toContainText("Link a device");
    await expect(page.getByTestId("bridge-state")).toHaveText("Connected", { timeout: 10_000 });
  });

  test("logs viewer filters by level and text", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-system").click();
    await page.getByRole("tab", { name: "Logs" }).click();
    await expect(page.getByTestId("log-line").first()).toBeVisible();
    await page.getByTestId("log-level").selectOption("error");
    await expect.poll(async () => {
      const levels = await page.getByTestId("log-line").locator("span.uppercase").allTextContents();
      return levels.length > 0 && levels.every((l) => l.trim() === "error");
    }).toBe(true);
    await page.getByTestId("log-tail").click();
    await expect(page.getByTestId("log-tail")).toContainText("Paused");
  });

  test("resolves an issue and exports a report", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-system").click();
    await page.getByRole("tab", { name: /Issues/ }).click();
    const rows = page.getByTestId("issue-row");
    await expect(rows.first()).toBeVisible();
    const before = await rows.count();
    await rows.first().getByTestId("resolve-issue").click();
    await expect(rows).toHaveCount(before - 1);
    await page.getByRole("tab", { name: /Resolved/ }).click();
    await expect(rows.first()).toBeVisible();
    await page.getByTestId("export-report").click();
    await expect(page.getByTestId("export-path")).toContainText(".zip");
  });
});

test.describe("automations", () => {
  test("creates and tests a rule", async ({ page }) => {
    await boot(page);
    await page.keyboard.press("Alt+7");
    await expect(page.getByTestId("rule-row").first()).toBeVisible();
    const count = await page.getByTestId("rule-row").count();
    await page.getByTestId("new-rule").click();
    const editor = page.getByTestId("rule-editor");
    await editor.getByTestId("rule-name").fill("Birthday wishes");
    const kw = editor.getByTestId("rule-keywords").locator("input");
    await kw.fill("birthday");
    await kw.press("Enter");
    await editor.getByTestId("test-chat").getByRole("combobox").fill("Ananya");
    await page.getByRole("option").filter({ hasText: "Ananya Rao" }).click();
    await expect(editor.getByTestId("test-sample")).toBeEnabled();
    await editor.getByTestId("run-test").click();
    await expect(editor.getByTestId("test-result")).toBeVisible();
    await expect(editor.getByTestId("test-result")).toContainText(/Would (not )?fire/);
    await editor.getByTestId("save-rule").click();
    await expect(page.getByTestId("rule-row")).toHaveCount(count + 1);
    await expect(page.getByTestId("rule-row").last()).toContainText("Birthday wishes");
  });

  test("rejects an invalid regex", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-automations").click();
    await page.getByTestId("new-rule").click();
    const editor = page.getByTestId("rule-editor");
    await editor.getByTestId("rule-name").fill("Bad");
    await editor.getByTestId("rule-regex").fill("([a-z]");
    await expect(editor.getByTestId("regex-status")).toContainText("Invalid");
    await editor.getByTestId("save-rule").click();
    await expect(editor.getByTestId("rule-invalid")).toBeVisible();
  });
});

test.describe("shell", () => {
  test("toggles the theme", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await boot(page);
    const html = page.locator("html");
    await expect(html).not.toHaveClass(/dark/);
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("theme-dark").click();
    await expect(html).toHaveClass(/dark/);
    await page.getByTestId("theme-light").click();
    await expect(html).not.toHaveClass(/dark/);
    await page.getByTestId("theme-toggle").click(); // light → dark
    await expect(html).toHaveClass(/dark/);
  });

  test("command palette jumps to a chat and runs a job", async ({ page }) => {
    await boot(page);
    await page.keyboard.press("Control+k");
    const pal = page.getByTestId("palette");
    await expect(pal).toBeVisible();
    await page.getByTestId("palette-input").fill("Priya Patel");
    await expect(pal.locator('[data-group="Chats"]').first()).toContainText("Priya Patel");
    await page.keyboard.press("Enter");
    await expect(pal).toBeHidden();
    await expect(page.getByTestId("thread-title")).toHaveText("Priya Patel");

    await page.keyboard.press("Control+k");
    await page.getByTestId("palette-input").fill("thumbnails");
    await expect(page.getByTestId("palette-item").first()).toContainText(/thumbnail/i);
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("toast").filter({ hasText: "Started" })).toBeVisible();
    await expect(page.getByTestId("nav-badge-system")).toBeVisible();

    await page.keyboard.press("Control+k");
    await page.getByTestId("palette-input").fill("open settings");
    // Wait for the filtered list (it updates after a short debounce) before choosing.
    await expect(page.getByTestId("palette-item").first()).toContainText(/settings/i);
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("view-settings")).toBeVisible();
  });

  test("keyboard shortcuts: Ctrl+F focuses search, Escape closes overlays", async ({ page }) => {
    await boot(page);
    await page.keyboard.press("Control+f");
    await expect(page.getByTestId("search-input")).toBeFocused();
    await page.keyboard.press("Control+k");
    await expect(page.getByTestId("palette")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("palette")).toBeHidden();
    await page.keyboard.press("Alt+1");
    await expect(page.getByTestId("view-chats")).toBeVisible();
  });

  test("settings save with validation", async ({ page }) => {
    await boot(page);
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("settings-days").fill("0");
    await expect(page.getByTestId("save-settings")).toBeDisabled();
    await page.getByTestId("settings-days").fill("45");
    await page.getByTestId("add-dir").click();
    await expect(page.getByText("/home/user/Pictures/Camera Exports")).toBeVisible();
    await page.getByTestId("save-settings").click();
    await expect(page.getByTestId("toast").filter({ hasText: "Settings saved" })).toBeVisible();
  });

  test("bridge-down errors show a friendly toast with a System action", async ({ page }) => {
    await boot(page);
    await page.evaluate(() => window.__waMock?.setBridge("unreachable"));
    await expect(page.getByTestId("bridge-pill")).toContainText("Offline");
    await openChat(page, "Dad");
    await page.getByTestId("composer-input").fill("will fail");
    await page.getByTestId("composer-input").press("Enter");
    const toast = page.getByTestId("toast").filter({ hasText: "WhatsApp bridge is offline" });
    await expect(toast).toBeVisible();
    await toast.getByRole("button", { name: "Open System" }).click();
    await expect(page.getByTestId("view-system")).toBeVisible();
  });

  test("live incoming messages appear in the open chat", async ({ page }) => {
    await boot(page);
    await openChat(page, "Ananya Rao");
    await page.evaluate(() => {
      const m = window.__waMock;
      const jid = m?.chats.find((c) => c.name === "Ananya Rao")?.jid;
      if (m && jid) m.incoming(jid, "Live message 🚀");
    });
    await expect(page.getByTestId("message").filter({ hasText: "Live message 🚀" })).toBeVisible();
  });
});
