import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { clearMocks } from "@tauri-apps/api/mocks";
import { installMock } from "./mock/install";
import type { MockBackend } from "./mock/backend";
import * as api from "./api";
import StatusTicks, { TICK_TEXT, tickTone, type TickState } from "./components/StatusTicks.svelte";
import Avatar from "./components/Avatar.svelte";
import ContextMenu from "./components/ContextMenu.svelte";
import ChatList from "./views/chats/ChatList.svelte";
import { ctxMenu } from "./state/menu.svelte";
import { avatarFailed, fullAvatarUrl, resetAvatarCache, markAvatarFailed } from "./avatar";
import { clampPosition, isMenuKey, nextIndex, tidyItems, type MenuItem } from "./utils/menu";
import { applyClick, inOrder, MAX_FORWARD_TARGETS, rangeIds, toggleTarget } from "./utils/selection";
import { convertOptions, convertParams } from "./utils/convert";
import { clearDraft, hasDraft, loadDraft, saveDraft } from "./utils/drafts";
import { chatLabel, displayName } from "./utils/format";

let b: MockBackend;
beforeEach(() => {
  b = installMock({ now: 1_760_000_000, quiet: true, latency: 0, speed: 50 });
  localStorage.clear();
  resetAvatarCache();
});
afterEach(() => {
  cleanup();
  ctxMenu.isOpen = false;
  b.stop();
  clearMocks();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("delivery ticks", () => {
  const cases: [TickState, string, "grey" | "blue" | "red"][] = [
    ["pending", "Sending…", "grey"],
    ["sent", "Sent", "grey"],
    ["delivered", "Delivered", "grey"],
    ["read", "Read", "blue"],
    ["played", "Played", "blue"],
    ["failed", "Not sent", "red"],
  ];
  for (const [status, text, tone] of cases) {
    it(`renders ${status} with tooltip and ${tone} tone`, () => {
      render(StatusTicks, { status });
      const el = screen.getByTestId("tick");
      expect(el.getAttribute("data-status")).toBe(status);
      expect(el.getAttribute("title")).toBe(text);
      expect(el.getAttribute("aria-label")).toBe(text);
      expect(el.getAttribute("data-tone")).toBe(tone);
      expect(TICK_TEXT[status]).toBe(text);
      expect(tickTone(status)).toBe(tone);
    });
  }

  it("uses one tick for sent and two for delivered/read", () => {
    const count = (s: TickState) => {
      const { container, unmount } = render(StatusTicks, { status: s });
      const n = container.querySelectorAll("svg").length;
      const paths = container.querySelectorAll("svg path").length;
      unmount();
      return { n, paths };
    };
    expect(count("sent").paths).toBeLessThan(count("delivered").paths);
    expect(count("delivered").paths).toBe(count("read").paths);
  });

  it("chat list shows ticks for own last messages", async () => {
    render(ChatList, { selected: null, onselect: () => {} });
    await waitFor(() => expect(screen.queryAllByTestId("chat-row").length).toBeGreaterThan(5));
    expect(screen.queryAllByTestId("tick").length).toBeGreaterThan(0);
  });
});

describe("avatar", () => {
  it("falls back to initials after the image errors and caches the failure", async () => {
    const url = "data:image/png;base64,AAAA";
    render(Avatar, { name: "Dad", id: "d@s.whatsapp.net", url, type: "direct" });
    const img = document.querySelector("img") as HTMLImageElement;
    expect(img).toBeTruthy();
    await fireEvent.error(img);
    await waitFor(() => expect(screen.getByTestId("avatar-initials").textContent).toBe("D"));
    expect(screen.getByTestId("avatar").getAttribute("data-state")).toBe("fallback");
    expect(avatarFailed(url)).toBe(true);
    cleanup();
    // A second avatar with the same URL never requests the image again.
    render(Avatar, { name: "Dad", id: "d@s.whatsapp.net", url, type: "direct" });
    expect(document.querySelector("img")).toBeNull();
    expect(screen.getByTestId("avatar-initials")).toBeTruthy();
  });

  it("uses a type icon for groups without a picture", () => {
    markAvatarFailed("x://g");
    render(Avatar, { name: "Family", id: "g@g.us", url: "x://g", type: "group" });
    expect(screen.getByTestId("avatar-icon")).toBeTruthy();
    expect(screen.queryByTestId("avatar-initials")).toBeNull();
  });

  it("fades in once loaded and is lazy", async () => {
    render(Avatar, { name: "Ana", id: "a@s.whatsapp.net", url: "data:image/gif;base64,R0lGODlhAQABAAAAACw=" });
    const img = document.querySelector("img") as HTMLImageElement;
    expect(img.getAttribute("loading")).toBe("lazy");
    expect(img.className).toContain("opacity-0");
    await fireEvent.load(img);
    await waitFor(() => expect(img.className).toContain("opacity-100"));
  });

  it("builds full-size URLs", () => {
    expect(fullAvatarUrl("wa-media://localhost/a/x")).toBe("wa-media://localhost/a/x?full=1");
    expect(fullAvatarUrl("wa-media://localhost/a/x?v=1")).toBe("wa-media://localhost/a/x?v=1&full=1");
    expect(fullAvatarUrl("data:image/svg+xml,abc")).toBe("data:image/svg+xml,abc");
  });
});

describe("context menu", () => {
  it("clamps inside the viewport (flips left/up)", () => {
    expect(clampPosition(10, 10, 200, 300, 1000, 800)).toEqual({ left: 10, top: 10 });
    const p = clampPosition(950, 780, 200, 300, 1000, 800);
    expect(p.left + 200).toBeLessThanOrEqual(1000);
    expect(p.top + 300).toBeLessThanOrEqual(800);
    expect(p).toEqual({ left: 750, top: 480 });
    // Too large for either side: pinned to the margin.
    expect(clampPosition(50, 50, 900, 700, 800, 600)).toEqual({ left: 8, top: 8 });
  });

  it("skips separators and disabled items when navigating, and wraps", () => {
    const items: MenuItem[] = [
      { id: "a", label: "A" },
      { id: "s", separator: true },
      { id: "b", label: "B", disabled: true },
      { id: "c", label: "C" },
    ];
    expect(nextIndex(items, -1, 1)).toBe(0);
    expect(nextIndex(items, 0, 1)).toBe(3);
    expect(nextIndex(items, 3, 1)).toBe(0);
    expect(nextIndex(items, 0, -1)).toBe(3);
    expect(nextIndex([], 0, 1)).toBe(-1);
  });

  it("recognises the keyboard triggers and tidies separators", () => {
    expect(isMenuKey({ key: "ContextMenu", shiftKey: false })).toBe(true);
    expect(isMenuKey({ key: "F10", shiftKey: true })).toBe(true);
    expect(isMenuKey({ key: "F10", shiftKey: false })).toBe(false);
    const t = tidyItems([{ id: "x", separator: true }, { id: "a", label: "A" }, { id: "y", separator: true }, { id: "z", separator: true }, { id: "b", label: "B" }, { id: "w", separator: true }]);
    expect(t.map((i) => i.id)).toEqual(["a", "y", "b"]);
  });

  it("opens, navigates with the keyboard, activates, and closes on Escape", async () => {
    const onA = vi.fn();
    const onB = vi.fn();
    render(ContextMenu);
    ctxMenu.open([{ id: "a", label: "Alpha", onselect: onA }, { id: "b", label: "Beta", onselect: onB }], 20, 20);
    const menu = await screen.findByTestId("context-menu");
    await fireEvent.keyDown(menu, { key: "ArrowDown" });
    await fireEvent.keyDown(menu, { key: "ArrowDown" });
    await fireEvent.keyDown(menu, { key: "Enter" });
    expect(onB).toHaveBeenCalled();
    expect(onA).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByTestId("context-menu")).toBeNull());

    ctxMenu.open([{ id: "a", label: "Alpha", onselect: onA }], 5, 5);
    const again = await screen.findByTestId("context-menu");
    await fireEvent.keyDown(again, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("context-menu")).toBeNull());
  });

  it("closes when clicking outside", async () => {
    render(ContextMenu);
    ctxMenu.open([{ id: "a", label: "Alpha" }], 5, 5);
    await screen.findByTestId("context-menu");
    await fireEvent.pointerDown(document.body);
    await waitFor(() => expect(screen.queryByTestId("context-menu")).toBeNull());
  });

  it("chat list rows open the right-click menu with the expected actions", async () => {
    render(ChatList, { selected: null, onselect: () => {} });
    render(ContextMenu);
    await waitFor(() => expect(screen.queryAllByTestId("chat-row").length).toBeGreaterThan(3));
    await fireEvent.contextMenu(screen.getAllByTestId("chat-row")[0]!, { clientX: 40, clientY: 40 });
    const menu = await screen.findByTestId("context-menu");
    for (const id of ["open", "read", "info", "search", "export", "jid"]) expect(menu.querySelector(`[data-testid="menu-${id}"]`)).toBeTruthy();
  });
});

