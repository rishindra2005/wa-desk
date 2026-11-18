<script lang="ts">
  import { tick, untrack } from "svelte";
  import {
    ArrowDown, MessageSquareDashed, MoreVertical, X, Copy, Forward, Star, FileDown, Download, Info, Search, ListChecks,
    Reply, SmilePlus, ExternalLink, Save, Wand2, CornerUpLeft, RefreshCw, StarOff, Phone, Video, Image as ImageIcon,
  } from "@lucide/svelte";
  import * as api from "../../api";
  import type { ChatSummary } from "../../bindings/ChatSummary";
  import type { Message } from "../../bindings/Message";
  import type { CallRecord } from "../../bindings/CallRecord";
  import type { MediaRef } from "../../bindings/MediaRef";
  import type { SendReq } from "../../bindings/SendReq";
  import Avatar from "../../components/Avatar.svelte";
  import VirtualList from "../../components/VirtualList.svelte";
  import type { VirtualListApi } from "../../components/virtual";
  import Skeleton from "../../components/Skeleton.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import Lightbox, { type LightboxItem } from "../../components/Lightbox.svelte";
  import Modal from "../../components/Modal.svelte";
  import DocPreview from "../../components/DocPreview.svelte";
  import ConvertMenu from "../../components/ConvertMenu.svelte";
  import StatusTicks, { TICK_TEXT, type TickState } from "../../components/StatusTicks.svelte";
  import MessageBubble from "./MessageBubble.svelte";
  import Composer from "./Composer.svelte";
  import { chatLabel, dateTime, dayKey, dayLabel, displayName, formatPhone } from "../../utils/format";
  import { toasts } from "../../state/toasts.svelte";
  import { app } from "../../state/app.svelte";
  import { ctxMenu } from "../../state/menu.svelte";
  import { chatState } from "../../state/chatstate.svelte";
  import { overlayCount } from "../../components/overlay";
  import { debounce, copyText } from "../../utils/misc";
  import { applyClick, inOrder } from "../../utils/selection";
  import type { MenuItem } from "../../utils/menu";
  import { baseName } from "../../utils/text";
  import { CALL_FROM_PHONE_HINT, direction, pillText, upsertCall } from "../../utils/calls";
  import { copyImageToClipboard, downloadMedia, openExternal, saveMedia } from "../../media";
  import { pickFolder } from "../../platform";

  interface Props {
    chat: ChatSummary;
    /** Message to scroll to (search results, quoted replies). */
    jumpId?: string | null;
    jumpNonce?: number;
    /** The chat was unread when opened (drives the unread divider). */
    wasUnread?: boolean;
  }
  let { chat, jumpId = null, jumpNonce = 0, wasUnread = false }: Props = $props();

  const PAGE = 80;
  type Row =
    | { kind: "day"; key: string; label: string }
    | { kind: "unread"; key: string; label: string }
    | { kind: "call"; key: string; c: CallRecord }
    | { kind: "msg"; key: string; m: Message; first: boolean; showName: boolean; local: boolean };

  interface Pending {
    id: string;
    chat: string;
    req: SendReq;
    ts: number;
    state: "pending" | "failed";
    awaiting: boolean;
    quote: Message | null;
  }

  let messages = $state<Message[]>([]);
  /** The chat's call log, merged into the thread by time. */
  let callRecs = $state<CallRecord[]>([]);
  let olderCursor = $state<string | null>(null);
  /** Set while showing a context window: cursor to page toward the newest message. */
  let newerCursor = $state<string | null>(null);
  /** True when showing a context window that doesn't reach the newest message. */
  let detached = $state(false);
  let loading = $state(true);
  let loadingOlder = $state(false);
  let loadingNewer = $state(false);
  let error = $state<unknown>(null);
  let highlightId = $state<string | null>(null);
  let replyTo = $state<Message | null>(null);
  let atBottom = $state(true);
  let unseen = $state(0);
  let unreadFromId = $state<string | null>(null);
  let list: VirtualListApi | undefined = $state();
  let lightbox = $state<{ items: LightboxItem[]; index: number } | null>(null);
  let previewMsg = $state<Message | null>(null);
  let convertMsg = $state<Message | null>(null);
  let infoMsg = $state<Message | null>(null);
  let pending = $state<Pending[]>([]);
  let loadSeq = 0;
  let selecting = $state(false);
  let selected = $state<Set<string>>(new Set());
  let anchor = $state<string | null>(null);
  let reactReq = $state<{ id: string; nonce: number } | null>(null);
  let moreBtn: HTMLButtonElement | undefined = $state();
  let section: HTMLElement | undefined = $state();

  const title = $derived(chatLabel(chat));
  const isGroupChat = $derived(chat.chat_type === "group" || chat.chat_type === "community");

  function localMessage(p: Pending): Message {
    const att = p.req.attachment ? `📎 ${baseName(p.req.attachment)}` : "";
    return {
      id: p.id, chat_jid: p.chat, sender_jid: null, sender_name: "Me", is_from_me: true, ts: p.ts,
      content: [att, p.req.text ?? ""].filter(Boolean).join("\n"), media: null,
      quoted: p.quote ? { id: p.quote.id, sender_name: p.quote.is_from_me ? "You" : p.quote.sender_name, preview: p.quote.content || null } : null,
      reactions: [], deleted: false, status: null, starred: false,
    };
  }

  const allMessages = $derived.by(() => {
    if (detached) return messages;
    const mine = pending.filter((p) => p.chat === chat.jid);
    return mine.length ? [...messages, ...mine.map(localMessage)] : messages;
  });
  const pendingById = $derived(new Map(pending.map((p) => [p.id, p])));
  const orderedIds = $derived(messages.map((m) => m.id));

  /** Calls inside the loaded time window (older calls wait for older messages to load). */
  const visibleCalls = $derived.by(() => {
    if (!callRecs.length) return [];
    const first = messages[0]?.ts;
    const last = messages[messages.length - 1]?.ts;
    const lo = olderCursor !== null && first !== undefined ? first : -Infinity;
    const hi = detached && last !== undefined ? last : Infinity;
    return callRecs.filter((c) => c.ts >= lo && c.ts <= hi).sort((a, b) => a.ts - b.ts || (a.call_id < b.call_id ? -1 : 1));
  });

  const rows = $derived.by(() => {
    const out: Row[] = [];
    let prev: Message | null = null;
    let lastDay = "";
    const now = Date.now();
    let unreadShown = false;
    let idx = -1;
    const cl = visibleCalls;
    let ci = 0;
    const day = (ts: number) => {
      const dk = dayKey(ts);
      const isNew = dk !== lastDay;
      if (isNew) out.push({ kind: "day", key: `d:${dk}`, label: dayLabel(ts, now) });
      lastDay = dk;
      return isNew;
    };
    const flushCalls = (until: number) => {
      while (ci < cl.length && (cl[ci] as CallRecord).ts <= until) {
        const c = cl[ci++] as CallRecord;
        day(c.ts);
        out.push({ kind: "call", key: `call:${c.call_id}:${c.chat_jid}`, c });
        prev = null;
      }
    };
    for (const m of allMessages) {
      idx++;
      flushCalls(m.ts - 1);
      const newDay = day(m.ts);
      if (!unreadShown && unreadFromId === m.id) {
        unreadShown = true;
        const n = allMessages.slice(idx).filter((x) => !x.is_from_me).length;
        out.push({ kind: "unread", key: "unread", label: `${n} unread message${n === 1 ? "" : "s"}` });
      }
      const sameSender =
        !!prev && !newDay && prev.is_from_me === m.is_from_me && prev.sender_jid === m.sender_jid && m.ts - prev.ts < 300 && !(unreadShown && unreadFromId === m.id);
      out.push({ kind: "msg", key: m.id, m, first: !sameSender, showName: !sameSender, local: pendingById.has(m.id) });
      prev = m;
    }
    flushCalls(Infinity);
    return out;
  });

  function estimate(r: Row): number {
    if (r.kind === "day" || r.kind === "unread" || r.kind === "call") return 40;
    const m = r.m;
    let h = r.first ? 44 : 38;
    if (m.media) h += m.media.kind === "audio" ? 44 : m.media.kind === "document" ? 90 : m.media.kind === "sticker" ? 130 : 230;
    if (m.quoted) h += 48;
    if (m.reactions.length) h += 22;
    h += Math.floor(m.content.length / 60) * 19;
    return h;
  }

  function merge(base: Message[], incoming: Message[]): Message[] {
    const byId = new Map(base.map((m) => [m.id, m]));
    for (const m of incoming) byId.set(m.id, m);
    return [...byId.values()].sort((a, b) => a.ts - b.ts || (a.id < b.id ? -1 : 1));
  }

  function markUnread(items: Message[]) {
    unreadFromId = null;
    if (!wasUnread) return;
    let i = items.length;
    while (i > 0 && !items[i - 1]?.is_from_me && items.length - i < 50) i--;
    if (i < items.length) unreadFromId = items[i]?.id ?? null;
  }

  async function loadLatest(scrollBottom = true) {
    const my = ++loadSeq;
    loading = messages.length === 0;
    error = null;
    try {
      const page = await api.getMessages({ chat_jid: chat.jid, before: null, after: null, limit: PAGE });
      if (my !== loadSeq) return;
      messages = page.items;
      olderCursor = page.next_cursor;
      newerCursor = null;
      detached = false;
      unseen = 0;
      if (scrollBottom && unreadFromId === null) markUnread(page.items);
      if (scrollBottom) {
        await tick();
        list?.scrollToBottom();
      }
    } catch (e) {
      if (my === loadSeq) error = e;
    } finally {
      if (my === loadSeq) loading = false;
    }
  }

  async function loadOlder() {
    if (loadingOlder || !olderCursor || loading) return;
    loadingOlder = true;
    const jid = chat.jid;
    try {
      const page = await api.getMessages({ chat_jid: jid, before: olderCursor, after: null, limit: PAGE });
      if (jid !== chat.jid) return;
      messages = merge(messages, page.items);
      olderCursor = page.next_cursor;
    } catch (e) {
      toasts.error(e, "Couldn't load older messages");
    } finally {
      loadingOlder = false;
    }
  }

  /** Scrolling down in a detached window pages toward the newest message with `after`. */
  async function loadNewer() {
    if (!detached || loadingNewer || !newerCursor || loading) return;
    loadingNewer = true;
    const jid = chat.jid;
    const from = newerCursor;
    try {
      const page = await api.getMessages({ chat_jid: jid, before: null, after: from, limit: PAGE });
      if (jid !== chat.jid || !detached) return;
      messages = merge(messages, page.items);
      if (page.next_cursor === null || page.items.length === 0) {
        newerCursor = null;
        detached = false; // reached the newest message
      } else newerCursor = page.next_cursor;
    } catch (e) {
      toasts.error(e, "Couldn't load newer messages");
    } finally {
      loadingNewer = false;
    }
  }

  async function jumpTo(id: string) {
    if (messages.some((m) => m.id === id)) {
      await tick();
      list?.scrollToKey(id, "center");
      flash(id);
      return;
    }
    try {
      const ctx = await api.getMessageContext(chat.jid, id, 40);
      loadSeq++;
      messages = ctx;
      olderCursor = ctx[0]?.id ?? null;
      newerCursor = ctx[ctx.length - 1]?.id ?? null;
      detached = true;
      loading = false;
      await tick();
      list?.scrollToKey(id, "center");
      flash(id);
    } catch (e) {
      toasts.error(e, "Couldn't find that message");
    }
  }

  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  function flash(id: string) {
    highlightId = id;
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => (highlightId = null), 2600);
  }

  // Load when the chat changes (and jump if requested).
  let currentJid = "";
  $effect(() => {
    const jid = chat.jid;
    untrack(() => {
      if (jid === currentJid) return;
      currentJid = jid;
      messages = [];
      callRecs = [];
      void loadCalls();
      replyTo = null;
      lightbox = null;
      unreadFromId = null;
      cancelSelection();
      loading = true;
      if (jumpId) {
        const target = jumpId;
        void loadLatest(false).then(() => jumpTo(target));
      } else void loadLatest(true);
    });
  });

  // Repeated jumps within the same chat.
  let lastNonce = 0;
  $effect(() => {
    const n = jumpNonce;
    const id = jumpId;
    untrack(() => {
      if (n && n !== lastNonce) {
        const first = lastNonce === 0 && currentJid === chat.jid && loading;
        lastNonce = n;
        if (id && !first) void jumpTo(id);
      }
    });
  });

  // Drop optimistic bubbles once the real message is in the list.
  $effect(() => {
    const msgs = messages;
    untrack(() => {
      const keep = pending.filter((p) => {
        if (!p.awaiting) return true;
        return !msgs.some((m) => m.is_from_me && m.ts >= p.ts - 10 && m.content.startsWith(p.req.text ?? "") && (!p.req.attachment || !!m.media));
      });
      if (keep.length !== pending.length) pending = keep;
    });
  });

  async function loadCalls() {
    const jid = chat.jid;
    try {
      const page = await api.listCalls({ chat_jid: jid, missed_only: false, media: null, cursor: null, limit: 200 });
      if (jid === chat.jid) callRecs = page.items;
    } catch {
      if (jid === chat.jid) callRecs = [];
    }
  }

  // Live updates.
  const refreshNew = debounce(async () => {
    if (detached) {
      unseen++;
      return;
    }
    try {
      const page = await api.getMessages({ chat_jid: chat.jid, before: null, after: null, limit: 50 });
      const known = new Set(messages.map((m) => m.id));
      const fresh = page.items.filter((m) => !known.has(m.id) && !m.is_from_me).length;
      messages = merge(messages, page.items);
      if (!atBottom && fresh) {
        unseen += fresh;
        if (unreadFromId === null) unreadFromId = page.items.find((m) => !known.has(m.id) && !m.is_from_me)?.id ?? null;
      }
      if (atBottom) {
        await tick();
        list?.scrollToBottom();
      }
    } catch {
      /* transient */
    }
  }, 120);

  const refreshChanged = debounce(async () => {
    try {
      if (detached) {
        const mid = messages[Math.floor(messages.length / 2)];
        if (!mid) return;
        messages = merge(messages, await api.getMessageContext(chat.jid, mid.id, Math.ceil(messages.length / 2) + 1));
        return;
      }
      // Re-read every loaded page so edits/reactions/revokes/ticks are reflected.
      const oldest = messages[0]?.id;
      let cursor: string | null = null;
      let acc: Message[] = [];
      for (let i = 0; i < 40; i++) {
        const page: Awaited<ReturnType<typeof api.getMessages>> = await api.getMessages({ chat_jid: chat.jid, before: cursor, after: null, limit: 500 });
        acc = acc.concat(page.items);
        cursor = page.next_cursor;
        if (!cursor || (oldest && page.items.some((m) => m.id === oldest))) break;
      }
      const oldestTs = messages[0]?.ts ?? 0;
      messages = merge(messages, acc.filter((m) => m.ts >= oldestTs));
    } catch {
      /* transient */
    }
  }, 200);

  $effect(() => {
    const a = api.subscribe("db:new-messages", (p) => {
      if (p.chat_jids.includes(chat.jid)) refreshNew();
    });
    const b = api.subscribe("db:changed", (p) => {
      if (p.chat_jids.includes(chat.jid)) refreshChanged();
    });
    const c = api.subscribe("call:update", (call) => {
      if (call.chat_jid === chat.jid) callRecs = upsertCall(callRecs, call);
    });
    return () => {
      a();
      b();
      c();
      refreshNew.cancel();
      refreshChanged.cancel();
    };
  });

  // ------------------------------------------------------------ sending (optimistic)

  let seqId = 0;
  /** Sends run strictly one after another (a batch of files keeps its order). */
  let sendChain: Promise<void> = Promise.resolve();
  /** Progress of the current burst of sends (shown when more than one). */
  let batch = $state({ total: 0, done: 0, failed: 0 });
  function onSend(req: SendReq, quote: Message | null) {
    const p: Pending = { id: `local:${Date.now()}:${seqId++}`, chat: chat.jid, req, ts: Math.floor(Date.now() / 1000), state: "pending", awaiting: false, quote };
    pending = [...pending, p];
    if (detached) void loadLatest(true);
    else {
      void tick().then(() => list?.scrollToBottom());
    }
    batch.total++;
    // runSend never rejects, so one failed file does not stop the ones after it.
    sendChain = sendChain.then(async () => {
      const ok = await runSend(p.id);
      batch.done++;
      if (!ok) batch.failed++;
      if (batch.done >= batch.total) setTimeout(() => batch.done >= batch.total && (batch = { total: 0, done: 0, failed: 0 }), 1500);
    });
  }

  /** Send one pending message; resolves true when it went out, false when it failed (never rejects). */
  async function runSend(id: string): Promise<boolean> {
    const upd = (patch: Partial<Pending>) => (pending = pending.map((p) => (p.id === id ? { ...p, ...patch } : p)));
    const p = pending.find((x) => x.id === id);
    if (!p) return true;
    const file = p.req.attachment ? baseName(p.req.attachment) : null;
    upd({ state: "pending", awaiting: false });
    try {
      const res = await api.sendMessage($state.snapshot(p.req));
      if (!res.ok) {
        upd({ state: "failed" });
        toasts.push({ kind: "error", title: file ? `${file} not sent` : "Not sent", message: res.message });
        return false;
      }
      upd({ awaiting: true });
      refreshNew();
      setTimeout(() => (pending = pending.filter((x) => x.id !== id)), 10_000);
      return true;
    } catch (e) {
      upd({ state: "failed" });
      toasts.error(e, file ? `${file} not sent` : "Message not sent");
      return false;
    }
  }

  function tickFor(m: Message): TickState | null {
    const p = pendingById.get(m.id);
    if (p) return p.state === "failed" ? "failed" : "pending";
    if (!m.is_from_me) return null;
    return m.status ?? "sent";
  }

  // ------------------------------------------------------------ actions

  async function onReact(m: Message, emoji: string) {
    const before = m.reactions;
    const idx = messages.findIndex((x) => x.id === m.id);
    const optimistic = [...before.filter((r) => !r.is_from_me), ...(emoji ? [{ emoji, sender_name: "Me", is_from_me: true }] : [])];
    if (idx >= 0) messages[idx] = { ...m, reactions: optimistic };
    try {
      await api.react({ chat_jid: m.chat_jid, message_id: m.id, emoji });
    } catch (e) {
      const j = messages.findIndex((x) => x.id === m.id);
      if (j >= 0) messages[j] = { ...m, reactions: before };
      toasts.error(e, "Reaction failed");
    }
  }

  function onMedia(m: Message, ref: MediaRef) {
    const i = messages.findIndex((x) => x.id === m.id);
    if (i >= 0) messages[i] = { ...m, media: ref };
  }

  async function setStar(ids: string[], starred: boolean) {
    const prev = messages;
    messages = messages.map((m) => (ids.includes(m.id) ? { ...m, starred } : m));
    try {
      await api.setStarred(chat.jid, ids, starred);
    } catch (e) {
      messages = prev;
      toasts.error(e, "Couldn't update stars");
    }
  }

  async function copySelection(ids: string[]) {
    try {
      const text = await api.copyMessagesText(chat.jid, ids);
      if (await copyText(text)) toasts.success(`Copied ${ids.length} message${ids.length === 1 ? "" : "s"}`);
      else toasts.push({ kind: "warn", title: "Clipboard unavailable" });
    } catch (e) {
      toasts.error(e, "Couldn't copy");
    }
  }

  function openViewer(m: Message) {
    const items: LightboxItem[] = messages
      .filter((x) => x.media && x.media.url && (x.media.kind === "image" || x.media.kind === "video") && !x.deleted)
      .map((x) => ({
        key: x.id,
        url: x.media?.url ?? "",
        kind: x.media?.kind === "video" ? "video" : "image",
        title: x.content || x.media?.filename || "",
        subtitle: `${x.is_from_me ? "You" : displayName(x.sender_name, x.sender_jid)} · ${new Date(x.ts * 1000).toLocaleString()}`,
        poster: x.media?.thumb_url ?? null,
        thumb: x.media?.thumb_url ?? null,
        chatJid: x.chat_jid,
        messageId: x.id,
        filename: x.media?.filename ?? null,
      }));
    const index = Math.max(0, items.findIndex((i) => i.key === m.id));
    lightbox = { items, index };
  }

  async function downloadSelected() {
    const targets = messages.filter((m) => selected.has(m.id) && m.media);
    if (targets.length === 0) return toasts.push({ kind: "info", title: "No media in the selection" });
    try {
      const dir = await pickFolder("Save media to");
      if (!dir) return;
      let ok = 0;
      for (const m of targets) {
        let ref = m.media as MediaRef;
        if (!ref.url) {
          const got = await downloadMedia(m.chat_jid, m.id);
          if (!got) continue;
          ref = got;
          onMedia(m, got);
        }
        await api.saveMediaAs(m.chat_jid, m.id, `${dir}/${ref.filename ?? m.id}`);
        ok++;
      }
      toasts.success(`Saved ${ok} of ${targets.length} file${targets.length === 1 ? "" : "s"}`, dir);
    } catch (e) {
      toasts.error(e, "Couldn't save media");
    }
  }

  // ------------------------------------------------------------ selection

  function startSelect(id?: string) {
    selecting = true;
    if (id) {
      selected = new Set([id]);
      anchor = id;
    }
  }
  function cancelSelection() {
    selecting = false;
    selected = new Set();
    anchor = null;
  }
  function onSelect(m: Message, e: MouseEvent | KeyboardEvent) {
    const r = applyClick(selected, orderedIds, anchor, m.id, e.shiftKey);
    selected = r.selected;
    anchor = r.anchor;
  }
  function selectAll() {
    selecting = true;
    selected = new Set(orderedIds);
  }
  const selIds = $derived(inOrder(orderedIds, selected));
  const allStarred = $derived(selIds.length > 0 && selIds.every((id) => messages.find((m) => m.id === id)?.starred));

  function onKey(e: KeyboardEvent) {
    const t = e.target as HTMLElement | null;
    const typing = !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a" && !typing && !e.shiftKey) {
      if (overlayCount() > 0) return;
      e.preventDefault();
      selectAll();
    }
  }

  function onWindowKey(e: KeyboardEvent) {
    if (e.key === "Escape" && selecting && overlayCount() === 0 && !ctxMenu.isOpen) {
      e.preventDefault();
      cancelSelection();
    }
  }

  // ------------------------------------------------------------ menus

  function menuFor(m: Message): MenuItem[] {
    const media = m.media;
    const local = !!media?.url;
    const items: MenuItem[] = [];
    if (!m.deleted) {
      items.push({ id: "reply", label: "Reply", icon: Reply, onselect: () => (replyTo = m) });
      items.push({ id: "react", label: "React", icon: SmilePlus, onselect: () => (reactReq = { id: m.id, nonce: Date.now() }) });
      if (m.content) items.push({ id: "copy", label: "Copy text", icon: Copy, onselect: () => void copyText(m.content).then((ok) => ok && toasts.success("Copied")) });
    }
    items.push({ id: "select", label: "Select", icon: ListChecks, onselect: () => startSelect(m.id) });
    if (!m.deleted) {
      items.push({ id: "forward", label: "Forward…", icon: Forward, onselect: () => app.openForward(chat.jid, [m.id]) });
      items.push({ id: "star", label: m.starred ? "Unstar" : "Star", icon: m.starred ? StarOff : Star, onselect: () => void setStar([m.id], !m.starred) });
    }
    if (media && !m.deleted) {
      items.push({ id: "sep-media", separator: true });
      if (!local) items.push({ id: "download", label: "Download", icon: Download, onselect: () => void downloadMedia(m.chat_jid, m.id).then((r) => r && onMedia(m, r)) });
      else {
        if (media.kind === "image" || media.kind === "sticker") items.push({ id: "copy-image", label: "Copy image", icon: ImageIcon, onselect: () => void copyImageToClipboard(m.chat_jid, m.id) });
        items.push({ id: "save", label: "Save as…", icon: Save, onselect: () => void saveMedia(m.chat_jid, m.id, media.filename) });
        items.push({ id: "open-ext", label: "Open externally", icon: ExternalLink, onselect: () => void openExternal(m.chat_jid, m.id) });
        items.push({ id: "convert", label: "Convert…", icon: Wand2, onselect: () => (convertMsg = m) });
      }
    }
    items.push({ id: "sep-info", separator: true });
    items.push({ id: "info", label: "Message info", icon: Info, onselect: () => (infoMsg = m) });
    if (m.quoted) items.push({ id: "jump", label: "Jump to quoted", icon: CornerUpLeft, onselect: () => m.quoted && void jumpTo(m.quoted.id) });
    return items;
  }

  function headerItems(): MenuItem[] {
    return [
      { id: "info", label: chat.chat_type === "direct" ? "Contact info" : "Group info", icon: Info, onselect: () => app.openProfile(chat.jid) },
      { id: "search", label: "Search in chat", icon: Search, onselect: () => app.searchInChat(chat.jid) },
      ...chatState.menuItems(chat),
      { id: "sep0", separator: true },
      { id: "select", label: "Select messages", icon: ListChecks, onselect: () => startSelect() },
      { id: "export", label: "Export chat…", icon: FileDown, onselect: () => app.openExport(chat.jid, title) },
      { id: "sep", separator: true },
      { id: "refresh", label: "Refresh names", icon: RefreshCw, onselect: () => void app.refreshNames() },
    ];
  }

  const subtitle = $derived(
    chat.parent
      ? `Community › Group · ${chat.parent.name}`
      : chat.chat_type === "direct" || chat.chat_type === "status"
        ? formatPhone(chat.jid)
        : chat.chat_type === "community" ? "Community" : chat.chat_type === "channel" ? "Channel" : chat.chat_type === "broadcast" ? "Broadcast list" : "Group",
  );
