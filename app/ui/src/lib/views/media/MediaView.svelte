<script lang="ts">
  import { Images, Download, Play, Music, ImageOff, ListChecks, X, Check } from "@lucide/svelte";
  import * as api from "../../api";
  import type { MediaItem } from "../../bindings/MediaItem";
  import type { MediaKind } from "../../bindings/MediaKind";
  import ChatPicker from "../../components/ChatPicker.svelte";
  import VirtualList from "../../components/VirtualList.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import Avatar from "../../components/Avatar.svelte";
  import Lightbox, { type LightboxItem } from "../../components/Lightbox.svelte";
  import Modal from "../../components/Modal.svelte";
  import ExportMediaDialog, { type ExportItems } from "../../components/ExportMediaDialog.svelte";
  import DocPreview from "../../components/DocPreview.svelte";
  import DocIcon from "../../components/DocIcon.svelte";
  import { downloadMedia, openExternal } from "../../media";
  import { formatSize, shortDate, formatCount, mediaEmoji, displayName } from "../../utils/format";
  import { debounce } from "../../utils/misc";
  import { app } from "../../state/app.svelte";

  const KINDS: MediaKind[] = ["image", "video", "audio", "document", "sticker"];
  let kinds = $state<MediaKind[]>(["image", "video"]);
  let chat = $state<string[]>([]);
  let available = $state<boolean | null>(null);
  let items = $state<MediaItem[]>([]);
  let cursor = $state<string | null>(null);
  let loading = $state(true);
  let loadingMore = $state(false);
  let error = $state<unknown>(null);
  let width = $state(900);
  let lightbox = $state<{ items: LightboxItem[]; index: number } | null>(null);
  let preview = $state<MediaItem | null>(null);
  let busy = $state<Set<string>>(new Set());
  let seq = 0;
  /** Multi-select mode: tiles toggle instead of opening, and "Download" exports the picks. */
  let selecting = $state(false);
  let picked = $state<Set<string>>(new Set());
  let exportItems = $state<ExportItems | null>(null);
  const keyOf = (it: MediaItem) => `${it.chat_jid}/${it.message_id}`;

  function toggleSelecting() {
    selecting = !selecting;
    picked = new Set();
  }
  function pick(it: MediaItem) {
    const k = keyOf(it);
    const next = new Set(picked);
    if (next.has(k)) next.delete(k);
    else next.add(k);
    picked = next;
  }
  function downloadPicked() {
    const targets = items.filter((x) => picked.has(keyOf(x))).map((x) => ({ chat_jid: x.chat_jid, message_id: x.message_id }));
    if (targets.length === 0) return;
    exportItems = { targets, label: chat.length ? (items.find((x) => x.chat_jid === chat[0])?.chat_name ?? "WhatsApp media") : "WhatsApp media" };
  }

  const TILE = 168;
  const cols = $derived(Math.max(2, Math.floor((width - 24) / TILE)));
  const rows = $derived.by(() => {
    const out: MediaItem[][] = [];
    for (let i = 0; i < items.length; i += cols) out.push(items.slice(i, i + cols));
    return out;
  });

  async function load() {
    const my = ++seq;
    loading = true;
    error = null;
    try {
      const page = await api.listMedia({ chat_jid: chat[0] ?? null, kinds: $state.snapshot(kinds), text: null, after: null, before: null, available, cursor: null, limit: 200 });
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
      const page = await api.listMedia({ chat_jid: chat[0] ?? null, kinds: $state.snapshot(kinds), text: null, after: null, before: null, available, cursor, limit: 200 });
      if (my !== seq) return;
      items = [...items, ...page.items];
      cursor = page.next_cursor;
    } finally {
      loadingMore = false;
    }
  }

  const reload = debounce(() => void load(), 100);
  $effect(() => {
    void kinds;
    void chat;
    void available;
    reload();
    return () => reload.cancel();
  });

  function toggleKind(k: MediaKind) {
    kinds = kinds.includes(k) ? kinds.filter((x) => x !== k) : [...kinds, k];
  }

  function open(it: MediaItem) {
    const m = it.media;
    if (!m.url) return;
    if (m.kind === "image" || m.kind === "video" || m.kind === "sticker") {
      const list: LightboxItem[] = items
        .filter((x) => x.media.url && (x.media.kind === "image" || x.media.kind === "video" || x.media.kind === "sticker"))
        .map((x) => ({
          key: `${x.chat_jid}/${x.message_id}`,
          url: x.media.url ?? "",
          kind: x.media.kind === "video" ? "video" : "image",
          title: x.caption || x.media.filename || "",
          subtitle: `${x.chat_name} · ${x.is_from_me ? "You" : x.sender_name} · ${shortDate(x.ts, Date.now(), true)}`,
          poster: x.media.thumb_url,
          thumb: x.media.thumb_url,
          chatJid: x.chat_jid,
          messageId: x.message_id,
          filename: x.media.filename,
        }));
      lightbox = { items: list, index: Math.max(0, list.findIndex((l) => l.key === `${it.chat_jid}/${it.message_id}`)) };
    } else preview = it;
  }

  async function download(it: MediaItem) {
    const key = `${it.chat_jid}/${it.message_id}`;
    busy = new Set([...busy, key]);
    const ref = await downloadMedia(it.chat_jid, it.message_id);
    busy = new Set([...busy].filter((k) => k !== key));
    if (ref) {
      const i = items.findIndex((x) => x.message_id === it.message_id && x.chat_jid === it.chat_jid);
      if (i >= 0) items[i] = { ...it, media: ref };
    }
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  <div class="flex flex-wrap items-end gap-3 border-b border-border bg-surface px-5 py-3">
    <div class="flex flex-wrap items-center gap-1.5" role="group" aria-label="Media kinds">
      {#each KINDS as k (k)}
        <button class="chip capitalize" aria-pressed={kinds.includes(k)} onclick={() => toggleKind(k)}>{mediaEmoji(k)} {k}</button>
      {/each}
    </div>
    <div class="w-64"><ChatPicker bind:selected={chat} multiple={false} label="Chat" placeholder="All chats" /></div>
    <div>
      <span class="label">Status</span>
      <div class="tabbar" role="tablist" aria-label="Availability">
        <button role="tab" aria-selected={available === null} onclick={() => (available = null)}>All</button>
        <button role="tab" aria-selected={available === true} onclick={() => (available = true)}>Downloaded</button>
        <button role="tab" aria-selected={available === false} onclick={() => (available = false)}>Missing</button>
      </div>
    </div>
    <p class="ml-auto text-xs text-muted">{formatCount(items.length)}{cursor ? "+" : ""} items</p>
    {#if selecting}
      <span class="text-[13px] font-semibold" data-testid="media-sel-count" aria-live="polite">{picked.size} selected</span>
      <button class="btn btn-sm" disabled={picked.size === 0} onclick={downloadPicked} data-testid="media-download-selected"><Download size={13} /> Download</button>
      <button class="btn btn-sm" onclick={toggleSelecting} data-testid="media-select-cancel"><X size={13} /> Cancel</button>
    {:else}
      <button class="btn btn-sm" onclick={toggleSelecting} data-testid="media-select-toggle"><ListChecks size={13} /> Select</button>
    {/if}
  </div>

  <div class="min-h-0 flex-1" bind:clientWidth={width}>
    {#if loading && items.length === 0}
      <div class="grid gap-2 p-3" style="grid-template-columns: repeat({cols}, minmax(0, 1fr))">
        {#each Array.from({ length: cols * 3 }, (_, i) => i) as i (i)}<div class="skeleton aspect-square"></div>{/each}
      </div>
    {:else if error}
      <ErrorPanel {error} reset={load} />
    {:else if items.length === 0}
      <EmptyState icon={Images} title="No media here" message="Try other kinds, another chat or include missing files." />
    {:else}
      <VirtualList items={rows} getKey={(r, i) => `${i}:${r[0]?.message_id ?? ""}`} estimate={TILE} class="h-full px-3 pt-3" onNearBottom={more} ariaLabel="Media gallery">
        {#snippet row(r: MediaItem[])}
          <div class="grid gap-2 pb-2" style="grid-template-columns: repeat({cols}, minmax(0, 1fr))">
            {#each r as it (it.chat_jid + it.message_id)}
              {@const m = it.media}
              {@const key = `${it.chat_jid}/${it.message_id}`}
              <div class="group relative aspect-square overflow-hidden rounded-[var(--radius)] border {selecting && picked.has(key) ? 'border-accent ring-2 ring-accent' : 'border-border'} bg-surface-2" data-testid="media-tile" data-selected={selecting ? picked.has(key) : undefined}>
                {#if selecting}
                  <button
                    class="absolute inset-0 z-10 cursor-pointer"
                    aria-label="{picked.has(key) ? 'Deselect' : 'Select'} {m.filename ?? m.kind} from {it.chat_name}"
                    aria-pressed={picked.has(key)}
                    onclick={() => pick(it)}
                    data-testid="media-pick"
                  ></button>
                  <span class="pointer-events-none absolute left-1.5 top-1.5 z-20 flex h-5 w-5 items-center justify-center rounded border-2 {picked.has(key) ? 'border-accent bg-accent text-accent-fg' : 'border-white bg-black/30'}">
                    {#if picked.has(key)}<Check size={12} />{/if}
                  </span>
                {/if}
                {#if m.source === "missing" || !m.url}
                  <div class="flex h-full flex-col items-center justify-center gap-1.5 p-2 text-center text-muted">
                    <ImageOff size={20} />
                    <span class="line-clamp-1 text-[11px]">{m.filename ?? m.kind}</span>
                    <button class="btn btn-sm" onclick={() => download(it)} disabled={busy.has(key)}>
                      {#if busy.has(key)}<Spinner size={12} />{:else}<Download size={12} />{/if} Download
                    </button>
                  </div>
                {:else}
                  <button class="block h-full w-full" onclick={() => open(it)} aria-label="Open {m.filename ?? m.kind} from {it.chat_name}">
                    {#if m.thumb_url && m.kind !== "document"}
                      <img src={m.thumb_url} alt="" loading="lazy" class="h-full w-full object-cover {m.kind === 'sticker' ? 'object-contain p-3' : ''}" />
                    {:else if m.kind === "audio"}
                      <div class="flex h-full flex-col items-center justify-center gap-2 text-muted"><Music size={28} /><span class="text-[11px]">{formatSize(m.size)}</span></div>
                    {:else if m.kind === "document"}
                      <div class="flex h-full flex-col items-center justify-center gap-2 p-2 text-center">
                        {#if m.thumb_url}<img src={m.thumb_url} alt="" loading="lazy" class="h-20 shadow-[var(--shadow)]" />{:else}<DocIcon filename={m.filename} mime={m.mime} size={44} />{/if}
                        <span class="line-clamp-2 text-[11px] text-muted">{m.filename}</span>
                      </div>
                    {:else}
                      <img src={m.url} alt="" loading="lazy" class="h-full w-full object-cover" />
                    {/if}
                    {#if m.kind === "video"}
                      <span class="absolute inset-0 flex items-center justify-center"><span class="rounded-full bg-black/55 p-2 text-white"><Play size={16} /></span></span>
                    {/if}
                  </button>
                {/if}
                <div class="pointer-events-none absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/70 to-transparent px-2 pb-1.5 pt-5 text-[11px] text-white opacity-0 transition-opacity group-hover:opacity-100">
                  <p class="truncate font-medium">{it.chat_name}</p>
                  <p class="flex items-center gap-1 truncate text-white/85"><Avatar name={it.sender_name} id={it.sender_jid ?? it.chat_jid} size={14} /> {it.is_from_me ? "You" : displayName(it.sender_name, it.sender_jid)}</p>
                  <p class="truncate text-white/75">{shortDate(it.ts)} · {formatSize(m.size)}</p>
                </div>
              </div>
            {/each}
          </div>
        {/snippet}
        {#snippet footer()}
          {#if loadingMore}<div class="flex justify-center py-3"><Spinner /></div>{/if}
        {/snippet}
      </VirtualList>
    {/if}
  </div>
</div>

{#if lightbox}
  <Lightbox
    items={lightbox.items}
    bind:index={lightbox.index}
    onclose={() => (lightbox = null)}
    onopen={(li) => {
      const [c, id] = li.key.split("/");
      if (c && id) void openExternal(c, id);
    }}
  />
{/if}

{#if exportItems}
  <ExportMediaDialog
    items={exportItems}
    onclose={() => (exportItems = null)}
    ondone={() => {
      toggleSelecting();
      void load();
    }}
  />
{/if}

{#if preview}
  {@const p = preview}
  <Modal title={p.media.filename ?? "Preview"} onclose={() => (preview = null)} width="min(960px, 92vw)">
    <div class="-mx-4 -my-4 h-[70vh]"><DocPreview chatJid={p.chat_jid} messageId={p.message_id} filename={p.media.filename ?? ""} media={p.media} /></div>
    {#snippet footer()}
      <button class="btn" onclick={() => app.jumpTo(p.chat_jid, p.message_id)}>Show in chat</button>
      <button class="btn" onclick={() => openExternal(p.chat_jid, p.message_id)}>Open externally</button>
    {/snippet}
  </Modal>
{/if}