describe("selection logic", () => {
  const ids = ["a", "b", "c", "d", "e", "f"];
  it("computes inclusive ranges in either direction", () => {
    expect(rangeIds(ids, "b", "e")).toEqual(["b", "c", "d", "e"]);
    expect(rangeIds(ids, "e", "b")).toEqual(["b", "c", "d", "e"]);
    expect(rangeIds(ids, null, "c")).toEqual(["c"]);
    expect(rangeIds(ids, "zz", "c")).toEqual(["c"]);
    expect(rangeIds(ids, "a", "nope")).toEqual([]);
  });

  it("toggles on click and extends on shift-click from the anchor", () => {
    let r = applyClick(new Set(), ids, null, "b", false);
    expect([...r.selected]).toEqual(["b"]);
    r = applyClick(r.selected, ids, r.anchor, "e", true);
    expect(inOrder(ids, r.selected)).toEqual(["b", "c", "d", "e"]);
    expect(r.anchor).toBe("b");
    r = applyClick(r.selected, ids, r.anchor, "c", false);
    expect(inOrder(ids, r.selected)).toEqual(["b", "d", "e"]);
    expect(r.anchor).toBe("c");
  });

  it("keeps chronological order for copy/forward", () => {
    expect(inOrder(ids, new Set(["e", "a", "c"]))).toEqual(["a", "c", "e"]);
  });
});

