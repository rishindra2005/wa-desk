import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearMocks } from "@tauri-apps/api/mocks";
import * as api from "../api";
import { MockBackend, matchTerms } from "./backend";
import { installMock } from "./install";
import { highlightSegments } from "../utils/text";
import type { Rule } from "../bindings/Rule";

const NOW = 1_760_000_000;
let b: MockBackend;

beforeEach(() => {
  b = installMock({ now: NOW, quiet: true, latency: 0, speed: 50 });
});
afterEach(() => {
  b.stop();
  clearMocks();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("seed", () => {
  it("is deterministic and realistic", () => {
    const other = new MockBackend({ now: NOW, quiet: true, latency: 0 });
    expect(other.totalMessages()).toBe(b.totalMessages());
    expect(b.chats.length).toBe(49);
    expect(b.totalMessages()).toBeGreaterThan(2500);
    expect(b.totalMessages()).toBeLessThan(3600);
    const all = b.chats.flatMap((c) => c.messages);
    expect(all.some((m) => m.deleted)).toBe(true);
    expect(all.some((m) => m.quoted)).toBe(true);
    expect(all.some((m) => m.reactions.length)).toBe(true);
    expect(all.some((m) => m.sender_jid?.endsWith("@lid"))).toBe(true);
    expect(all.some((m) => /[ऀ-ॿ]/.test(m.content))).toBe(true);
    for (const k of ["image", "video", "audio", "document", "sticker"]) expect(all.some((m) => m.media?.kind === k)).toBe(true);
    expect(all.some((m) => m.media?.source === "missing")).toBe(true);
  });
});

describe("commands", () => {
  it("handles every contract command", async () => {
    for (const cmd of api.COMMANDS) {
      try {
        await b.handle(cmd, {
          query: { filter: null, kind: "all", unread_only: false, cursor: null, limit: 1, chat_jid: b.chats[0]?.jid, before: null, text: "a", chat_jids: [], senders: [], media_kinds: [], from_me: null, after: null, offset: null, kinds: [], available: null },
          jid: b.chats[0]?.jid, chatJid: b.chats[0]?.jid, messageId: "x", radius: 1, limit: 1, id: 999, ids: null, enabled: false,
          filter: { level: null, contains: null, target: null, since: null, limit: 1 }, settings: b.settings,
          spec: { type: "reindex" }, rule: { name: "" }, sample: {}, req: { recipient: b.chats[0]?.jid, text: "x" }, dest: "/x",
        });
      } catch (e) {
        expect((e as Error).message).not.toMatch(/Unknown command/);
      }
    }
  });

  it("lists chats newest first with filters", async () => {
    const all = await api.listChats({ filter: null, kind: "all", unread_only: false, cursor: null, limit: 500 });
    const ts = all.items.map((c) => c.last_message_ts ?? 0);
    expect([...ts].sort((x, y) => y - x)).toEqual(ts);
    const groups = await api.listChats({ filter: null, kind: "group", unread_only: false, cursor: null, limit: 500 });
    expect(groups.items.length).toBe(17);
    expect(groups.items.every((c) => c.is_group)).toBe(true);
    const f = await api.listChats({ filter: "राहुल", kind: "all", unread_only: false, cursor: null, limit: 10 });
    expect(f.items.map((c) => c.name)).toContain("राहुल शर्मा");
    const page = await api.listChats({ filter: null, kind: "all", unread_only: false, cursor: null, limit: 10 });
    expect(page.next_cursor).toBe("10");
  });

  it("pages messages backwards in ascending order", async () => {
    const jid = b.chats[0]!.jid;
    const p1 = await api.getMessages({ chat_jid: jid, before: null, after: null, limit: 80 });
    expect(p1.items.length).toBe(80);
    expect(p1.items[0]!.ts).toBeLessThanOrEqual(p1.items[79]!.ts);
    const p2 = await api.getMessages({ chat_jid: jid, before: p1.next_cursor, after: null, limit: 80 });
    expect(p2.items[79]!.ts).toBeLessThanOrEqual(p1.items[0]!.ts);
    expect(p2.items.some((m) => p1.items.some((x) => x.id === m.id))).toBe(false);
  });

  it("returns message context around a target", async () => {
    const c = b.chats[0]!;
    const target = c.messages[300]!;
    const ctx = await api.getMessageContext(c.jid, target.id, 5);
    expect(ctx.length).toBe(11);
    expect(ctx[5]!.id).toBe(target.id);
    await expect(api.getMessageContext(c.jid, "nope", 5)).rejects.toMatchObject({ kind: "not_found" });
  });

  it("search returns UTF-8 byte highlights that map back to the query", async () => {
    const r = await api.search({ text: "मिलते", chat_jids: [], senders: [], media_kinds: [], from_me: null, after: null, before: null, limit: 5, offset: null });
    expect(r.total).toBeGreaterThan(0);
    for (const h of r.hits) {
      const segs = highlightSegments(h.message.content, h.highlights).filter((s) => s.hit);
      expect(segs.map((s) => s.text)).toContain("मिलते");
    }
    const emoji = matchTerms("ok 😂 lol", ["lol"]);
    expect(emoji).toEqual([[8, 11]]);
  });

  it("search filters by media kind and sender side", async () => {
    const r = await api.search({ text: "", chat_jids: [], senders: [], media_kinds: ["document"], from_me: true, after: null, before: null, limit: 500, offset: null });
    expect(r.hits.every((h) => h.message.media?.kind === "document" && h.message.is_from_me)).toBe(true);
  });

  it("send requires a connected bridge and emits db:new-messages", async () => {
    const jid = b.chats[1]!.jid;
    await expect(api.sendMessage({ recipient: jid, text: "hi", attachment: null, reply_to: null, mentions: [] })).rejects.toMatchObject({ kind: "bridge_down" });
    b.setBridge("connected");
    const cb = vi.fn();
    const un = await api.onNewMessages(cb);
    const res = await api.sendMessage({ recipient: jid, text: "hello", attachment: null, reply_to: null, mentions: [] });
    expect(res.ok).toBe(true);
    expect(cb).toHaveBeenCalledWith(expect.objectContaining({ chat_jids: [jid], count: 1 }));
    const last = await api.getMessages({ chat_jid: jid, before: null, after: null, limit: 1 });
    expect(last.items[0]).toMatchObject({ content: "hello", is_from_me: true });
    await un();
  });

  it("react replaces my reaction and removes with empty emoji", async () => {
    b.setBridge("connected");
    const c = b.chats[0]!;
    const m = c.messages.find((x) => !x.deleted)!;
    await api.react({ chat_jid: c.jid, message_id: m.id, emoji: "👍" });
    await api.react({ chat_jid: c.jid, message_id: m.id, emoji: "❤️" });
    expect(m.reactions.filter((r) => r.is_from_me).map((r) => r.emoji)).toEqual(["❤️"]);
    await api.react({ chat_jid: c.jid, message_id: m.id, emoji: "" });
    expect(m.reactions.some((r) => r.is_from_me)).toBe(false);
  });

  it("ensure_media downloads missing files and fails for corrupt ones", async () => {
    b.setBridge("connected");
    const all = b.chats.flatMap((c) => c.messages);
    const missing = all.find((m) => m.media?.source === "missing" && !m.media.filename?.startsWith("corrupt"))!;
    const ref = await api.ensureMedia(missing.chat_jid, missing.id);
    expect(ref.source).not.toBe("missing");
    expect(ref.url).toBeTruthy();
    const corrupt = all.find((m) => m.media?.filename?.startsWith("corrupt"));
    if (corrupt) await expect(api.ensureMedia(corrupt.chat_jid, corrupt.id)).rejects.toMatchObject({ kind: "external" });
  });

  it("jobs progress and finish via events", async () => {
    const progress = vi.fn();
    const done = new Promise((resolve) => void api.onJobFinished(resolve));
    await api.onJobProgress(progress);
    const job = await api.startJob({ type: "thumbnails" });
    expect(job.status).toBe("queued");
    const fin = (await done) as { status: string; id: number };
    expect(fin).toMatchObject({ id: job.id, status: "succeeded" });
    expect(progress).toHaveBeenCalled();
    await expect(api.startJob({ type: "backfill", days: 0, kinds: [], workers: 1, retry_failed: false })).rejects.toMatchObject({ kind: "invalid" });
  });

  it("cancel_job cancels a running job", async () => {
    const job = await api.startJob({ type: "reindex" });
    await api.cancelJob(job.id);
    const jobs = await api.listJobs(10);
    expect(jobs.find((j) => j.id === job.id)?.status).toBe("cancelled");
    await expect(api.cancelJob(job.id)).rejects.toMatchObject({ kind: "invalid" });
  });

  it("rules CRUD validates and test_rule explains", async () => {
    const rules = await api.listRules();
    const bad = { ...rules[0]!, id: null, trigger: { ...rules[0]!.trigger, regex: "([" } } as Rule;
    await expect(api.saveRule(bad)).rejects.toMatchObject({ kind: "invalid" });
    const created = await api.saveRule({ ...rules[0]!, id: null, name: "Copy" });
    expect(created.id).toBeGreaterThan(3);
    const sample = b.chats[0]!.messages.find((m) => !m.is_from_me)!;
    const ev = await api.testRule(created, sample);
    expect(ev.reasons.length).toBeGreaterThan(0);
    await api.deleteRule(created.id!);
    expect((await api.listRules()).some((r) => r.id === created.id)).toBe(false);
  });

  it("logs filter by level, text and limit (newest first)", async () => {
    const errs = await api.queryLogs({ level: "error", contains: null, target: null, since: null, limit: 50 });
    expect(errs.every((l) => l.level === "error")).toBe(true);
    const all = await api.queryLogs({ level: "trace", contains: null, target: null, since: null, limit: 20 });
    expect(all.length).toBe(20);
    expect(all[0]!.ts).toBeGreaterThanOrEqual(all[19]!.ts);
  });

  it("issues resolve, clear and export", async () => {
    const issues = await api.listIssues();
    expect(issues[0]!.status).toBe("open");
    await api.resolveIssue(issues[0]!.id);
    const n = await api.clearResolvedIssues();
    expect(n).toBe(2);
    const path = await api.exportIssueBundle(null);
    expect(path).toMatch(/\.zip$/);
  });

  it("settings validate and automation toggle", async () => {
    const s = await api.getSettings();
    await expect(api.setSettings({ ...s, backfill_days: 0 })).rejects.toMatchObject({ kind: "invalid" });
    expect((await api.setAutomationEnabled(true)).automation_enabled).toBe(true);
  });

  it("bridge cycles through pairing with a QR on reconnect", async () => {
    const states: string[] = [];
    await api.onBridgeState((s) => states.push(s.state));
    await api.reconnect();
    await vi.waitFor(() => expect(states).toContain("connected"), { timeout: 2000 });
    expect(states).toContain("pairing");
  });
});
