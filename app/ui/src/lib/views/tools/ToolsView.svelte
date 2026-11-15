<script lang="ts">
  import { FileUp, Images, Play, FolderOpen, X, Wrench } from "@lucide/svelte";
  import * as api from "../../api";
  import type { ConvertInput } from "../../bindings/ConvertInput";
  import type { ConvertKind } from "../../bindings/ConvertKind";
  import type { ImageFormat } from "../../bindings/ImageFormat";
  import type { MediaItem } from "../../bindings/MediaItem";
  import type { MediaKind } from "../../bindings/MediaKind";
  import JobCard from "../../components/JobCard.svelte";
  import Modal from "../../components/Modal.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import DocIcon from "../../components/DocIcon.svelte";
  import { FILTERS, pickFile, pickFolder, type FileFilter } from "../../platform";
  import { app } from "../../state/app.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { baseName } from "../../utils/text";
  import { shortDate, formatSize } from "../../utils/format";

  type KindId = ConvertKind["type"];
  const KINDS: { id: KindId; label: string; hint: string; filter: FileFilter[]; media: MediaKind[] }[] = [
    { id: "office_to_pdf", label: "Office → PDF", hint: "Word, Excel, PowerPoint, ODF", filter: FILTERS.office, media: ["document"] },
    { id: "image_compress", label: "Compress image", hint: "Resize and re-encode photos", filter: FILTERS.image, media: ["image"] },
    { id: "video_compress", label: "Compress video", hint: "H.264 with CRF and max height", filter: FILTERS.video, media: ["video"] },
    { id: "pdf_compress", label: "Compress PDF", hint: "Ghostscript quality presets", filter: FILTERS.pdf, media: ["document"] },
    { id: "audio_to_opus", label: "Audio → Opus", hint: "Small, voice-friendly audio", filter: FILTERS.audio, media: ["audio"] },
    { id: "pdf_to_images", label: "PDF → images", hint: "One PNG per page", filter: FILTERS.pdf, media: ["document"] },
  ];

  let kindId = $state<KindId>("office_to_pdf");
  let quality = $state(80);
  let maxDim = $state(2048);
  let format = $state<ImageFormat>("jpeg");
  let crf = $state(28);
  let maxHeight = $state(720);
  let level = $state("ebook");
  let bitrate = $state(32);
  let dpi = $state(150);
  let input = $state<{ value: ConvertInput; label: string } | null>(null);
  let outputDir = $state<string | null>(null);
  let starting = $state(false);
  let lastJobId = $state<number | null>(null);
  let picking = $state(false);
  let pickItems = $state<MediaItem[]>([]);
  let pickLoading = $state(false);

  const kindDef = $derived(KINDS.find((k) => k.id === kindId) ?? KINDS[0]!);
  const currentJob = $derived(lastJobId !== null ? app.jobs.find((j) => j.id === lastJobId) ?? null : null);
  const history = $derived(app.jobs.filter((j) => j.spec.type === "convert" && j.id !== lastJobId));

  // Prefill from Docs/Media "Convert".
  $effect(() => {
    const t = app.toolsInput;
    if (t) {
      input = { value: { type: "message", chat_jid: t.chatJid, message_id: t.messageId }, label: t.label };
      if (/\.pdf$/i.test(t.label)) kindId = "pdf_compress";
      app.toolsInput = null;
    }
  });

  function buildKind(): ConvertKind {
    switch (kindId) {
      case "office_to_pdf": return { type: "office_to_pdf" };
      case "image_compress": return { type: "image_compress", quality, max_dim: maxDim, format };
      case "video_compress": return { type: "video_compress", crf, max_height: maxHeight };
      case "pdf_compress": return { type: "pdf_compress", level };
      case "audio_to_opus": return { type: "audio_to_opus", bitrate_kbps: bitrate };
      case "pdf_to_images": return { type: "pdf_to_images", dpi };
    }
  }

  async function chooseFile() {
    try {
      const p = await pickFile("Choose a file to convert", kindDef.filter);
      if (p) input = { value: { type: "file", path: p }, label: baseName(p) };
    } catch (e) {
      toasts.error(e);
    }
  }

  async function chooseDir() {
    try {
      const p = await pickFolder("Output folder");
      if (p) outputDir = p;
    } catch (e) {
      toasts.error(e);
    }
  }

  async function openPicker() {
    picking = true;
    pickLoading = true;
    try {
      const page = await api.listMedia({ chat_jid: null, kinds: kindDef.media, text: null, after: null, before: null, available: true, cursor: null, limit: 80 });
      pickItems = page.items;
    } catch (e) {
      toasts.error(e);
    } finally {
      pickLoading = false;
    }
  }

  async function start() {
    if (!input) return;
    starting = true;
    try {
      const job = await api.startJob({ type: "convert", input: $state.snapshot(input.value), kind: buildKind(), output_dir: outputDir });
      app.upsertJob(job);
      lastJobId = job.id;
    } catch (e) {
      toasts.error(e, "Couldn't start conversion");
    } finally {
      starting = false;
    }
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto grid max-w-6xl gap-5 p-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
    <section class="card space-y-5 p-5" aria-labelledby="conv-title">
      <div>
        <h2 id="conv-title" class="text-[15px] font-semibold">Convert a file</h2>
        <p class="text-xs text-muted">Outputs go to the exports folder unless you pick another.</p>
      </div>

      <div>
        <span class="label">Conversion</span>
        <div class="grid grid-cols-2 gap-2 sm:grid-cols-3" role="radiogroup" aria-label="Conversion kind">
          {#each KINDS as k (k.id)}
            <button role="radio" aria-checked={kindId === k.id} class="rounded-[var(--radius)] border p-2.5 text-left transition-colors {kindId === k.id ? 'border-accent bg-accent-soft' : 'border-border hover:bg-surface-2'}" onclick={() => (kindId = k.id)}>
              <span class="block text-[13px] font-medium">{k.label}</span>
              <span class="block text-[11px] text-muted">{k.hint}</span>
            </button>
          {/each}
        </div>
      </div>

      <div>
        <span class="label">Input</span>
        {#if input}
          <div class="flex items-center gap-2 rounded-[var(--radius)] border border-border bg-surface-2 px-3 py-2" data-testid="convert-input">
            <DocIcon filename={input.label} size={26} />
            <div class="min-w-0 flex-1">
              <p class="truncate text-[13px] font-medium">{input.label}</p>
              <p class="truncate text-[11px] text-muted">{input.value.type === "file" ? input.value.path : "From a WhatsApp message"}</p>
            </div>
            <button class="icon-btn h-7 w-7" aria-label="Clear input" onclick={() => (input = null)}><X size={14} /></button>
          </div>
        {:else}
          <div class="flex gap-2">
            <button class="btn" onclick={chooseFile} data-testid="choose-file"><FileUp size={15} /> Choose file…</button>
            <button class="btn" onclick={openPicker}><Images size={15} /> Pick from WhatsApp media</button>
          </div>
        {/if}
      </div>

      <fieldset class="grid grid-cols-2 gap-3">
        <legend class="label col-span-2">Options</legend>
        {#if kindId === "office_to_pdf"}
          <p class="col-span-2 text-xs text-muted">No options — converted with LibreOffice in headless mode.</p>
        {:else if kindId === "image_compress"}
          <label class="text-xs">Quality ({quality})<input type="range" min="10" max="100" bind:value={quality} class="mt-1 w-full accent-[var(--accent)]" /></label>
          <label class="text-xs">Max dimension (px)<input type="number" min="256" max="8192" step="64" bind:value={maxDim} class="input mt-1" /></label>
          <label class="text-xs">Format
            <select class="input mt-1" bind:value={format}><option value="jpeg">JPEG</option><option value="webp">WebP</option><option value="png">PNG</option></select>
          </label>
        {:else if kindId === "video_compress"}
          <label class="text-xs">CRF ({crf}) — lower is better quality<input type="range" min="18" max="40" bind:value={crf} class="mt-1 w-full accent-[var(--accent)]" /></label>
          <label class="text-xs">Max height
            <select class="input mt-1" bind:value={maxHeight}>{#each [360, 480, 720, 1080] as h (h)}<option value={h}>{h}p</option>{/each}</select>
          </label>
        {:else if kindId === "pdf_compress"}
          <label class="text-xs">Level
            <select class="input mt-1" bind:value={level}>
              <option value="screen">Screen (smallest)</option><option value="ebook">eBook (balanced)</option><option value="printer">Printer (high)</option><option value="prepress">Prepress (highest)</option>
            </select>
          </label>
        {:else if kindId === "audio_to_opus"}
          <label class="text-xs">Bitrate (kbps)
            <select class="input mt-1" bind:value={bitrate}>{#each [16, 24, 32, 48, 64, 96] as b (b)}<option value={b}>{b}</option>{/each}</select>
          </label>
        {:else if kindId === "pdf_to_images"}
          <label class="text-xs">DPI
            <select class="input mt-1" bind:value={dpi}>{#each [72, 100, 150, 200, 300] as d (d)}<option value={d}>{d}</option>{/each}</select>
          </label>
        {/if}
      </fieldset>

      <div>
        <span class="label">Output folder</span>
        <div class="flex items-center gap-2">
          <span class="min-w-0 flex-1 truncate text-[13px] {outputDir ? '' : 'text-muted'}">{outputDir ?? "Exports folder (default)"}</span>
          <button class="btn btn-sm" onclick={chooseDir}><FolderOpen size={13} /> Choose…</button>
          {#if outputDir}<button class="btn btn-sm btn-ghost" onclick={() => (outputDir = null)}>Reset</button>{/if}
        </div>
      </div>

      <button class="btn btn-primary w-full" disabled={!input || starting} onclick={start} data-testid="start-convert">
        {#if starting}<Spinner size={14} />{:else}<Play size={15} />{/if} Start conversion
      </button>
      {#if currentJob}<JobCard job={currentJob} />{/if}
    </section>

    <section aria-labelledby="hist-title" class="space-y-3">
      <h2 id="hist-title" class="section-title">History</h2>
      {#each history as j (j.id)}
        <JobCard job={j} />
      {:else}
        <div class="card"><EmptyState icon={Wrench} title="No conversions yet" /></div>
      {/each}
    </section>
  </div>
</div>

{#if picking}
  <Modal title="Pick WhatsApp media" onclose={() => (picking = false)} width="40rem">
    {#if pickLoading}
      <div class="flex justify-center p-6"><Spinner /></div>
    {:else if pickItems.length === 0}
      <EmptyState title="No downloaded media of this kind" />
    {:else}
      <ul class="max-h-[55vh] space-y-1 overflow-y-auto">
        {#each pickItems as it (it.chat_jid + it.message_id)}
          <li>
            <button
              class="flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left hover:bg-surface-2"
              onclick={() => {
                input = { value: { type: "message", chat_jid: it.chat_jid, message_id: it.message_id }, label: it.media.filename ?? it.media.kind };
                picking = false;
              }}
            >
              {#if it.media.thumb_url}<img src={it.media.thumb_url} alt="" class="h-9 w-9 rounded object-cover" />{:else}<DocIcon filename={it.media.filename} size={36} />{/if}
              <div class="min-w-0 flex-1">
                <p class="truncate text-[13px]">{it.media.filename}</p>
                <p class="truncate text-[11px] text-muted">{it.chat_name} · {shortDate(it.ts)} · {formatSize(it.media.size)}</p>
              </div>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </Modal>
{/if}