describe("copy text", () => {
  it("returns WhatsApp-style lines in chronological order", async () => {
    const chat = b.chats[0]!;
    const picks = [chat.messages[10]!, chat.messages[3]!, chat.messages[7]!];
    const text = await api.copyMessagesText(chat.jid, picks.map((m) => m.id));
    const lines = text.split("\n");
    expect(lines).toHaveLength(3);
    expect(lines.every((l) => /^\[\d{2}\/\d{2}\/\d{4}, \d{2}:\d{2}\] .+: /.test(l))).toBe(true);
    const ts = [chat.messages[3]!, chat.messages[7]!, chat.messages[10]!];
    ts.forEach((m, i) => expect(lines[i]).toContain(m.is_from_me ? "Me:" : `${m.sender_name}:`));
  });
});

describe("forward", () => {
  it("limits targets to five in the UI helper and in the backend", async () => {
    let sel: string[] = [];
    for (let i = 0; i < 6; i++) {
      const r = toggleTarget(sel, `c${i}`);
      sel = r.selected;
      expect(r.limited).toBe(i === 5);
    }
    expect(sel).toHaveLength(MAX_FORWARD_TARGETS);
    expect(toggleTarget(sel, "c0").selected).toHaveLength(4);

    const from = b.chats[0]!;
    const to = b.chats.filter((c) => c.chat_type === "direct").slice(1, 7).map((c) => c.jid);
    await expect(api.forwardMessages({ from_chat: from.jid, message_ids: [from.messages[0]!.id], to })).rejects.toMatchObject({ kind: "invalid" });
  });

  it("returns one result per message x target, failing admin-only chats", async () => {
    const from = b.chats[0]!;
    const direct = b.chats[1]!;
    const community = b.chats.find((c) => c.chat_type === "community")!;
    const res = await api.forwardMessages({ from_chat: from.jid, message_ids: [from.messages[0]!.id, from.messages[1]!.id], to: [direct.jid, community.jid] });
    expect(res).toHaveLength(4);
    expect(res.filter((r) => r.ok)).toHaveLength(2);
    expect(res.filter((r) => !r.ok)[0]!.message).toMatch(/admins/);
  });
});

describe("convert options", () => {
  it("offers kind-valid conversions with sensible defaults", () => {
    expect(convertOptions("document", "Trip.docx").map((o) => o.kind.type)).toEqual(["office_to_pdf"]);
    expect(convertOptions("document", "Budget.xlsx").map((o) => o.kind.type)).toEqual(["office_to_pdf"]);
    expect(convertOptions("document", "Invoice.pdf").map((o) => o.kind.type)).toEqual(["pdf_compress", "pdf_to_images"]);
    expect(convertOptions("document", "photos.zip")).toEqual([]);
    expect(convertOptions("document", "notes.txt")).toEqual([]);
    expect(convertOptions("image", "a.jpg").map((o) => o.id)).toEqual(["image_jpeg", "image_webp", "image_png"]);
    expect(convertOptions("image", "a.jpg")[0]!.kind).toEqual({ type: "image_compress", quality: 80, max_dim: 2048, format: "jpeg" });
    expect(convertOptions("video", "a.mp4")[0]!.kind).toEqual({ type: "video_compress", crf: 28, max_height: 720 });
    expect(convertOptions("audio", "a.opus")[0]!.kind).toEqual({ type: "audio_to_opus", bitrate_kbps: 32 });
    expect(convertOptions("sticker", "s.webp")).toEqual([]);
  });

  it("exposes tunable parameters only where they exist", () => {
    expect(convertParams({ type: "office_to_pdf" })).toEqual([]);
    expect(convertParams({ type: "pdf_compress", level: "ebook" })).toEqual([]);
    expect(convertParams({ type: "image_compress", quality: 80, max_dim: 2048, format: "jpeg" }).map((p) => p.key)).toEqual(["quality", "max_dim"]);
    expect(convertParams({ type: "video_compress", crf: 28, max_height: 720 }).map((p) => p.key)).toEqual(["crf", "max_height"]);
  });

  it("starts a message conversion job that finishes with an output", async () => {
    const m = b.chats.flatMap((c) => c.messages.map((x) => ({ c, x }))).find(({ x }) => x.media?.kind === "image" && x.media.url)!;
    const spec = convertOptions("image", m.x.media!.filename)[0]!;
    const job = await api.startJob({ type: "convert", input: { type: "message", chat_jid: m.c.jid, message_id: m.x.id }, kind: spec.kind, output_dir: null });
    expect(job.spec.type).toBe("convert");
    await waitFor(() => expect(b.jobs.find((j) => j.id === job.id)?.status).toBe("succeeded"), { timeout: 5000 });
    expect(b.jobs.find((j) => j.id === job.id)!.outputs.length).toBeGreaterThan(0);
  });
});

