<!-- Preview a message's document via preview_doc (office → PDF), with instant placeholders and Convert. -->
<script lang="ts">
  import { FileQuestion } from "@lucide/svelte";
  import * as api from "../api";
  import type { MediaRef } from "../bindings/MediaRef";
  import type { PreviewRef } from "../bindings/PreviewRef";
  import PdfViewer from "./PdfViewer.svelte";
  import Spinner from "./Spinner.svelte";
  import ErrorPanel from "./ErrorPanel.svelte";
  import EmptyState from "./EmptyState.svelte";
  import ConvertMenu from "./ConvertMenu.svelte";
  import { extOf } from "../utils/text";

  interface Props {
    chatJid: string;
    messageId: string;
    filename: string;
    /** The message's media, when known: gives an instant thumbnail and enables Convert. */
    media?: MediaRef | null;
  }
  let { chatJid, messageId, filename, media = null }: Props = $props();

  let preview = $state<PreviewRef | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let text = $state<string | null>(null);
  let seq = 0;

  async function load(c: string, m: string) {
    const my = ++seq;
    error = null;
    text = null;
    // A local PDF needs no round-trip: start rendering at once.
    if (media?.url && media.kind === "document" && extOf(media.filename) === "pdf") {
      preview = { kind: "pdf", url: media.url, original_url: media.url, note: null };
      loading = false;
      return;
    }
    loading = true;
    preview = null;
    try {
      const p = await api.previewDoc(c, m);
      if (my !== seq) return;
      preview = p;
      if (p.kind === "text" && p.url) {
        const r = await fetch(p.url);
        text = (await r.text()).slice(0, 200_000);
      }
    } catch (e) {
      if (my === seq) error = e;
    } finally {
      if (my === seq) loading = false;
    }
  }

  $effect(() => {
    void load(chatJid, messageId);
  });
</script>

<div class="flex h-full min-h-0 flex-col" data-testid="doc-preview-body">
  {#if media}
    <div class="flex items-center justify-end gap-2 border-b border-border bg-surface px-2 py-1">
      <ConvertMenu {chatJid} {messageId} kind={media.kind} filename={media.filename ?? filename} />
    </div>
  {/if}
  {#if loading}
    <div class="relative flex min-h-0 flex-1 items-center justify-center bg-surface-3/60 text-sm text-muted">
      {#if media?.thumb_url}<img src={media.thumb_url} alt="" aria-hidden="true" class="max-h-full max-w-full object-contain opacity-80 blur-[2px]" data-testid="preview-placeholder" />{/if}
      <span class="late-in absolute flex items-center gap-2 rounded-md bg-surface/85 px-2.5 py-1.5"><Spinner /> Preparing preview…</span>
    </div>
  {:else if error}
    <ErrorPanel {error} reset={() => void load(chatJid, messageId)} title="Preview unavailable" />
  {:else if preview}
    {#if preview.note}<p class="border-b border-border bg-info-soft px-3 py-1.5 text-xs text-info">{preview.note}</p>{/if}
    {#if preview.kind === "pdf" && preview.url}
      <div class="min-h-0 flex-1"><PdfViewer url={preview.url} title={filename} placeholder={media?.thumb_url ?? null} /></div>
    {:else if preview.kind === "image" && preview.url}
      <div class="relative flex min-h-0 flex-1 items-center justify-center bg-surface-3/60 p-4">
        {#if media?.thumb_url && media.thumb_url !== preview.url}<img src={media.thumb_url} alt="" aria-hidden="true" class="absolute max-h-full max-w-full blur-md" />{/if}
        <img src={preview.url} alt={filename} class="relative max-h-full max-w-full" data-testid="preview-image" />
      </div>
    {:else if preview.kind === "video" && preview.url}
      <!-- svelte-ignore a11y_media_has_caption -->
      <div class="flex min-h-0 flex-1 items-center justify-center bg-black"><video src={preview.url} poster={media?.thumb_url ?? undefined} controls class="max-h-full max-w-full"></video></div>
    {:else if preview.kind === "audio" && preview.url}
      <div class="flex flex-1 items-center justify-center p-6"><audio src={preview.url} controls></audio></div>
    {:else if preview.kind === "text"}
      <pre class="min-h-0 flex-1 select-text overflow-auto whitespace-pre-wrap p-4 font-mono text-xs">{text ?? ""}</pre>
    {:else}
      <EmptyState icon={FileQuestion} title="No preview for this file" message={preview.note ?? "Open it externally or save a copy."} />
    {/if}
  {/if}
</div>
