<script lang="ts">
  import { FileText, ArrowUp, ArrowDown, ExternalLink, Save, Repeat, X, Download, MessageSquare } from "@lucide/svelte";
  import * as api from "../../api";
  import type { MediaItem } from "../../bindings/MediaItem";
  import VirtualList from "../../components/VirtualList.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import DocIcon from "../../components/DocIcon.svelte";
  import Avatar from "../../components/Avatar.svelte";
  import DocPreview from "../../components/DocPreview.svelte";
  import { downloadMedia, openExternal, saveMedia } from "../../media";
  import { formatSize, shortDate, formatCount, displayName } from "../../utils/format";
  import { docBucket } from "../../utils/text";
  import { debounce } from "../../utils/misc";
  import { app } from "../../state/app.svelte";

  type SortKey = "name" | "chat" | "sender" | "date" | "size" | "status";
  let docs = $state<MediaItem[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let text = $state("");
  let bucket = $state<"all" | "pdf" | "office" | "other">("all");
  let sortKey = $state<SortKey>("date");
  let asc = $state(false);
  let selected = $state<MediaItem | null>(null);
  let downloading = $state(false);
  let seq = 0;

  async function load() {
    const my = ++seq;
    loading = docs.length === 0;
    error = null;
    try {
      const all: MediaItem[] = [];
      let cursor: string | null = null;
      do {
        const page = await api.listMedia({ chat_jid: null, kinds: ["document"], text: text.trim() || null, after: null, before: null, available: null, cursor, limit: 500 });
        all.push(...page.items);
        cursor = page.next_cursor;
      } while (cursor && all.length < 5000);
      if (my === seq) docs = all;
    } catch (e) {
      if (my === seq) error = e;
    } finally {
      if (my === seq) loading = false;
    }
  }
  const reload = debounce(() => void load(), 200);
  $effect(() => {
    void text;
    reload();
    return () => reload.cancel();
  });

  const view = $derived.by(() => {
    const list = docs.filter((d) => bucket === "all" || docBucket(d.media.filename, d.media.mime) === bucket);
    const dir = asc ? 1 : -1;
    const val = (d: MediaItem): string | number => {
      switch (sortKey) {
        case "name": return (d.media.filename ?? "").toLowerCase();
        case "chat": return d.chat_name.toLowerCase();
        case "sender": return d.sender_name.toLowerCase();
        case "date": return d.ts;
        case "size": return d.media.size ?? 0;
        case "status": return d.media.source;
      }
    };
    return [...list].sort((a, b) => {
      const x = val(a);
      const y = val(b);
      return (x < y ? -1 : x > y ? 1 : 0) * dir;
    });
  });

  function sortBy(k: SortKey) {
    if (sortKey === k) asc = !asc;
    else {
      sortKey = k;
      asc = k === "name" || k === "chat" || k === "sender";
    }
  }

  const cols: { key: SortKey; label: string; cls: string }[] = [
    { key: "name", label: "Name", cls: "flex-[3]" },
    { key: "chat", label: "Chat", cls: "flex-[2]" },
    { key: "sender", label: "Sender", cls: "flex-[1.5]" },
    { key: "date", label: "Date", cls: "w-28" },
    { key: "size", label: "Size", cls: "w-20 text-right" },
    { key: "status", label: "Status", cls: "w-24" },
  ];

  async function download(d: MediaItem) {
    downloading = true;
    const ref = await downloadMedia(d.chat_jid, d.message_id);
    downloading = false;
    if (ref) {
      const updated = { ...d, media: ref };
      const i = docs.findIndex((x) => x.message_id === d.message_id && x.chat_jid === d.chat_jid);
      if (i >= 0) docs[i] = updated;
      if (selected?.message_id === d.message_id) selected = updated;
    }
  }

  function convert(d: MediaItem) {
    app.toolsInput = { chatJid: d.chat_jid, messageId: d.message_id, label: d.media.filename ?? "document" };
    app.go("tools");
  }
</script>

<div class="flex h-full min-h-0">
  <div class="flex min-w-0 flex-1 flex-col">
    <div class="flex flex-wrap items-center gap-3 border-b border-border bg-surface px-5 py-3">
      <input class="input w-72" placeholder="Filter by name or caption" aria-label="Filter documents" bind:value={text} data-testid="docs-filter" />
      <div class="tabbar" role="tablist" aria-label="Document type">
        {#each [["all", "All"], ["pdf", "PDF"], ["office", "Office"], ["other", "Other"]] as [id, label] (id)}
          <button role="tab" aria-selected={bucket === id} onclick={() => (bucket = id as typeof bucket)}>{label}</button>
        {/each}
      </div>
      <p class="ml-auto text-xs text-muted">{formatCount(view.length)} documents</p>
    </div>
    <div class="flex h-9 shrink-0 items-center gap-3 border-b border-border bg-surface-2/60 px-5 text-xs font-medium text-muted" role="row">
      {#each cols as c (c.key)}
        <button class="flex items-center gap-1 {c.cls} {c.cls.includes('text-right') ? 'justify-end' : ''} hover:text-fg" role="columnheader" aria-sort={sortKey === c.key ? (asc ? "ascending" : "descending") : "none"} onclick={() => sortBy(c.key)}>
          {c.label}
          {#if sortKey === c.key}{#if asc}<ArrowUp size={12} />{:else}<ArrowDown size={12} />{/if}{/if}
        </button>
      {/each}
    </div>
    <div class="min-h-0 flex-1">
      {#if loading}
        <Skeleton rows={10} />
      {:else if error}
        <ErrorPanel {error} reset={load} />
      {:else if view.length === 0}
        <EmptyState icon={FileText} title="No documents" message="Nothing matches these filters." />
      {:else}
        <VirtualList items={view} getKey={(d) => d.chat_jid + d.message_id} estimate={44} class="h-full" role="table" ariaLabel="Documents">
          {#snippet row(d: MediaItem)}
            {@const isSel = selected?.message_id === d.message_id && selected?.chat_jid === d.chat_jid}
            <button class="flex h-11 w-full items-center gap-3 border-b border-border px-5 text-left text-[13px] {isSel ? 'bg-accent-soft' : 'hover:bg-surface-2'}" role="row" aria-selected={isSel} onclick={() => (selected = d)} data-testid="doc-row">
              <span class="flex min-w-0 flex-[3] items-center gap-2" role="cell"><DocIcon filename={d.media.filename} mime={d.media.mime} size={24} /><span class="truncate">{d.media.filename}</span></span>
              <span class="flex-[2] truncate text-muted" role="cell">{d.chat_name}</span>
              <span class="flex min-w-0 flex-[1.5] items-center gap-1.5 text-muted" role="cell">{#if !d.is_from_me}<Avatar name={d.sender_name} id={d.sender_jid ?? d.chat_jid} size={18} />{/if}<span class="truncate">{d.is_from_me ? "You" : displayName(d.sender_name, d.sender_jid)}</span></span>
              <span class="w-28 text-muted" role="cell">{shortDate(d.ts)}</span>
              <span class="w-20 text-right tabular-nums text-muted" role="cell">{formatSize(d.media.size)}</span>
              <span class="w-24" role="cell">
                {#if d.media.source === "missing"}<span class="badge badge-warn">Missing</span>{:else if d.media.source === "phone"}<span class="badge badge-info">Phone</span>{:else}<span class="badge badge-accent">Local</span>{/if}
              </span>
            </button>
          {/snippet}
        </VirtualList>
      {/if}
    </div>
  </div>

  {#if selected}
    {@const d = selected}
    <aside class="flex w-[46%] min-w-[380px] max-w-[760px] flex-col border-l border-border bg-surface" aria-label="Document preview" data-testid="doc-panel">
      <div class="flex items-center gap-2 border-b border-border px-3 py-2">
        <DocIcon filename={d.media.filename} mime={d.media.mime} size={28} />
        <div class="min-w-0 flex-1">
          <p class="truncate text-[13px] font-semibold">{d.media.filename}</p>
          <p class="truncate text-[11px] text-muted">{d.chat_name} · {formatSize(d.media.size)}</p>
        </div>
        <button class="icon-btn" aria-label="Close preview" onclick={() => (selected = null)}><X size={16} /></button>
      </div>
      <div class="flex flex-wrap gap-1.5 border-b border-border px-3 py-2">
        {#if d.media.source === "missing"}
          <button class="btn btn-sm btn-primary" disabled={downloading} onclick={() => download(d)}>{#if downloading}<Spinner size={12} />{:else}<Download size={13} />{/if} Download</button>
        {:else}
          <button class="btn btn-sm" onclick={() => openExternal(d.chat_jid, d.message_id)}><ExternalLink size={13} /> Open externally</button>
          <button class="btn btn-sm" onclick={() => saveMedia(d.chat_jid, d.message_id, d.media.filename)}><Save size={13} /> Save as…</button>
          <button class="btn btn-sm" onclick={() => convert(d)}><Repeat size={13} /> Convert</button>
        {/if}
        <button class="btn btn-sm btn-ghost" onclick={() => app.jumpTo(d.chat_jid, d.message_id)}><MessageSquare size={13} /> Show in chat</button>
      </div>
      <div class="min-h-0 flex-1">
        {#if d.media.source === "missing"}
          <EmptyState icon={FileText} title="Not downloaded yet" message="Download the file to preview it." />
        {:else}
          {#key d.chat_jid + d.message_id}
            <DocPreview chatJid={d.chat_jid} messageId={d.message_id} filename={d.media.filename ?? ""} media={d.media} />
          {/key}
        {/if}
      </div>
    </aside>
  {/if}
</div>