describe("drafts", () => {
  it("saves, loads and clears per chat", () => {
    saveDraft("a@s.whatsapp.net", "hello");
    saveDraft("b@g.us", "other");
    expect(loadDraft("a@s.whatsapp.net")).toBe("hello");
    expect(hasDraft("b@g.us")).toBe(true);
    clearDraft("a@s.whatsapp.net");
    expect(loadDraft("a@s.whatsapp.net")).toBe("");
    expect(loadDraft("b@g.us")).toBe("other");
  });

  it("treats whitespace-only text as empty", () => {
    saveDraft("x", "keep");
    saveDraft("x", "   \n");
    expect(hasDraft("x")).toBe(false);
  });

  it("never throws when storage is unavailable", () => {
    const get = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    const set = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    expect(() => saveDraft("x", "y")).not.toThrow();
    expect(loadDraft("x")).toBe("");
    get.mockRestore();
    set.mockRestore();
  });
});

describe("naming", () => {
  it("never shows raw digit names", () => {
    expect(chatLabel({ name: "120363041234567890", jid: "1@g.us", chat_type: "group" })).toBe("Unnamed group");
    expect(chatLabel({ name: "919731234567", jid: "919731234567@s.whatsapp.net", chat_type: "direct" })).toBe("+91 97312 34567");
    expect(chatLabel({ name: "204958372615402", jid: "204958372615402@lid", chat_type: "direct" })).toBe("Unknown contact");
    expect(chatLabel({ name: "Ana", jid: "x", chat_type: "direct" })).toBe("Ana");
    expect(displayName("Rahul K", "1182@lid")).toBe("~Rahul K");
  });
});

describe("mock contract coverage", () => {
  it("returns every profile type", async () => {
    const contact = await api.getProfile(b.chats[0]!.jid);
    expect(contact.type).toBe("contact");
    const group = await api.getProfile(b.chats.find((c) => c.chat_type === "group")!.jid);
    expect(group.type === "group" && group.participants.some((p) => p.is_admin)).toBe(true);
    const community = await api.getProfile(b.chats.find((c) => c.chat_type === "community")!.jid);
    expect(community.type === "group" && community.sub_groups.length).toBe(3);
    const channel = await api.getProfile(b.chats.find((c) => c.chat_type === "channel")!.jid);
    expect(channel.type).toBe("channel");
    const big = b.chats.find((c) => c.participants.length >= 200);
    expect(big).toBeTruthy();
    expect(Math.min(...b.chats.filter((c) => c.chat_type === "group").map((c) => c.participants.length))).toBeLessThanOrEqual(3);
  });

  it("pages forward with `after` until the newest message", async () => {
    const c = b.chats[0]!;
    let cursor: string | null = c.messages[100]!.id;
    let seen = 0;
    for (let i = 0; i < 20 && cursor; i++) {
      const p = await api.getMessages({ chat_jid: c.jid, before: null, after: cursor, limit: 100 });
      seen += p.items.length;
      cursor = p.next_cursor;
    }
    expect(seen).toBe(c.messages.length - 101);
    expect(cursor).toBeNull();
  });

  it("refreshes placeholder names once", async () => {
    expect(await api.refreshChatNames()).toBe(2);
    expect(await api.refreshChatNames()).toBe(0);
  });

  it("stars, lists links and exports", async () => {
    const c = b.chats[0]!;
    await api.setStarred(c.jid, [c.messages[0]!.id], true);
    expect((await api.listStarred(c.jid, null, 500)).items.some((h) => h.message.id === c.messages[0]!.id)).toBe(true);
    expect((await api.chatStats(c.jid)).starred).toBeGreaterThan(0);
    expect((await api.listLinks(null, null, 50)).items.length).toBeGreaterThan(0);
    expect(await api.exportChat({ chat_jid: c.jid, format: "txt", message_ids: [], after: null, before: null, include_media: false })).toMatch(/\.txt$/);
  });
});
