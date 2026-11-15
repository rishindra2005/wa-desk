// In-memory mock of every command in docs/CONTRACT.md, with simulated events.
// Events are delivered through the same `listen()` callback registry that the
// real Tauri runtime uses (see install.ts).

import type { AppError } from "../bindings/AppError";
import type { AutomationEvent } from "../bindings/AutomationEvent";
import type { ChatQuery } from "../bindings/ChatQuery";
import type { ChatSummary } from "../bindings/ChatSummary";
import type { ConnectionState } from "../bindings/ConnectionState";
import type { BridgeState } from "../bindings/BridgeState";
import type { Issue } from "../bindings/Issue";
import type { JobRecord } from "../bindings/JobRecord";
import type { JobSpec } from "../bindings/JobSpec";
import type { LogFilter } from "../bindings/LogFilter";
import type { LogLevel } from "../bindings/LogLevel";
import type { LogLine } from "../bindings/LogLine";
import type { MediaItem } from "../bindings/MediaItem";
import type { MediaQuery } from "../bindings/MediaQuery";
import type { MediaRef } from "../bindings/MediaRef";
import type { Message } from "../bindings/Message";
import type { MessagesQuery } from "../bindings/MessagesQuery";
import type { Overview } from "../bindings/Overview";
import type { Page } from "../bindings/Page";
import type { PreviewRef } from "../bindings/PreviewRef";
import type { ReactReq } from "../bindings/ReactReq";
import type { Rule } from "../bindings/Rule";
import type { SearchHit } from "../bindings/SearchHit";
import type { SearchQuery } from "../bindings/SearchQuery";
import type { SearchResults } from "../bindings/SearchResults";
import type { SendReq } from "../bindings/SendReq";
import type { Settings } from "../bindings/Settings";
import type { EventMap, EventName } from "../api";
import { MOCK_PDF_URL, qrSvg } from "./assets";
import { buildMedia, generateSeed, mediaEmoji, phoneJid, randomLogLine, type MockChat, type Seed } from "./seed";
import { evaluateRule, hasErrors, validateRule } from "../utils/rules";
import { indexToByteOffset, extOf, baseName, docBucket } from "../utils/text";
import { mulberry32 } from "../utils/misc";

export class MockFail extends Error {
  constructor(public readonly err: AppError) {
    super(err.message);
  }
}

function fail(kind: AppError["kind"], message: string, detail?: string): never {
  throw new MockFail(detail ? { kind, message, detail } : { kind, message });
}

export interface MockOptions {
  /** Fixed "now" (unix seconds) for deterministic tests. */
  now?: number;
  /** Disable spontaneous incoming messages and log noise (e2e/tests). */
  quiet?: boolean;
  /** Per-call latency in ms (default 40). */
  latency?: number;
  /** Speed multiplier for timers (jobs, bridge); 1 = realistic. */
  speed?: number;
}

type Emit = <K extends EventName>(name: K, payload: EventMap[K]) => void;

const LEVELS: LogLevel[] = ["trace", "debug", "info", "warn", "error"];

const EXPORTS = "/home/user/.local/share/wa-desk/exports";

export class MockBackend {
  readonly seed: Seed;
  chats: MockChat[];
  rules: Rule[];
  autoLog: AutomationEvent[];
  logs: LogLine[];
  issues: Issue[];
  jobs: JobRecord[];
  settings: Settings;
  conn: ConnectionState;
  indexed = 0;
  /** Calls to opener/dialog/notification plugins, for assertions in tests. */
  readonly pluginCalls: { cmd: string; args: unknown }[] = [];
  private emitFn: Emit = () => {};
  private timers = new Set<ReturnType<typeof setTimeout>>();
  private intervals = new Set<ReturnType<typeof setInterval>>();
  private jobTimers = new Map<number, ReturnType<typeof setInterval>>();
  private nextId = 1000;
  private rowid = 100_000;
  private rnd = mulberry32(7);
  readonly opts: Required<MockOptions>;

  constructor(opts: MockOptions = {}) {
    this.opts = {
      now: opts.now ?? Math.floor(Date.now() / 1000),
      quiet: opts.quiet ?? false,
      latency: opts.latency ?? 40,
      speed: opts.speed ?? 1,
    };
    this.seed = generateSeed(this.opts.now);
    this.chats = this.seed.chats;
    this.rules = this.seed.rules;
    this.autoLog = this.seed.automationLog;
    this.logs = this.seed.logs;
    this.issues = this.seed.issues;
    this.jobs = this.seed.jobs;
    this.settings = {
      theme: "system",
      automation_enabled: false,
      automation_max_per_hour: 20,
      desktop_notifications: true,
      confirm_sends: true,
      backfill_days: 30,
      start_hidden: false,
      autostart: false,
      link_extra_dirs: ["/home/user/Pictures/WhatsApp Images"],
    };
    this.conn = this.connState("connecting", "Connecting to the WhatsApp bridge…");
    this.indexed = Math.floor(this.totalMessages() * 0.7);
  }

