<script lang="ts">
  import { tick, untrack } from "svelte";
  import { ArrowDown, MessageSquareDashed } from "@lucide/svelte";
  import * as api from "../../api";
  import type { ChatSummary } from "../../bindings/ChatSummary";
  import type { Message } from "../../bindings/Message";
  import type { MediaRef } from "../../bindings/MediaRef";
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
  import MessageBubble from "./MessageBubble.svelte";
  import Composer from "./Composer.svelte";
  import { dayKey, dayLabel, formatPhone } from "../../utils/format";
  import { toasts } from "../../state/toasts.svelte";
  import { app } from "../../state/app.svelte";
  import { debounce } from "../../utils/misc";

  interface Props {
    chat: ChatSummary;
    /** Message to scroll to (search results, quoted replies). */
    jumpId?: string | null;
    jumpNonce?: number;
  }
  let { chat, jumpId = null, jumpNonce = 0 }: Props = $props();

  const PAGE = 80;
  type Row =
    | { kind: "day"; key: string; label: string }
    | { kind: "msg"; key: string; m: Message; first: boolean; showName: boolean };

  let messages = $state<Message[]>([]);
  let olderCursor = $state<string | null>(null);
  /** True when showing a context window that doesn't reach the newest message. */
  let detached = $state(false);
  let loading = $state(true);
  let loadingOlder = $state(false);
  let error = $state<unknown>(null);
  let highlightId = $state<string | null>(null);
  let replyTo = $state<Message | null>(null);
  let atBottom = $state(true);
  let unseen = $state(0);
  let list: VirtualListApi | undefined = $state();
  let lightbox = $state<{ items: LightboxItem[]; index: number } | null>(null);
  let previewMsg = $state<Message | null>(null);
  let loadSeq = 0;

  const rows = $derived.by(() => {
    const out: Row[] = [];
    let prev: Message | null = null;
    const now = Date.now();
    for (const m of messages) {
      const dk = dayKey(m.ts);
      const newDay = !prev || dayKey(prev.ts) !== dk;
      if (newDay) out.push({ kind: "day", key: `d:${dk}`, label: dayLabel(m.ts, now) });
      const sameSender =
        !!prev && !newDay && prev.is_from_me === m.is_from_me && prev.sender_jid === m.sender_jid && m.ts - prev.ts < 300;
      out.push({ kind: "msg", key: m.id, m, first: !sameSender, showName: !sameSender });
      prev = m;
    }
    return out;
  });

  function estimate(r: Row): number {
    if (r.kind === "day") return 40;
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

  async function loadLatest(scrollBottom = true) {
    const my = ++loadSeq;
    loading = messages.length === 0;
    error = null;
    try {
      const page = await api.getMessages({ chat_jid: chat.jid, before: null, limit: PAGE });
      if (my !== loadSeq) return;
      messages = page.items;
      olderCursor = page.next_cursor;
      detached = false;
      unseen = 0;
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
      const page = await api.getMessages({ chat_jid: jid, before: olderCursor, limit: PAGE });
      if (jid !== chat.jid) return;
      messages = merge(messages, page.items);
      olderCursor = page.next_cursor;
    } catch (e) {
      toasts.error(e, "Couldn't load older messages");
    } finally {
      loadingOlder = false;
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
      replyTo = null;
      lightbox = null;
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

  // Live updates.
  const refreshNew = debounce(async () => {
    if (detached) {
      unseen++;
      return;
    }
    try {
      const page = await api.getMessages({ chat_jid: chat.jid, before: null, limit: 50 });
      const known = new Set(messages.map((m) => m.id));
      const fresh = page.items.filter((m) => !known.has(m.id)).length;
      messages = merge(messages, page.items);
      if (!atBottom && fresh) unseen += fresh;
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
      // Re-read every loaded page so edits/reactions/revokes are reflected.
      const oldest = messages[0]?.id;
      let cursor: string | null = null;
      let acc: Message[] = [];
      for (let i = 0; i < 40; i++) {
        const page: Awaited<ReturnType<typeof api.getMessages>> = await api.getMessages({ chat_jid: chat.jid, before: cursor, limit: 500 });
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
    return () => {
      a();
      b();
      refreshNew.cancel();
      refreshChanged.cancel();
    };
  });

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

  function openViewer(m: Message) {
    const items: LightboxItem[] = messages
      .filter((x) => x.media && x.media.url && (x.media.kind === "image" || x.media.kind === "video") && !x.deleted)
      .map((x) => ({
        key: x.id,
        url: x.media?.url ?? "",
        kind: x.media?.kind === "video" ? "video" : "image",
        title: x.content || x.media?.filename || "",
        subtitle: `${x.is_from_me ? "You" : x.sender_name} · ${new Date(x.ts * 1000).toLocaleString()}`,
        poster: x.media?.thumb_url ?? null,
      }));
    const index = Math.max(0, items.findIndex((i) => i.key === m.id));
    lightbox = { items, index };
  }

  const subtitle = $derived(chat.is_group ? "Group" : formatPhone(chat.jid));
</script>

<section class="flex h-full min-h-0 flex-col" aria-label="Conversation with {chat.name}" data-testid="thread" data-loaded-count={messages.length}>
  <header class="flex h-14 shrink-0 items-center gap-3 border-b border-border bg-surface px-4">
    <Avatar name={chat.name} id={chat.jid} size={36} group={chat.is_group} />
    <div class="min-w-0 flex-1">
      <h2 class="truncate text-[15px] font-semibold" data-testid="thread-title">{chat.name}</h2>
      <p class="truncate text-xs text-muted">{subtitle}</p>
    </div>
    {#if loadingOlder}<Spinner size={14} label="Loading older messages" />{/if}
  </header>

  <div class="relative min-h-0 flex-1" style="background: var(--thread-bg)">
    {#if loading}
      <Skeleton rows={8} />
    {:else if error}
      <ErrorPanel {error} reset={() => void loadLatest()} />
    {:else if messages.length === 0}
      <EmptyState icon={MessageSquareDashed} title="No messages yet" message="Say hello 👋" />
    {:else}
      <VirtualList
        items={rows}
        getKey={(r) => r.key}
        estimate={estimate}
        class="h-full"
        stickToBottom
        startAtBottom
        bind:api={list}
        onNearTop={loadOlder}
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
          {:else}
            <MessageBubble
              m={r.m}
              isGroup={chat.is_group}
              showName={r.showName}
              first={r.first}
              highlighted={highlightId === r.m.id}
              onreact={onReact}
              onreply={(m) => (replyTo = m)}
              onjump={(id) => void jumpTo(id)}
              onview={openViewer}
              onpreview={(m) => (previewMsg = m)}
              onmedia={onMedia}
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

  <Composer
    {chat}
    {replyTo}
    confirmSends={app.settings?.confirm_sends ?? true}
    oncancelreply={() => (replyTo = null)}
    onsent={() => {
      if (detached) void loadLatest(true);
      else list?.scrollToBottom();
    }}
  />
</section>

{#if lightbox}
  <Lightbox items={lightbox.items} bind:index={lightbox.index} onclose={() => (lightbox = null)} />
{/if}

{#if previewMsg?.media}
  {@const pm = previewMsg}
  <Modal title={pm.media?.filename ?? "Document"} onclose={() => (previewMsg = null)} width="min(960px, 92vw)">
    <div class="-mx-4 -my-4 h-[75vh]">
      <DocPreview chatJid={pm.chat_jid} messageId={pm.id} filename={pm.media?.filename ?? "Document"} />
    </div>
  </Modal>
{/if}