</script>

<svelte:window onkeydown={onWindowKey} />

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  bind:this={section}
  class="flex h-full min-h-0 flex-col"
  aria-label="Conversation with {title}"
  data-testid="thread"
  data-loaded-count={messages.length}
  data-detached={detached}
  onkeydown={onKey}
>
  {#if selecting}
    <header class="flex h-14 shrink-0 items-center gap-2 border-b border-border bg-accent-soft px-3" data-testid="selection-bar">
      <button class="icon-btn" aria-label="Cancel selection" onclick={cancelSelection} data-testid="sel-cancel"><X size={18} /></button>
      <span class="flex-1 text-[15px] font-semibold" data-testid="sel-count" aria-live="polite">{selIds.length} selected</span>
      <button class="btn btn-sm" disabled={selIds.length === 0} onclick={() => void copySelection(selIds)} data-testid="sel-copy"><Copy size={13} /> Copy</button>
      <button class="btn btn-sm" disabled={selIds.length === 0} onclick={() => app.openForward(chat.jid, selIds)} data-testid="sel-forward"><Forward size={13} /> Forward</button>
      <button class="btn btn-sm" disabled={selIds.length === 0} onclick={() => void setStar(selIds, !allStarred)} data-testid="sel-star">
        {#if allStarred}<StarOff size={13} /> Unstar{:else}<Star size={13} /> Star{/if}
      </button>
      <button class="btn btn-sm" disabled={selIds.length === 0} onclick={() => app.openExport(chat.jid, title, selIds)} data-testid="sel-export"><FileDown size={13} /> Export</button>
      <button class="btn btn-sm" disabled={selIds.length === 0} onclick={downloadSelected} data-testid="sel-download"><Download size={13} /> Download</button>
    </header>
  {:else}
    <header class="flex h-14 shrink-0 items-center gap-3 border-b border-border bg-surface px-4">
      <button class="flex min-w-0 flex-1 items-center gap-3 rounded-md text-left" onclick={() => app.openProfile(chat.jid)} aria-label="Open {title} info" data-testid="thread-header">
        <Avatar name={title} id={chat.jid} url={chat.avatar_url} type={chat.chat_type} size={36} />
        <div class="min-w-0 flex-1">
          <h2 class="truncate text-[15px] font-semibold" data-testid="thread-title">{title}</h2>
          <p class="truncate text-xs text-muted" data-testid="thread-subtitle">{subtitle}</p>
        </div>
      </button>
      {#if loadingOlder || loadingNewer}<Spinner size={14} label="Loading messages" />{/if}
      {#if chat.chat_type === "direct" || chat.chat_type === "group"}
        <!-- Linked devices have no call media: these only explain where to call from. -->
        <button class="icon-btn opacity-60" aria-disabled="true" aria-label="Video call. {CALL_FROM_PHONE_HINT}" title={CALL_FROM_PHONE_HINT} onclick={(e) => e.preventDefault()} data-testid="header-video-call"><Video size={18} /></button>
        <button class="icon-btn opacity-60" aria-disabled="true" aria-label="Voice call. {CALL_FROM_PHONE_HINT}" title={CALL_FROM_PHONE_HINT} onclick={(e) => e.preventDefault()} data-testid="header-voice-call"><Phone size={17} /></button>
      {/if}
      <button bind:this={moreBtn} class="icon-btn" aria-label="Chat menu" aria-haspopup="menu" onclick={() => moreBtn && ctxMenu.openFor(moreBtn, headerItems(), "Chat menu")} data-testid="thread-menu"><MoreVertical size={18} /></button>
    </header>
  {/if}

  <div class="relative min-h-0 flex-1" style="background: var(--thread-bg)">
    {#if loading}
      <Skeleton rows={8} />
    {:else if error}
      <ErrorPanel {error} reset={() => void loadLatest()} />
    {:else if allMessages.length === 0}
      <EmptyState icon={MessageSquareDashed} title="No messages yet" message="Say hello 👋" />
    {:else}
      <VirtualList
        items={rows}
        getKey={(r) => r.key}
        estimate={estimate}
        canAnchor={(r) => r.kind === "msg"}
        class="h-full"
        stickToBottom
        startAtBottom
        bind:api={list}
        onNearTop={loadOlder}
        onNearBottom={loadNewer}
        onScroll={(b) => {
          atBottom = b;
          if (b && !detached) unseen = 0;
        }}
        ariaLabel="Messages"
        role="log"
      >
        {#snippet row(r: Row)}
          {#if r.kind === "day"}
            <div class="flex justify-center py-2" role="separator" aria-label={r.label}>
              <span class="rounded-md bg-surface px-2.5 py-0.5 text-[11.5px] font-medium text-muted shadow-[var(--shadow)]" data-testid="day-separator">{r.label}</span>
            </div>
          {:else if r.kind === "call"}
            <div class="flex justify-center px-4 py-1.5" data-testid="call-pill-row">
              <span
                class="inline-flex items-center rounded-full bg-surface px-3 py-1 text-[12px] shadow-[var(--shadow)] {direction(r.c) === 'missed' || direction(r.c) === 'declined' ? 'text-danger' : 'text-muted'}"
                data-testid="call-pill"
                data-call-id={r.c.call_id}
                data-result={r.c.result}
              >{pillText(r.c)}</span>
            </div>
          {:else if r.kind === "unread"}
            <div class="flex items-center gap-3 px-4 py-2" role="separator" aria-label={r.label} data-testid="unread-divider">
              <span class="h-px flex-1 bg-accent/40"></span>
              <span class="rounded-full bg-accent-soft px-3 py-0.5 text-[11.5px] font-semibold text-accent-text">{r.label}</span>
              <span class="h-px flex-1 bg-accent/40"></span>
            </div>
          {:else}
            <MessageBubble
              m={r.m}
              isGroup={isGroupChat}
              showName={r.showName}
              first={r.first}
              highlighted={highlightId === r.m.id}
              tick={tickFor(r.m)}
              local={r.local}
              selecting={selecting && !r.local}
              selected={selected.has(r.m.id)}
              reactRequest={reactReq?.id === r.m.id ? reactReq.nonce : 0}
              menuItems={menuFor}
              onreact={onReact}
              onreply={(m) => (replyTo = m)}
              onjump={(id) => void jumpTo(id)}
              onview={openViewer}
              onpreview={(m) => (previewMsg = m)}
              onmedia={onMedia}
              onselect={onSelect}
              onprofile={(jid) => app.openProfile(jid)}
              onretry={(m) => void runSend(m.id)}
              ondiscard={(m) => (pending = pending.filter((p) => p.id !== m.id))}
            />
          {/if}
        {/snippet}
        {#snippet footer()}<div class="h-2"></div>{/snippet}
      </VirtualList>
      {#if detached || !atBottom || unseen > 0}
        <button
          class="absolute bottom-4 right-5 flex h-9 items-center gap-1.5 rounded-full border border-border bg-surface px-3 text-xs font-medium shadow-[var(--shadow-lg)] hover:bg-surface-2"
          onclick={() => (detached ? void loadLatest(true) : list?.scrollToBottom())}
          aria-label="Jump to latest"
          data-testid="jump-latest"
        >
          <ArrowDown size={14} />
          {#if unseen > 0}<span class="badge badge-accent">{unseen} new</span>{:else}Latest{/if}
        </button>
      {/if}
    {/if}
  </div>

  {#if batch.total > 1}
    <div class="border-t border-border bg-surface px-3 py-1 text-xs text-muted" role="status" data-testid="send-progress">
      {#if batch.done < batch.total}Sending {batch.done + 1} of {batch.total}…{:else}Sent {batch.total - batch.failed} of {batch.total}{batch.failed > 0 ? ` · ${batch.failed} failed (use Retry)` : ""}{/if}
    </div>
  {/if}
  <Composer {chat} {replyTo} confirmSends={app.settings?.confirm_sends ?? true} oncancelreply={() => (replyTo = null)} onsend={onSend} />
</section>

{#if lightbox}
  <Lightbox items={lightbox.items} bind:index={lightbox.index} onclose={() => (lightbox = null)} />
{/if}

{#if previewMsg?.media}
  {@const pm = previewMsg}
  <Modal title={pm.media?.filename ?? "Document"} onclose={() => (previewMsg = null)} width="min(960px, 92vw)">
    <div class="-mx-4 -my-4 h-[75vh]">
      <DocPreview chatJid={pm.chat_jid} messageId={pm.id} filename={pm.media?.filename ?? "Document"} media={pm.media} />
    </div>
  </Modal>
{/if}

{#if convertMsg?.media}
  {@const cm = convertMsg}
  <Modal title="Convert {cm.media?.filename ?? 'media'}" onclose={() => (convertMsg = null)} width="26rem">
    {#if cm.media}<ConvertMenu chatJid={cm.chat_jid} messageId={cm.id} kind={cm.media.kind} filename={cm.media.filename} inline />{/if}
  </Modal>
{/if}

{#if infoMsg}
  {@const im = infoMsg}
  {@const st = tickFor(im)}
  <Modal title="Message info" onclose={() => (infoMsg = null)} width="24rem">
    <dl class="space-y-2.5 text-[13px]" data-testid="message-info">
      <div><dt class="section-title">From</dt><dd>{im.is_from_me ? "You" : displayName(im.sender_name, im.sender_jid)}</dd></div>
      <div><dt class="section-title">Time</dt><dd>{dateTime(im.ts)}</dd></div>
      {#if st}<div><dt class="section-title">Status</dt><dd class="flex items-center gap-1.5"><StatusTicks status={st} /> {TICK_TEXT[st]}</dd></div>{/if}
      {#if im.starred}<div><dt class="section-title">Starred</dt><dd>Yes</dd></div>{/if}
      <div><dt class="section-title">ID</dt><dd class="break-all font-mono text-[11px] text-muted">{im.id}</dd></div>
    </dl>
  </Modal>
{/if}