  /** Wire the event sink and start background simulation. */
  start(emit: Emit): void {
    this.emitFn = emit;
    this.later(900, () => this.setBridge("connected"));
    this.every(400, () => {
      const total = this.totalMessages();
      if (this.indexed < total) this.indexed = Math.min(total, this.indexed + Math.ceil(total / 60));
    });
    this.every(2000, () => this.addLog(randomLogLine(this.rnd, this.nowSec())));
    if (!this.opts.quiet) {
      this.every(45_000, () => {
        const chat = this.chats[Math.floor(this.rnd() * 8)];
        if (chat) this.incoming(chat.jid, this.pickIncomingText());
      });
    }
  }

  stop(): void {
    for (const t of this.timers) clearTimeout(t);
    for (const t of this.intervals) clearInterval(t);
    for (const t of this.jobTimers.values()) clearInterval(t);
    this.timers.clear();
    this.intervals.clear();
    this.jobTimers.clear();
  }

  private later(ms: number, fn: () => void): void {
    const t = setTimeout(() => {
      this.timers.delete(t);
      fn();
    }, ms / this.opts.speed);
    this.timers.add(t);
  }

  private every(ms: number, fn: () => void): void {
    this.intervals.add(setInterval(fn, ms / this.opts.speed));
  }

  private emit<K extends EventName>(name: K, payload: EventMap[K]): void {
    this.emitFn(name, JSON.parse(JSON.stringify(payload)) as EventMap[K]);
  }

  nowSec(): number {
    // Advance with wall-clock from the seeded "now".
    return this.opts.now + Math.floor((Date.now() - this.bootMs) / 1000);
  }
  private readonly bootMs = Date.now();

  totalMessages(): number {
    return this.chats.reduce((n, c) => n + c.messages.length, 0);
  }

  // ------------------------------------------------------------ dispatch

  /** Handle one command. Returns the (JSON-cloned) result or throws MockFail. */
  async handle(cmd: string, args: Record<string, unknown>): Promise<unknown> {
    if (this.opts.latency > 0) await new Promise((r) => setTimeout(r, this.opts.latency));
    const a = args as never;
    switch (cmd) {
      case "overview": return this.overview();
      case "list_chats": return this.listChats((a as { query: ChatQuery }).query);
      case "get_chat": return this.summary(this.chat((a as { jid: string }).jid));
      case "get_messages": return this.getMessages((a as { query: MessagesQuery }).query);
      case "get_message_context": {
        const { chatJid, messageId, radius } = a as { chatJid: string; messageId: string; radius: number };
        return this.context(chatJid, messageId, radius);
      }
      case "search": return this.search((a as { query: SearchQuery }).query);
      case "list_media": return this.listMedia((a as { query: MediaQuery }).query);
      case "ensure_media": {
        const { chatJid, messageId } = a as { chatJid: string; messageId: string };
        return this.ensureMedia(chatJid, messageId);
      }
      case "preview_doc": {
        const { chatJid, messageId } = a as { chatJid: string; messageId: string };
        return this.previewDoc(chatJid, messageId);
      }
      case "open_media_external": {
        const { chatJid, messageId } = a as { chatJid: string; messageId: string };
        const m = this.message(chatJid, messageId);
        if (!m.media?.url) fail("not_found", "Media is not downloaded yet");
        this.pluginCalls.push({ cmd, args });
        return null;
      }
      case "save_media_as": {
        const { chatJid, messageId, dest } = a as { chatJid: string; messageId: string; dest: string };
        const m = this.message(chatJid, messageId);
        if (!m.media?.url) fail("not_found", "Media is not downloaded yet");
        if (!dest.startsWith("/")) fail("invalid", "Destination must be an absolute path");
        return dest;
      }
      case "send_message": return this.send((a as { req: SendReq }).req);
      case "react": return this.react((a as { req: ReactReq }).req);
      case "mark_read": {
        this.chat((a as { chatJid: string }).chatJid).unread = false;
        return null;
      }
      case "connection_status": return this.conn;
      case "reconnect": return this.reconnect();
      case "start_job": return this.startJob((a as { spec: JobSpec }).spec);
      case "cancel_job": return this.cancelJob((a as { id: number }).id);
      case "list_jobs": return this.jobs.slice(0, (a as { limit: number }).limit);
      case "list_rules": return this.rules;
      case "save_rule": return this.saveRule((a as { rule: Rule }).rule);
      case "delete_rule": {
        const id = (a as { id: number }).id;
        if (!this.rules.some((r) => r.id === id)) fail("not_found", `Rule ${id} not found`);
        this.rules = this.rules.filter((r) => r.id !== id);
        return null;
      }
      case "test_rule": {
        const { rule, sample } = a as { rule: Rule; sample: Message };
        const errs = validateRule(rule);
        if (errs.regex) fail("invalid", `Invalid regex: ${errs.regex}`);
        const chat = this.chats.find((c) => c.jid === sample.chat_jid);
        return evaluateRule(rule, sample, chat?.name ?? sample.chat_jid, new Date(this.nowSec() * 1000));
      }
      case "automation_log": return this.autoLog.slice(0, (a as { limit: number }).limit);
      case "set_automation_enabled": {
        this.settings = { ...this.settings, automation_enabled: (a as { enabled: boolean }).enabled };
        return this.settings;
      }
      case "query_logs": return this.queryLogs((a as { filter: LogFilter }).filter);
      case "list_issues": return this.listIssues();
      case "resolve_issue": {
        const issue = this.issues.find((i) => i.id === (a as { id: number }).id);
        if (!issue) fail("not_found", "Issue not found");
        issue.status = "resolved";
        return null;
      }
      case "clear_resolved_issues": {
        const before = this.issues.length;
        this.issues = this.issues.filter((i) => i.status !== "resolved");
        return before - this.issues.length;
      }
      case "export_issue_bundle": {
        const ids = (a as { ids: number[] | null }).ids;
        const n = ids ? ids.length : this.issues.length;
        const stamp = new Date(this.nowSec() * 1000).toISOString().slice(0, 19).replace(/[:T]/g, "-");
        return `${EXPORTS}/wa-desk-report-${stamp}-${n}issues.zip`;
      }
      case "get_settings": return this.settings;
      case "set_settings": {
        const s = (a as { settings: Settings }).settings;
        if (s.backfill_days < 1 || s.backfill_days > 3650) fail("invalid", "Backfill days must be between 1 and 3650");
        this.settings = s;
        return s;
      }
      default:
        return this.plugin(cmd, args);
    }
  }

