<!-- Preview a message's document via preview_doc (office → PDF). -->
<script lang="ts">
  import { FileQuestion } from "@lucide/svelte";
  import * as api from "../api";
  import type { PreviewRef } from "../bindings/PreviewRef";
  import PdfViewer from "./PdfViewer.svelte";
  import Spinner from "./Spinner.svelte";
  import ErrorPanel from "./ErrorPanel.svelte";
  import EmptyState from "./EmptyState.svelte";

  let { chatJid, messageId, filename }: { chatJid: string; messageId: string; filename: string } = $props();

  let preview = $state<PreviewRef | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let text = $state<string | null>(null);
  let seq = 0;

  async function load(c: string, m: string) {
    const my = ++seq;
    loading = true;
    error = null;
    preview = null;
    text = null;
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

<div class="flex h-full min-h-0 flex-col">
  {#if loading}
    <div class="flex flex-1 items-center justify-center gap-2 text-sm text-muted"><Spinner /> Preparing preview…</div>
  {:else if error}
    <ErrorPanel {error} reset={() => void load(chatJid, messageId)} title="Preview unavailable" />
  {:else if preview}
    {#if preview.note}<p class="border-b border-border bg-info-soft px-3 py-1.5 text-xs text-info">{preview.note}</p>{/if}
    {#if preview.kind === "pdf" && preview.url}
      <div class="min-h-0 flex-1"><PdfViewer url={preview.url} title={filename} /></div>
    {:else if preview.kind === "image" && preview.url}
      <div class="flex min-h-0 flex-1 items-center justify-center bg-surface-3/60 p-4"><img src={preview.url} alt={filename} class="max-h-full max-w-full" /></div>
    {:else if preview.kind === "video" && preview.url}
      <!-- svelte-ignore a11y_media_has_caption -->
      <div class="flex min-h-0 flex-1 items-center justify-center bg-black"><video src={preview.url} controls class="max-h-full max-w-full"></video></div>
    {:else if preview.kind === "audio" && preview.url}
      <div class="flex flex-1 items-center justify-center p-6"><audio src={preview.url} controls></audio></div>
    {:else if preview.kind === "text"}
      <pre class="min-h-0 flex-1 overflow-auto whitespace-pre-wrap p-4 font-mono text-xs">{text ?? ""}</pre>
    {:else}
      <EmptyState icon={FileQuestion} title="No preview for this file" message={preview.note ?? "Open it externally or save a copy."} />
    {/if}
  {/if}
</div>
