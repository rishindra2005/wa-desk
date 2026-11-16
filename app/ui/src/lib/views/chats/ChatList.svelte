<script lang="ts">
  import { Search, MessageSquare, Users, MoreVertical, ChevronDown, CheckCheck, Info, FileDown, Copy, MessageCircle, RefreshCw, Megaphone, Radio, Network } from "@lucide/svelte";
  import * as api from "../../api";
  import type { ChatSummary } from "../../bindings/ChatSummary";
  import type { ChatKind } from "../../bindings/ChatKind";
  import Avatar from "../../components/Avatar.svelte";
  import VirtualList from "../../components/VirtualList.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import StatusTicks from "../../components/StatusTicks.svelte";
  import { relativeTime, chatLabel } from "../../utils/format";
  import { contextMenu, ctxMenu } from "../../state/menu.svelte";
  import type { MenuItem } from "../../utils/menu";
  import { app } from "../../state/app.svelte";
  import { copyText } from "../../utils/misc";
  import { toasts } from "../../state/toasts.svelte";
  import { debounce } from "../../utils/misc";

  interface Props {
    selected: string | null;
    onselect: (chat: ChatSummary) => void;
  }
  let { selected, onselect }: Props = $props();

  let filter = $state("");
  let kind = $state<ChatKind>("all");
  let unreadOnly = $state(false);
  let chats = $state<ChatSummary[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let now = $state(Date.now());
  let seq = 0;

  async function load() {
    const my = ++seq;
    try {
      const all: ChatSummary[] = [];
      let cursor: string | null = null;
      do {
        const page = await api.listChats({ filter: filter.trim() || null, kind, unread_only: unreadOnly, cursor, limit: 500 });
        all.push(...page.items);
        cursor = page.next_cursor;
      } while (cursor && all.length < 5000);
      if (my !== seq) return;
      chats = all;
      error = null;
      now = Date.now();
    } catch (e) {
      if (my === seq) error = e;
    } finally {
      if (my === seq) loading = false;
    }
  }

  const reload = debounce(() => void load(), 150);

  // Refetch on filter/tab/unread change.
  $effect(() => {
    void filter;
    void kind;
    void unreadOnly;
    reload();
  });

  $effect(() => {
    const a = api.subscribe("db:new-messages", () => reload());
    const b = api.subscribe("db:changed", () => reload());
    const t = setInterval(() => (now = Date.now()), 60_000);
    return () => {
      a();
      b();
      clearInterval(t);
      reload.cancel();
    };
  });

  /** Mark a chat read locally (the parent calls mark_read). */
  export function markLocalRead(jid: string) {
    const c = chats.find((x) => x.jid === jid);
    if (c) c.unread = false;
  }

  const tabs: { id: ChatKind; label: string }[] = [
    { id: "all", label: "All" },
    { id: "direct", label: "Direct" },
    { id: "group", label: "Groups" },
    { id: "community", label: "Communities" },
    { id: "channel", label: "Channels" },
    { id: "broadcast", label: "Broadcast" },
  ];

  async function markRead(c: ChatSummary) {
    markLocalRead(c.jid);
    try {
      await api.markRead(c.jid);
      void app.refreshUnread();
    } catch (e) {
      toasts.error(e, "Couldn't mark as read");
    }
  }

  function itemsFor(c: ChatSummary): MenuItem[] {
    return [
      { id: "open", label: "Open", icon: MessageCircle, onselect: () => onselect(c) },
      { id: "read", label: "Mark as read", icon: CheckCheck, disabled: !c.unread, onselect: () => void markRead(c) },
      { id: "info", label: c.chat_type === "direct" ? "Contact info" : c.chat_type === "channel" ? "Channel info" : "Group info", icon: Info, onselect: () => app.openProfile(c.jid) },
      { id: "search", label: "Search in chat", icon: Search, onselect: () => app.searchInChat(c.jid) },
      { id: "export", label: "Export chat…", icon: FileDown, onselect: () => app.openExport(c.jid, chatLabel(c)) },
      { id: "sep", separator: true },
      { id: "jid", label: "Copy JID", icon: Copy, onselect: () => void copyText(c.jid).then((ok) => ok && toasts.success("JID copied")) },
    ];
  }

  let listMenuBtn: HTMLButtonElement | undefined = $state();
  function listMenu() {
    if (!listMenuBtn) return;
    ctxMenu.openFor(listMenuBtn, [{ id: "refresh-names", label: "Refresh names", icon: RefreshCw, onselect: () => void app.refreshNames().then(() => load()) }], "Chat list menu");
  }

  function onListKey(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const i = chats.findIndex((c) => c.jid === selected);
    const next = chats[Math.max(0, Math.min(chats.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
    if (next) onselect(next);
  }
</script>

<section class="flex h-full min-h-0 flex-col" aria-label="Chat list">
  <div class="space-y-2 border-b border-border p-3">
    <div class="relative">
      <Search size={14} class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
      <input class="input pl-8" placeholder="Filter chats" aria-label="Filter chats" bind:value={filter} data-testid="chat-filter" />
    </div>
    <div class="flex items-center justify-between gap-2">
      <div class="tabbar min-w-0 overflow-x-auto" role="tablist" aria-label="Chat type" data-testid="chat-tabs">
        {#each tabs as t (t.id)}
          <button role="tab" class="shrink-0" aria-selected={kind === t.id} onclick={() => (kind = t.id)} data-testid="tab-{t.id}">{t.label}</button>
        {/each}
      </div>
      <div class="flex shrink-0 items-center gap-1">
        <button class="chip" aria-pressed={unreadOnly} onclick={() => (unreadOnly = !unreadOnly)}>Unread</button>
        <button bind:this={listMenuBtn} class="icon-btn h-7 w-7" aria-label="Chat list menu" aria-haspopup="menu" onclick={listMenu} data-testid="list-menu"><MoreVertical size={16} /></button>
      </div>
    </div>
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="min-h-0 flex-1" onkeydown={onListKey}>
    {#if loading && chats.length === 0}
      <Skeleton rows={9} avatar />
    {:else if error && chats.length === 0}
      <ErrorPanel {error} reset={() => void load()} />
    {:else if chats.length === 0}
      <EmptyState icon={filter ? Search : unreadOnly ? MessageSquare : Users} title={filter ? "No chats match" : unreadOnly ? "All caught up" : "No chats yet"} message={filter ? `Nothing matches “${filter}”.` : undefined} />
    {:else}
      <VirtualList items={chats} getKey={(c) => c.jid} estimate={64} class="h-full" ariaLabel="Chats">
        {#snippet row(c: ChatSummary)}
          {@const label = chatLabel(c)}
          <div class="group/row relative">
            <button
              class="flex h-16 w-full items-center gap-3 px-3 text-left transition-colors {c.jid === selected ? 'bg-accent-soft' : 'hover:bg-surface-2'}"
              aria-current={c.jid === selected ? "true" : undefined}
              data-testid="chat-row"
              data-jid={c.jid}
              data-type={c.chat_type}
              use:contextMenu={() => itemsFor(c)}
              onclick={() => onselect(c)}
            >
              <Avatar name={label} id={c.jid} url={c.avatar_url} type={c.chat_type} size={42} />
              <div class="min-w-0 flex-1">
                <div class="flex items-baseline gap-2">
                  <span class="flex min-w-0 flex-1 items-center gap-1 truncate text-[14px] {c.unread ? 'font-semibold' : 'font-medium'}" data-testid="chat-name-wrap">
                    {#if c.chat_type === "community"}<Network size={13} class="shrink-0 text-muted" data-testid="type-community" />
                    {:else if c.chat_type === "channel"}<Megaphone size={13} class="shrink-0 text-muted" data-testid="type-channel" />
                    {:else if c.chat_type === "broadcast"}<Radio size={13} class="shrink-0 text-muted" data-testid="type-broadcast" />{/if}
                    {#if c.parent}<span class="max-w-[45%] shrink-0 truncate font-normal text-faint" data-testid="chat-parent">{c.parent.name} ›</span>{/if}
                    <span class="truncate" data-testid="chat-name">{label}</span>
                  </span>
                  <span class="shrink-0 text-[11px] {c.unread ? 'font-semibold text-accent-text' : 'text-faint'}">{relativeTime(c.last_message_ts, now)}</span>
                </div>
                <div class="flex items-center gap-1.5">
                  {#if c.last_is_from_me && c.last_status}<StatusTicks status={c.last_status} size={14} />{/if}
                  <span class="min-w-0 flex-1 truncate text-[13px] {c.unread ? 'text-fg' : 'text-muted'}">
                    {#if c.last_is_from_me}<span class="text-faint">{c.is_group ? "You: " : ""}</span>{:else if c.is_group && c.last_sender_name}<span class="text-faint">{c.last_sender_name + ": "}</span>{/if}{c.last_preview ?? ""}
                  </span>
                  {#if c.unread}<span class="h-2.5 w-2.5 shrink-0 rounded-full bg-accent" aria-label="Unread" data-testid="unread-dot"></span>{/if}
                </div>
              </div>
            </button>
            <button
              class="icon-btn absolute right-1.5 top-8 h-6 w-6 bg-surface/90 opacity-0 shadow-[var(--shadow)] focus:opacity-100 group-hover/row:opacity-100"
              aria-label="Chat actions for {label}"
              aria-haspopup="menu"
              data-testid="row-menu"
              onclick={(e) => ctxMenu.openFor(e.currentTarget, itemsFor(c), "Chat actions")}
            ><ChevronDown size={14} /></button>
          </div>
        {/snippet}
      </VirtualList>
    {/if}
  </div>
</section>