  // ------------------------------------------------------------ chats / messages

  chat(jid: string): MockChat {
    const c = this.chats.find((x) => x.jid === jid || x.jid === phoneJid(jid.replace(/^\+/, "")));
    if (!c) fail("not_found", "Chat not found", jid);
    return c;
  }

  message(chatJid: string, id: string): Message {
    const m = this.chat(chatJid).messages.find((x) => x.id === id);
    if (!m) fail("not_found", "Message not found", id);
    return m;
  }

  summary(c: MockChat): ChatSummary {
    const last = c.messages[c.messages.length - 1];
    return {
      jid: c.jid,
      name: c.name,
      is_group: c.is_group,
      last_message_ts: last?.ts ?? null,
      last_preview: last ? previewOf(last) : null,
      last_sender_name: last ? (last.is_from_me ? "Me" : last.sender_name) : null,
      last_is_from_me: last?.is_from_me ?? false,
      unread: c.unread,
    };
  }

  listChats(q: ChatQuery): Page<ChatSummary> {
    const f = q.filter?.trim().toLowerCase() ?? "";
    let list = this.chats
      .filter((c) => (q.kind === "all" ? true : q.kind === "group" ? c.is_group : !c.is_group))
      .filter((c) => (q.unread_only ? c.unread : true))
      .filter((c) => !f || c.name.toLowerCase().includes(f) || c.jid.includes(f))
      .map((c) => this.summary(c));
    list = list.sort((a, b) => (b.last_message_ts ?? 0) - (a.last_message_ts ?? 0));
    return paginate(list, q.cursor, clampLimit(q.limit, 100));
  }

  getMessages(q: MessagesQuery): Page<Message> {
    const c = this.chat(q.chat_jid);
    const limit = clampLimit(q.limit, 80);
    let end = c.messages.length;
    if (q.before) {
      const i = c.messages.findIndex((m) => m.id === q.before);
      if (i < 0) fail("invalid", "Unknown cursor");
      end = i;
    }
    const start = Math.max(0, end - limit);
    const items = c.messages.slice(start, end);
    return { items, next_cursor: start > 0 && items[0] ? items[0].id : null };
  }

  context(chatJid: string, id: string, radius: number): Message[] {
    const c = this.chat(chatJid);
    const i = c.messages.findIndex((m) => m.id === id);
    if (i < 0) fail("not_found", "Message not found (it may be older than the archive)");
    const r = Math.max(0, Math.min(250, radius));
    return c.messages.slice(Math.max(0, i - r), i + r + 1);
  }

