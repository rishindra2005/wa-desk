<script lang="ts">
  import { Download, Eye, ExternalLink, Save, ImageOff, Play } from "@lucide/svelte";
  import type { Message } from "../../bindings/Message";
  import type { MediaRef } from "../../bindings/MediaRef";
  import DocIcon from "../../components/DocIcon.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import { downloadMedia, openExternal, saveMedia, mediaLabel } from "../../media";
  import { formatSize } from "../../utils/format";
  import { extOf } from "../../utils/text";

  interface Props {
    message: Message;
    media: MediaRef;
    onview: (m: Message) => void;
    onpreview: (m: Message) => void;
    onupdate: (ref: MediaRef) => void;
  }
  let { message, media, onview, onpreview, onupdate }: Props = $props();

  let busy = $state(false);
  let thumbFailed = $state(false);
  const missing = $derived(media.source === "missing" || !media.url);

  async function download() {
    busy = true;
    const ref = await downloadMedia(message.chat_jid, message.id);
    busy = false;
    if (ref) onupdate(ref);
  }
</script>

{#if media.kind === "document"}
  <div class="flex w-72 max-w-full items-center gap-3 rounded-md bg-black/[0.04] p-2.5 dark:bg-white/[0.05]" data-testid="doc-card">
    <DocIcon filename={media.filename} mime={media.mime} />
    <div class="min-w-0 flex-1">
      <p class="truncate text-[13px] font-medium" title={media.filename ?? ""}>{media.filename ?? "Document"}</p>
      <p class="text-[11px] text-muted">{formatSize(media.size)} · {extOf(media.filename).toUpperCase() || "FILE"}{missing ? " · not downloaded" : ""}</p>
    </div>
  </div>
  <div class="mt-1.5 flex flex-wrap gap-1">
    {#if missing}
      <button class="btn btn-sm" onclick={download} disabled={busy} data-testid="doc-download">
        {#if busy}<Spinner size={13} label="Downloading" />{:else}<Download size={13} />{/if} Download
      </button>
    {:else}
      <button class="btn btn-sm" onclick={() => onpreview(message)} data-testid="doc-preview"><Eye size={13} /> Preview</button>
      <button class="btn btn-sm" onclick={() => openExternal(message.chat_jid, message.id)}><ExternalLink size={13} /> Open</button>
      <button class="btn btn-sm" onclick={() => saveMedia(message.chat_jid, message.id, media.filename)}><Save size={13} /> Save</button>
    {/if}
  </div>
{:else if missing}
  <div class="flex h-36 w-64 max-w-full flex-col items-center justify-center gap-2 rounded-md bg-black/[0.05] text-muted dark:bg-white/[0.05]" data-testid="media-missing">
    <ImageOff size={22} />
    <span class="text-xs">{mediaLabel(media)} · {formatSize(media.size)}</span>
    <button class="btn btn-sm" onclick={download} disabled={busy}>
      {#if busy}<Spinner size={13} label="Downloading" />{:else}<Download size={13} />{/if} Download
    </button>
  </div>
{:else if media.kind === "image"}
  <button class="block overflow-hidden rounded-md" onclick={() => onview(message)} aria-label="View photo" data-testid="media-image">
    {#if thumbFailed}
      <div class="flex h-40 w-56 items-center justify-center bg-surface-3 text-muted"><ImageOff size={22} /></div>
    {:else}
      <img src={media.thumb_url ?? media.url} alt={message.content || media.filename || "Photo"} loading="lazy" class="block max-h-80 min-h-24 w-auto min-w-24 max-w-72 object-cover" onerror={() => (thumbFailed = true)} />
    {/if}
  </button>
{:else if media.kind === "sticker"}
  <img src={media.url} alt="Sticker" loading="lazy" class="h-32 w-32 object-contain" data-testid="media-sticker" />
{:else if media.kind === "video"}
  <div class="relative w-72 max-w-full overflow-hidden rounded-md bg-black">
    <!-- svelte-ignore a11y_media_has_caption -->
    <video src={media.url} poster={media.thumb_url ?? undefined} controls preload="none" class="block max-h-72 w-full" data-testid="media-video"></video>
    <button class="absolute right-1.5 top-1.5 rounded-md bg-black/50 p-1 text-white hover:bg-black/70" aria-label="Open video in viewer" onclick={() => onview(message)}><Play size={14} /></button>
  </div>
{:else if media.kind === "audio"}
  <audio src={media.url} controls preload="none" class="h-10 w-72 max-w-full" data-testid="media-audio"></audio>
{/if}
