<!-- Call log: incoming / missed / outgoing calls recorded by the bridge. Calls can't be placed or answered here. -->
<script lang="ts">
  import { untrack } from "svelte";
  import { Phone, PhoneIncoming, PhoneMissed, PhoneOff, PhoneOutgoing, Video, Users, Info, MessageSquare, Copy, Contact } from "@lucide/svelte";
  import * as api from "../../api";
  import type { CallRecord } from "../../bindings/CallRecord";
  import Avatar from "../../components/Avatar.svelte";
  import VirtualList from "../../components/VirtualList.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import { app } from "../../state/app.svelte";
  import { calls } from "../../state/calls.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { contextMenu } from "../../state/menu.svelte";
  import type { MenuItem } from "../../utils/menu";
  import { CALL_FROM_PHONE_HINT, callDetail, callDuration, callTitle, direction, filterQuery, type CallFilter } from "../../utils/calls";
  import { chatLabel, dayLabel, displayName, formatPhone, timeOfDay } from "../../utils/format";
  import { copyText, debounce } from "../../utils/misc";

  const FILTERS: { id: CallFilter; label: string }[] = [
    { id: "all", label: "All" },
    { id: "missed", label: "Missed" },
    { id: "video", label: "Video" },
  ];

  let filter = $state<CallFilter>("all");
  let items = $state<CallRecord[]>([]);
  let cursor = $state<string | null>(null);
  let loading = $state(true);
  let loadingMore = $state(false);
  let error = $state<unknown>(null);
  let seq = 0;

  async function load() {
    const my = ++seq;
    loading = items.length === 0;
    error = null;
    try {
      const page = await api.listCalls(filterQuery(filter));
      if (my !== seq) return;
      items = page.items;
      cursor = page.next_cursor;
    } catch (e) {
      if (my === seq) error = e;
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function more() {
    if (!cursor || loadingMore || loading) return;
    loadingMore = true;
    const my = seq;
    try {
      const page = await api.listCalls({ ...filterQuery(filter), cursor });
      if (my !== seq) return;
      const known = new Set(items.map((c) => `${c.call_id}:${c.chat_jid}`));
      items = [...items, ...page.items.filter((c) => !known.has(`${c.call_id}:${c.chat_jid}`))];
      cursor = page.next_cursor;
    } catch (e) {
      toasts.error(e, "Couldn't load more calls");
    } finally {
      loadingMore = false;
    }
  }

  function pick(f: CallFilter) {
    if (f === filter) return;
    filter = f;
    items = [];
    cursor = null;
    void load();
  }

  const reload = debounce(() => void load(), 150);

  // Runs once on mount: load() and markSeen() read and write state, so they must not
  // become dependencies of this effect (that re-ran it after every load, forever).
  $effect(() => {
    untrack(() => {
      void load();
      calls.viewing = true;
      calls.markSeen();
    });
    const un = api.subscribe("call:update", () => {
      reload();
      calls.markSeen();
    });
    return () => {
      un();
      reload.cancel();
      calls.viewing = false;
      calls.markSeen();
    };
  });

  const now = Date.now();
  function when(ts: number): string {
    return `${dayLabel(ts, now)}, ${timeOfDay(ts)}`;
  }

  function title(c: CallRecord): string {
    return chatLabel({ name: c.chat_name, jid: c.chat_jid, chat_type: c.is_group ? "group" : "direct" });
  }

  function number(c: CallRecord): string {
    return formatPhone(c.caller_jid);
  }

  async function copyNumber(c: CallRecord) {
    const n = number(c);
    if (n && (await copyText(n))) toasts.success("Number copied");
  }

  function itemsFor(c: CallRecord): MenuItem[] {
    return [
      { id: "open", label: "Open chat", icon: MessageSquare, onselect: () => app.openChat(c.chat_jid) },
      {
        id: "info",
        label: c.is_group ? "Group info" : "Contact info",
        icon: Contact,
        onselect: () => {
          app.openChat(c.chat_jid);
          app.openProfile(c.chat_jid);
        },
      },
      { id: "copy", label: c.is_group ? "Copy caller's number" : "Copy number", icon: Copy, onselect: () => void copyNumber(c) },
    ];
  }

  const EMPTY: Record<CallFilter, string> = {
    all: "Calls made and received on your phone appear here.",
    missed: "You haven't missed any calls.",
    video: "No video calls yet.",
  };
</script>

<div class="flex h-full min-h-0 flex-col" data-testid="calls-view">
  <div class="flex shrink-0 flex-wrap items-center gap-3 border-b border-border bg-surface px-5 py-2.5">
    <div class="tabbar" role="tablist" aria-label="Call filter">
      {#each FILTERS as f (f.id)}
        <button role="tab" aria-selected={filter === f.id} onclick={() => pick(f.id)} data-testid="calls-filter-{f.id}">{f.label}</button>
      {/each}
    </div>
    <p class="ml-auto flex items-center gap-1.5 text-xs text-muted" title={CALL_FROM_PHONE_HINT} data-testid="calls-note">
      <Info size={13} class="shrink-0 text-faint" />
      Calls can only be placed or answered on your phone. Here you can review them and decline a ringing call.
    </p>
  </div>

  <div class="min-h-0 flex-1">
    {#if loading}
      <Skeleton rows={9} avatar />
    {:else if error}
      <ErrorPanel {error} reset={() => void load()} />
    {:else if items.length === 0}
      <EmptyState icon={Phone} title={filter === "missed" ? "No missed calls" : "No calls"} message={EMPTY[filter]} />
    {:else}
      <VirtualList items={items} getKey={(c) => `${c.call_id}:${c.chat_jid}`} estimate={64} class="h-full" ariaLabel="Calls" onNearBottom={more}>
        {#snippet row(c: CallRecord)}
          {@const dir = direction(c)}
          {@const missed = dir === "missed" || dir === "declined"}
          <button
            class="flex h-16 w-full items-center gap-3 px-5 text-left transition-colors hover:bg-surface-2"
            data-testid="call-row"
            data-call-id={c.call_id}
            data-result={c.result}
            data-media={c.media}
            data-direction={dir}
            data-jid={c.chat_jid}
            aria-label="{callTitle(c)} with {title(c)}, {when(c.ts)}"
            use:contextMenu={() => itemsFor(c)}
            onclick={() => app.openChat(c.chat_jid)}
          >
            <Avatar name={title(c)} id={c.chat_jid} url={c.avatar_url} type={c.is_group ? "group" : "direct"} size={42} />
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2">
                <span class="min-w-0 truncate text-[14px] font-medium {missed ? 'text-danger' : ''}" data-testid="call-name" dir="auto">{title(c)}</span>
                {#if c.is_group}<span class="badge shrink-0" data-testid="call-group-badge"><Users size={11} class="mr-1" />Group</span>{/if}
              </div>
              <div class="flex items-center gap-1.5 text-[12.5px] {missed ? 'text-danger' : 'text-muted'}">
                {#if dir === "missed"}<PhoneMissed size={14} class="shrink-0" data-testid="dir-missed" />
                {:else if dir === "declined"}<PhoneOff size={14} class="shrink-0" data-testid="dir-declined" />
                {:else if dir === "outgoing"}<PhoneOutgoing size={14} class="shrink-0 text-muted" data-testid="dir-outgoing" />
                {:else}<PhoneIncoming size={14} class="shrink-0 text-accent-text" data-testid="dir-incoming" />{/if}
                <span class="truncate" data-testid="call-detail">
                  {#if c.is_group}<span class="text-faint">{displayName(c.caller_name, c.caller_jid)}&nbsp;·&nbsp;</span>{/if}{callDetail(c)}
                </span>
              </div>
            </div>
            <div class="flex shrink-0 flex-col items-end gap-0.5 text-right">
              <span class="text-[12px] text-faint" data-testid="call-time">{when(c.ts)}</span>
              <span class="flex items-center gap-1 text-[11.5px] text-muted">
                {#if c.media === "video"}<Video size={13} data-testid="media-video" aria-label="Video call" />{:else}<Phone size={13} data-testid="media-voice" aria-label="Voice call" />{/if}
                {#if callDuration(c.duration_secs) && (c.result === "answered" || c.result === "ended")}<span data-testid="call-duration">{callDuration(c.duration_secs)}</span>{/if}
              </span>
            </div>
          </button>
        {/snippet}
        {#snippet footer()}
          {#if loadingMore}<p class="py-3 text-center text-xs text-faint">Loading…</p>{:else}<div class="h-2"></div>{/if}
        {/snippet}
      </VirtualList>
    {/if}
  </div>
</div>