  // ------------------------------------------------------------ search

  search(q: SearchQuery): SearchResults {
    const t0 = performance.now();
    const terms = q.text.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const hits: SearchHit[] = [];
    const chatSet = new Set(q.chat_jids);
    const senders = q.senders.map((s) => s.replace(/^\+/, "").toLowerCase());
    for (const c of this.chats) {
      if (chatSet.size && !chatSet.has(c.jid)) continue;
      for (const m of c.messages) {
        if (m.deleted) continue;
        if (q.from_me !== null && m.is_from_me !== q.from_me) continue;
        if (q.after !== null && m.ts < q.after) continue;
        if (q.before !== null && m.ts > q.before) continue;
        if (q.media_kinds.length && !(m.media && q.media_kinds.includes(m.media.kind))) continue;
        if (senders.length) {
          const sj = (m.sender_jid ?? "").toLowerCase();
          const sn = m.sender_name.toLowerCase();
          if (!senders.some((s) => sj === s || sj.startsWith(`${s}@`) || sn.includes(s))) continue;
        }
        const target = m.content || m.media?.filename || "";
        const hl = terms.length ? matchTerms(target, terms) : [];
        if (terms.length && !hl) continue;
        hits.push({ message: m, chat_name: c.name, highlights: hl ?? [] });
      }
    }
    hits.sort((a, b) => b.message.ts - a.message.ts);
    const total = Math.min(hits.length, 10_000);
    const offset = q.offset ?? 0;
    const limit = clampLimit(q.limit, 50);
    const total_ms = Math.max(1, Math.round(performance.now() - t0));
    return {
      hits: hits.slice(offset, offset + limit),
      total,
      took_ms: total_ms,
      index_building: this.indexed < this.totalMessages(),
    };
  }

  // ------------------------------------------------------------ media

  listMedia(q: MediaQuery): Page<MediaItem> {
    const text = q.text?.trim().toLowerCase() ?? "";
    const items: MediaItem[] = [];
    for (const c of this.chats) {
      if (q.chat_jid && c.jid !== q.chat_jid) continue;
      for (const m of c.messages) {
        if (!m.media || m.deleted) continue;
        if (q.kinds.length && !q.kinds.includes(m.media.kind)) continue;
        if (q.available === true && m.media.source === "missing") continue;
        if (q.available === false && m.media.source !== "missing") continue;
        if (q.after !== null && m.ts < q.after) continue;
        if (q.before !== null && m.ts > q.before) continue;
        if (text && !(`${m.media.filename ?? ""} ${m.content}`.toLowerCase().includes(text))) continue;
        items.push({
          message_id: m.id,
          chat_jid: c.jid,
          chat_name: c.name,
          sender_name: m.sender_name,
          is_from_me: m.is_from_me,
          ts: m.ts,
          caption: m.content,
          media: m.media,
        });
      }
    }
    items.sort((a, b) => b.ts - a.ts);
    return paginate(items, q.cursor, clampLimit(q.limit, 200));
  }

  async ensureMedia(chatJid: string, id: string): Promise<MediaRef> {
    const m = this.message(chatJid, id);
    if (!m.media) fail("not_found", "This message has no media");
    if (m.media.source !== "missing") return m.media;
    if (!this.conn.connected) fail("bridge_down", "WhatsApp bridge is offline", this.conn.message);
    await new Promise((r) => setTimeout(r, 900 / this.opts.speed));
    if ((m.media.filename ?? "").startsWith("corrupt")) {
      this.recordIssue("Media download failed", "external", "bridge.media", "decrypt: media key mismatch");
      fail("external", "Download failed: the phone no longer has this file", "decrypt: media key mismatch");
    }
    const hue = (m.ts % 360);
    m.media = { ...buildMedia(m.media.kind, m.media.filename ?? "file", m.media.size ?? 0, hue, true), source: "store" };
    this.emit("db:changed", { chat_jids: [chatJid] });
    return m.media;
  }

  async previewDoc(chatJid: string, id: string): Promise<PreviewRef> {
    const m = this.message(chatJid, id);
    const media = m.media;
    if (!media) fail("not_found", "This message has no media");
    if (!media.url) fail("not_found", "Download the file first");
    const ext = extOf(media.filename);
    if (media.kind === "image" || media.kind === "sticker") return { kind: "image", url: media.url, original_url: media.url, note: null };
    if (media.kind === "video") return { kind: "video", url: media.url, original_url: media.url, note: null };
    if (media.kind === "audio") return { kind: "audio", url: media.url, original_url: media.url, note: null };
    if (ext === "pdf") return { kind: "pdf", url: media.url, original_url: media.url, note: null };
    if (ext === "txt") return { kind: "text", url: media.url, original_url: media.url, note: null };
    if (docBucket(media.filename) === "office") {
      await new Promise((r) => setTimeout(r, 700 / this.opts.speed));
      return { kind: "pdf", url: MOCK_PDF_URL, original_url: media.url, note: "Converted to PDF with LibreOffice (cached)" };
    }
    return { kind: "unsupported", url: null, original_url: media.url, note: `No preview for .${ext || "?"} files` };
  }

