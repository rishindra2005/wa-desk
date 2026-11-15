<script lang="ts">
  import { Search as SearchIcon, SlidersHorizontal, Info, X } from "@lucide/svelte";
  import * as api from "../../api";
  import type { MediaKind } from "../../bindings/MediaKind";
  import type { SearchResults } from "../../bindings/SearchResults";
  import type { SearchHit } from "../../bindings/SearchHit";
  import ChatPicker from "../../components/ChatPicker.svelte";
  import VirtualList from "../../components/VirtualList.svelte";
  import Avatar from "../../components/Avatar.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import { app } from "../../state/app.svelte";
  import { highlightSegments, snippetSegments } from "../../utils/text";
  import { dateTime, formatCount, fromDateInput, displayName, mediaEmoji } from "../../utils/format";
  import { debounce } from "../../utils/misc";

  let text = $state(app.searchText);
  let chatJids = $state<string[]>([]);
  let sender = $state("");
  let kinds = $state<MediaKind[]>([]);
  let fromMe = $state<boolean | null>(null);
  let after = $state("");
  let before = $state("");
  let showFilters = $state(true);
  let results = $state<SearchResults | null>(null);
  let loading = $state(false);
  let loadingMore = $state(false);
  let error = $state<unknown>(null);
  let input: HTMLInputElement | undefined = $state();
  let seq = 0;
  const LIMIT = 50;

  const KINDS: MediaKind[] = ["image", "video", "audio", "document", "sticker"];
  const hasFilters = $derived(chatJids.length > 0 || !!sender.trim() || kinds.length > 0 || fromMe !== null || !!after || !!before);

  function buildQuery(offset: number) {
    return {
      text: text.trim(),
      chat_jids: chatJids,
      senders: sender.trim() ? [sender.trim()] : [],
      media_kinds: kinds,
      from_me: fromMe,
      after: after ? fromDateInput(after) : null,
      before: before ? fromDateInput(before, true) : null,
      limit: LIMIT,
      offset,
    };
  }

  async function run() {
    const my = ++seq;
    if (!text.trim() && !hasFilters) {
      results = null;
      loading = false;
      return;
    }
    loading = true;
    error = null;
    try {
      const r = await api.search($state.snapshot(buildQuery(0)));
      if (my === seq) results = r;
    } catch (e) {
      if (my === seq) error = e;
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function more() {
    if (!results || loadingMore || results.hits.length >= results.total) return;
    loadingMore = true;
    const my = seq;
    try {
      const r = await api.search($state.snapshot(buildQuery(results.hits.length)));
      if (my === seq && results) results = { ...r, hits: [...results.hits, ...r.hits] };
    } catch {
      /* ignore */
    } finally {
      loadingMore = false;
    }
  }

  const debounced = debounce(() => void run(), 250);

  $effect(() => {
    void text;
    void chatJids;
    void sender;
    void kinds;
    void fromMe;
    void after;
    void before;
    debounced();
    return () => debounced.cancel();
  });

  // External requests (top bar / Ctrl+F) set the text and focus.
  let lastNonce = -1;
  $effect(() => {
    const n = app.searchNonce;
    if (n !== lastNonce) {
      lastNonce = n;
      if (app.searchText) text = app.searchText;
      queueMicrotask(() => input?.focus());
    }
  });

  function toggleKind(k: MediaKind) {
    kinds = kinds.includes(k) ? kinds.filter((x) => x !== k) : [...kinds, k];
  }
  function clearFilters() {
    chatJids = [];
    sender = "";
    kinds = [];
    fromMe = null;
    after = "";
    before = "";
  }

  function snippet(h: SearchHit) {
    const target = h.message.content || h.message.media?.filename || "";
    return snippetSegments(highlightSegments(target, h.highlights), 220);
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  <div class="border-b border-border bg-surface px-5 py-3">
    <div class="flex items-center gap-2">
      <div class="relative flex-1">
        <SearchIcon size={16} class="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-faint" />
        <input
          bind:this={input}
          bind:value={text}
          class="input h-10 pl-9 text-[14px]"
          placeholder="Search all messages, captions and file names…"
          aria-label="Search messages"
          data-testid="search-input"
          onkeydown={(e) => e.key === "Enter" && debounced.flush()}
        />
      </div>
      <button class="btn h-10" aria-pressed={showFilters} aria-expanded={showFilters} onclick={() => (showFilters = !showFilters)}>
        <SlidersHorizontal size={15} /> Filters {#if hasFilters}<span class="badge badge-accent">on</span>{/if}
      </button>
    </div>
    {#if showFilters}
      <div class="mt-3 grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-4" data-testid="search-filters">
        <ChatPicker bind:selected={chatJids} label="Chats" placeholder="All chats" testid="search-chats" />
        <div>
          <label class="label" for="s-sender">Sender</label>
          <input id="s-sender" class="input" placeholder="Name, number or JID" bind:value={sender} />
        </div>
        <div>
          <span class="label">From</span>
          <div class="tabbar" role="tablist" aria-label="From">
            <button role="tab" aria-selected={fromMe === null} onclick={() => (fromMe = null)}>Anyone</button>
            <button role="tab" aria-selected={fromMe === true} onclick={() => (fromMe = true)}>Me</button>
            <button role="tab" aria-selected={fromMe === false} onclick={() => (fromMe = false)}>Others</button>
          </div>
        </div>
        <div class="flex gap-2">
          <div class="flex-1">
            <label class="label" for="s-after">After</label>
            <input id="s-after" type="date" class="input" bind:value={after} />
          </div>
          <div class="flex-1">
            <label class="label" for="s-before">Before</label>
            <input id="s-before" type="date" class="input" bind:value={before} />
          </div>
        </div>
        <div class="flex flex-wrap items-center gap-1.5 md:col-span-2 xl:col-span-4">
          <span class="label mb-0 mr-1">Media</span>
          {#each KINDS as k (k)}
            <button class="chip capitalize" aria-pressed={kinds.includes(k)} onclick={() => toggleKind(k)}>{mediaEmoji(k)} {k}</button>
          {/each}
          {#if hasFilters}
            <button class="btn btn-ghost btn-sm ml-auto" onclick={clearFilters}><X size={13} /> Clear filters</button>
          {/if}
        </div>
      </div>
    {/if}
  </div>

  {#if results?.index_building}
    <div class="flex items-center gap-2 border-b border-border bg-info-soft px-5 py-2 text-xs text-info" role="status" data-testid="index-banner">
      <Info size={14} /> The search index is still being built — results may be incomplete.
    </div>
  {/if}

  <div class="min-h-0 flex-1">
    {#if loading && !results}
      <Skeleton rows={7} avatar />
    {:else if error}
      <ErrorPanel {error} reset={run} />
    {:else if !results}
      <EmptyState icon={SearchIcon} title="Search your archive" message="Type at least one word. Use filters to narrow by chat, sender, media or date." />
    {:else if results.hits.length === 0}
      <EmptyState icon={SearchIcon} title="No results" message="Try fewer words or remove some filters." />
    {:else}
      <div class="flex h-full flex-col">
        <p class="px-5 py-2 text-xs text-muted" data-testid="search-count" aria-live="polite">
          {formatCount(results.total)}{results.total >= 10_000 ? "+" : ""} result{results.total === 1 ? "" : "s"} · {results.took_ms} ms
          {#if loading}<span class="ml-1 text-faint">updating…</span>{/if}
        </p>
        <VirtualList items={results.hits} getKey={(h) => `${h.message.chat_jid}/${h.message.id}`} estimate={84} class="min-h-0 flex-1" onNearBottom={more} ariaLabel="Search results">
          {#snippet row(h: SearchHit)}
            <button
              class="flex w-full gap-3 border-b border-border px-5 py-3 text-left hover:bg-surface-2"
              onclick={() => app.jumpTo(h.message.chat_jid, h.message.id)}
              data-testid="search-hit"
            >
              <Avatar name={h.chat_name} id={h.message.chat_jid} size={34} />
              <div class="min-w-0 flex-1">
                <div class="flex items-baseline gap-2 text-xs">
                  <span class="truncate font-semibold text-fg">{h.chat_name}</span>
                  {#if h.message.is_from_me || h.message.sender_name !== h.chat_name}<span class="truncate text-muted">{h.message.is_from_me ? "You" : displayName(h.message.sender_name, h.message.sender_jid)}</span>{/if}
                  <span class="ml-auto shrink-0 text-faint">{dateTime(h.message.ts)}</span>
                </div>
                <p class="mt-0.5 line-clamp-2 text-[13px]" dir="auto" data-testid="hit-snippet">
                  {#if h.message.media && !h.message.content}<span class="mr-1">{mediaEmoji(h.message.media.kind)}</span>{/if}
                  {#each snippet(h) as s, i (i)}{#if s.hit}<mark class="hl">{s.text}</mark>{:else}{s.text}{/if}{/each}
                </p>
              </div>
            </button>
          {/snippet}
        </VirtualList>
      </div>
    {/if}
  </div>
</div>