  // ------------------------------------------------------------ sending

  async send(req: SendReq): Promise<{ ok: boolean; message: string }> {
    if (!this.conn.connected) fail("bridge_down", "WhatsApp bridge is offline", `state: ${this.conn.state}`);
    if (!req.text?.trim() && !req.attachment) fail("invalid", "Nothing to send");
    const chat = this.chat(req.recipient);
    if (req.attachment && !req.attachment.startsWith("/")) fail("invalid", "Attachment path must be absolute");
    await new Promise((r) => setTimeout(r, 250 / this.opts.speed));
    let quoted = null;
    if (req.reply_to) {
      const q = chat.messages.find((m) => m.id === req.reply_to);
      if (!q) fail("invalid", "Reply target is not in this chat");
      quoted = { id: q.id, sender_name: q.is_from_me ? "You" : q.sender_name, preview: previewOf(q) };
    }
    let media: MediaRef | null = null;
    if (req.attachment) {
      const name = baseName(req.attachment);
      const ext = extOf(name);
      const kind = ["jpg", "jpeg", "png", "webp", "gif"].includes(ext) ? "image" : ["mp4", "webm", "mov"].includes(ext) ? "video" : ["ogg", "opus", "mp3", "m4a"].includes(ext) ? "audio" : "document";
      media = buildMedia(kind, name, 245_760, 150, true);
    }
    const msg: Message = {
      id: `3EB0MOCK${(this.nextId++).toString(16).toUpperCase()}`,
      chat_jid: chat.jid,
      sender_jid: null,
      sender_name: "Me",
      is_from_me: true,
      ts: this.nowSec(),
      content: req.text ?? "",
      media,
      quoted,
      reactions: [],
      deleted: false,
    };
    this.push(chat, msg);
    if (!this.opts.quiet && !chat.is_group) {
      this.later(2500, () => this.incoming(chat.jid, "👍 got it"));
    }
    return { ok: true, message: "Sent" };
  }

  private push(chat: MockChat, msg: Message): void {
    chat.messages.push(msg);
    this.rowid++;
    this.indexed++;
    this.emit("db:new-messages", { chat_jids: [chat.jid], max_rowid: this.rowid, count: 1 });
  }

  /** Simulate an incoming message (exposed for e2e via window.__waMock). */
  incoming(chatJid: string, text: string): Message {
    const chat = this.chat(chatJid);
    const other = [...chat.messages].reverse().find((m) => !m.is_from_me);
    const msg: Message = {
      id: `3EB0IN${(this.nextId++).toString(16).toUpperCase()}`,
      chat_jid: chat.jid,
      sender_jid: other?.sender_jid ?? null,
      sender_name: other?.sender_name ?? chat.name,
      is_from_me: false,
      ts: this.nowSec(),
      content: text,
      media: null,
      quoted: null,
      reactions: [],
      deleted: false,
    };
    chat.unread = true;
    this.push(chat, msg);
    this.runAutomations(chat, msg);
    return msg;
  }

  private pickIncomingText(): string {
    const pool = ["Are you free later?", "कहाँ हो? 🙂", "Sent you the file", "😂😂", "Urgent: call me back", "See you tomorrow!"];
    return pool[Math.floor(this.rnd() * pool.length)] ?? "Hi";
  }

  private runAutomations(chat: MockChat, msg: Message): void {
    if (!this.settings.automation_enabled) return;
    for (const rule of this.rules) {
      if (!rule.enabled) continue;
      const ev = evaluateRule(rule, msg, chat.name, new Date(this.nowSec() * 1000));
      if (!ev.matched) continue;
      rule.fired_count++;
      rule.last_fired_at = this.nowSec();
      const e: AutomationEvent = {
        rule_id: rule.id ?? 0,
        rule_name: rule.name,
        chat_jid: chat.jid,
        message_id: msg.id,
        dry_run: rule.dry_run,
        outcomes: ev.planned,
        at: this.nowSec(),
      };
      this.autoLog.unshift(e);
      this.emit("automation:fired", e);
    }
  }

  react(req: ReactReq): null {
    if (!this.conn.connected) fail("bridge_down", "WhatsApp bridge is offline");
    const m = this.message(req.chat_jid, req.message_id);
    if (m.deleted) fail("invalid", "Can't react to a deleted message");
    m.reactions = m.reactions.filter((r) => !r.is_from_me);
    if (req.emoji) m.reactions.push({ emoji: req.emoji, sender_name: "Me", is_from_me: true });
    this.emit("db:changed", { chat_jids: [req.chat_jid] });
    return null;
  }

  // ------------------------------------------------------------ bridge

  private connState(state: BridgeState, message: string): ConnectionState {
    const pairing = state === "pairing";
    return {
      state,
      connected: state === "connected",
      paired: state !== "pairing" && state !== "logged_out",
      message,
      qr_svg: pairing ? qrSvg(this.nextId++) : null,
      qr_expires_at: pairing ? this.nowSec() + 20 : null,
      last_error: state === "error" || state === "unreachable" ? "connection refused (os error 111)" : (this.conn?.last_error ?? null),
      checked_at: this.nowSec(),
    };
  }

  setBridge(state: BridgeState, message?: string): void {
    const msgs: Record<BridgeState, string> = {
      connected: "Connected as +91 98450 12345",
      connecting: "Connecting to the WhatsApp bridge…",
      pairing: "Scan the QR code with your phone to link this device",
      disconnected: "Disconnected from WhatsApp",
      logged_out: "Logged out — link the device again",
      outdated: "Bridge client is outdated; update whatsmeow",
      error: "Bridge error",
      unreachable: "Bridge at 127.0.0.1:8080 is not responding",
    };
    this.conn = this.connState(state, message ?? msgs[state]);
    this.emit("bridge:state", this.conn);
  }

  reconnect(): ConnectionState {
    this.setBridge("connecting");
    // Cycle through pairing (with a QR) to connected so every state is visible.
    this.later(1200, () => this.setBridge("pairing"));
    this.later(7000, () => this.setBridge("connected"));
    return this.conn;
  }

  // ------------------------------------------------------------ jobs

  private jobLabel(spec: JobSpec): string {
    switch (spec.type) {
      case "backfill": return `Backfill media (${spec.days} days)`;
      case "phone_sync": return "Phone sync (USB)";
      case "link_phone_media": return "Link phone media";
      case "reindex": return "Rebuild search index";
      case "thumbnails": return "Generate thumbnails";
      case "convert": {
        const name = spec.input.type === "file" ? baseName(spec.input.path) : "message media";
        return `Convert ${name} (${spec.kind.type.replace(/_/g, " ")})`;
      }
    }
  }

  startJob(spec: JobSpec): JobRecord {
    if (spec.type === "backfill" && (spec.days < 1 || spec.workers < 1 || spec.workers > 16)) {
      fail("invalid", "Days must be ≥ 1 and workers between 1 and 16");
    }
    if (this.jobs.some((j) => j.status === "running" && j.spec.type === spec.type && spec.type !== "convert")) {
      fail("busy", "A job of this kind is already running");
    }
    const id = Math.max(0, ...this.jobs.map((j) => j.id)) + 1;
    const missing = this.chats.flatMap((c) => c.messages).filter((m) => m.media?.source === "missing");
    const total =
      spec.type === "backfill" ? missing.filter((m) => !spec.kinds.length || spec.kinds.includes(m.media?.kind ?? "image")).length
      : spec.type === "reindex" ? this.totalMessages()
      : spec.type === "thumbnails" ? 180
      : spec.type === "link_phone_media" ? 520
      : spec.type === "convert" ? 100
      : 0;
    const job: JobRecord = {
      id, spec, label: this.jobLabel(spec), status: "queued", done: 0, total, phase: "queued",
      message: null, error: null, outputs: [], created_at: this.nowSec(), finished_at: null,
    };
    this.jobs.unshift(job);
    let ticks = 0;
    const t = setInterval(() => {
      ticks++;
      if (job.status === "queued") {
        job.status = "running";
        job.phase = spec.type === "phone_sync" ? "connecting" : spec.type === "convert" ? "converting" : "scanning";
      }
      if (spec.type === "phone_sync" && ticks >= 4) {
        this.finishJob(job, "failed", "No phone connected over USB (enable File transfer)");
        this.recordIssue("Phone sync failed: no USB device", "external", "jobs.phone_sync", "No phone connected over USB (enable File transfer)");
        return;
      }
      const step = Math.max(1, Math.ceil(total / 25));
      job.done = Math.min(total, job.done + step);
      if (job.phase === "scanning" && job.done > total * 0.1) job.phase = spec.type === "reindex" ? "indexing" : spec.type === "backfill" ? "downloading" : "processing";
      job.message = `${job.done} / ${total}`;
      this.emit("job:progress", { id: job.id, done: job.done, total: job.total, phase: job.phase, message: job.message });
      if (job.done >= total && spec.type !== "phone_sync") {
        if (spec.type === "convert") job.outputs = convertOutputs(spec);
        if (spec.type === "backfill") {
          for (const m of missing.slice(0, total)) {
            if (m.media && !(m.media.filename ?? "").startsWith("corrupt")) {
              m.media = buildMedia(m.media.kind, m.media.filename ?? "file", m.media.size ?? 0, m.ts % 360, true);
            }
          }
          this.emit("db:changed", { chat_jids: this.chats.map((c) => c.jid) });
        }
        job.message = spec.type === "backfill" ? `${total} files downloaded` : spec.type === "link_phone_media" ? "Linked 37 files from phone folders" : "Done";
        this.finishJob(job, "succeeded", null);
      }
    }, 350 / this.opts.speed);
    this.jobTimers.set(id, t);
    return job;
  }

  private finishJob(job: JobRecord, status: JobRecord["status"], error: string | null): void {
    const t = this.jobTimers.get(job.id);
    if (t) clearInterval(t);
    this.jobTimers.delete(job.id);
    job.status = status;
    job.error = error;
    job.phase = status === "succeeded" ? "done" : job.phase;
    job.finished_at = this.nowSec();
    this.emit("job:finished", job);
  }

  cancelJob(id: number): null {
    const job = this.jobs.find((j) => j.id === id);
    if (!job) fail("not_found", "Job not found");
    if (job.status !== "running" && job.status !== "queued") fail("invalid", "Job is not running");
    job.message = "Cancelled by user";
    this.finishJob(job, "cancelled", null);
    return null;
  }

  // ------------------------------------------------------------ rules

  saveRule(rule: Rule): Rule {
    const errs = validateRule(rule);
    if (hasErrors(errs)) {
      const first = Object.values(errs)[0] ?? "Invalid rule";
      fail("invalid", errs.regex ? `Invalid regex: ${errs.regex}` : first);
    }
    if (rule.id === null) {
      const saved = { ...rule, id: Math.max(0, ...this.rules.map((r) => r.id ?? 0)) + 1 };
      this.rules.push(saved);
      return saved;
    }
    const i = this.rules.findIndex((r) => r.id === rule.id);
    if (i < 0) fail("not_found", "Rule not found");
    const prev = this.rules[i] as Rule;
    const saved = { ...rule, fired_count: prev.fired_count, last_fired_at: prev.last_fired_at };
    this.rules[i] = saved;
    return saved;
  }

  // ------------------------------------------------------------ logs / issues

  addLog(line: LogLine): void {
    this.logs.unshift(line);
    if (this.logs.length > 5000) this.logs.length = 5000;
  }

  queryLogs(f: LogFilter): LogLine[] {
    const min = LEVELS.indexOf(f.level ?? "info");
    const contains = f.contains?.toLowerCase() ?? "";
    const target = f.target?.toLowerCase() ?? "";
    const limit = Math.min(5000, f.limit ?? 500);
    const out: LogLine[] = [];
    for (const l of this.logs) {
      if (LEVELS.indexOf(l.level) < min) continue;
      if (f.since !== null && l.ts < f.since) continue;
      if (target && !l.target.toLowerCase().startsWith(target)) continue;
      if (contains && !`${l.message} ${l.fields}`.toLowerCase().includes(contains)) continue;
      out.push(l);
      if (out.length >= limit) break;
    }
    return out;
  }

  listIssues(): Issue[] {
    return [...this.issues].sort((a, b) =>
      a.status !== b.status ? (a.status === "open" ? -1 : 1) : b.last_seen - a.last_seen,
    );
  }

  recordIssue(title: string, kind: Issue["kind"], source: string, detail: string | null): void {
    const fp = `${kind}:${source}:${title}`;
    let issue = this.issues.find((i) => i.fingerprint === fp || (i.title === title && i.source === source));
    const now = this.nowSec();
    if (issue) {
      issue.count++;
      issue.last_seen = now;
      issue.last_detail = detail;
      issue.status = "open";
    } else {
      issue = {
        id: Math.max(0, ...this.issues.map((i) => i.id)) + 1, fingerprint: fp, title, kind, source, count: 1,
        first_seen: now, last_seen: now, last_detail: detail, status: "open",
      };
      this.issues.push(issue);
    }
    this.addLog({ ts: now, level: "error", target: source.replace(".", "::"), message: title, fields: detail ? JSON.stringify({ err: detail }) : "" });
    this.emit("issue:new", issue);
  }

  // ------------------------------------------------------------ overview

  overview(): Overview {
    let total = 0;
    let local = 0;
    let oldest = Infinity;
    let newest = 0;
    for (const c of this.chats) {
      for (const m of c.messages) {
        if (m.media) {
          total++;
          if (m.media.source !== "missing") local++;
        }
        if (m.ts < oldest) oldest = m.ts;
        if (m.ts > newest) newest = m.ts;
      }
    }
    return {
      chats: this.chats.length,
      messages: this.totalMessages(),
      media_total: total,
      media_local: local,
      oldest_ts: Number.isFinite(oldest) ? oldest : null,
      newest_ts: newest || null,
      indexed: this.indexed,
      app_version: "0.1.0-mock",
    };
  }

  // ------------------------------------------------------------ plugins (dialog/opener/notification)

  private plugin(cmd: string, args: Record<string, unknown>): unknown {
    if (!cmd.startsWith("plugin:")) fail("not_found", `Unknown command ${cmd}`);
    this.pluginCalls.push({ cmd, args });
    const opts = (args.options ?? {}) as { directory?: boolean; multiple?: boolean; defaultPath?: string; filters?: { extensions: string[] }[] };
    switch (cmd) {
      case "plugin:dialog|open": {
        if (opts.directory) return opts.multiple ? ["/home/user/Pictures/Camera Exports"] : "/home/user/Pictures/Camera Exports";
        const exts = opts.filters?.flatMap((f) => f.extensions) ?? [];
        const path = exts.includes("docx") ? "/home/user/Documents/Quarterly report.docx"
          : exts.includes("jpg") ? "/home/user/Pictures/holiday.jpg"
          : "/home/user/Documents/Quarterly report.pdf";
        return opts.multiple ? [path] : path;
      }
      case "plugin:dialog|save":
        return `/home/user/Downloads/${baseName(opts.defaultPath ?? "file")}`;
      case "plugin:dialog|message":
        return "Ok";
      case "plugin:notification|is_permission_granted":
        return true;
      case "plugin:notification|request_permission":
        return "granted";
      default:
        return null;
    }
  }
}

// ------------------------------------------------------------------- helpers

function previewOf(m: Message): string {
  if (m.deleted) return "This message was deleted";
  if (m.media) {
    const label = m.media.kind === "document" ? m.media.filename ?? "Document" : m.content || m.media.kind[0]?.toUpperCase() + m.media.kind.slice(1);
    return `${mediaEmoji(m.media.kind)} ${label}`;
  }
  return m.content.slice(0, 120);
}

function clampLimit(limit: number | null, dflt: number): number {
  return Math.max(1, Math.min(500, limit ?? dflt));
}

function paginate<T>(items: T[], cursor: string | null, limit: number): Page<T> {
  const start = cursor ? Number(cursor) || 0 : 0;
  const page = items.slice(start, start + limit);
  return { items: page, next_cursor: start + limit < items.length ? String(start + limit) : null };
}

/** All occurrences of every term (case-insensitive) as UTF-8 byte ranges; null if any term is absent. */
export function matchTerms(text: string, terms: string[]): [number, number][] | null {
  const lower = text.toLowerCase();
  // toLowerCase can change length for a few scripts; fall back to exact positions only when lengths match.
  if (lower.length !== text.length) return null;
  const ranges: [number, number][] = [];
  for (const t of terms) {
    let i = lower.indexOf(t);
    if (i < 0) return null;
    while (i >= 0) {
      ranges.push([indexToByteOffset(text, i), indexToByteOffset(text, i + t.length)]);
      i = lower.indexOf(t, i + t.length);
    }
  }
  return ranges.sort((a, b) => a[0] - b[0]);
}

function convertOutputs(spec: Extract<JobSpec, { type: "convert" }>): string[] {
  const src = spec.input.type === "file" ? baseName(spec.input.path) : "media";
  const stem = src.replace(/\.[^.]+$/, "");
  const dir = spec.output_dir ?? EXPORTS;
  const out = (name: string, url: string) => `${url}${url.includes("?") ? "&" : "?"}path=${encodeURIComponent(`${dir}/${name}`)}`;
  switch (spec.kind.type) {
    case "office_to_pdf":
    case "pdf_compress":
      return [out(`${stem}${spec.kind.type === "pdf_compress" ? "-small" : ""}.pdf`, MOCK_PDF_URL)];
    case "image_compress":
      return [out(`${stem}.${spec.kind.format === "jpeg" ? "jpg" : spec.kind.format}`, "/mock/photo.svg")];
    case "video_compress":
      return [out(`${stem}-${spec.kind.max_height}p.mp4`, "/mock/sample.webm")];
    case "audio_to_opus":
      return [out(`${stem}.opus`, "/mock/sample.ogg")];
    case "pdf_to_images":
      return [1, 2, 3].map((p) => out(`${stem}-page${p}.png`, "/mock/photo.svg"));
  }
}
