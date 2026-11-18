<!-- "Download" for several media/document files: one ZIP, or individual files in a folder. -->
<script lang="ts" module>
  import type { MediaTarget } from "../bindings/MediaTarget";

  /** Pick a sensible default zip name. */
  export function zipName(label: string): string {
    const safe = label.replace(/[\\/:*?"<>|]+/g, " ").replace(/\s+/g, " ").trim().slice(0, 60);
    return `${safe || "WhatsApp media"}.zip`;
  }

  export interface ExportItems {
    targets: MediaTarget[];
    /** Used for the default zip file name and the dialog subtitle. */
    label: string;
  }
</script>

<script lang="ts">
  import { Archive, FolderDown, CircleAlert, CircleCheck } from "@lucide/svelte";
  import * as api from "../api";
  import type { MediaExportMode } from "../bindings/MediaExportMode";
  import type { MediaExportProgress } from "../bindings/MediaExportProgress";
  import type { MediaExportSummary } from "../bindings/MediaExportSummary";
  import Modal from "./Modal.svelte";
  import ProgressBar from "./ProgressBar.svelte";
  import { pickFolder, pickSavePath } from "../platform";
  import { toasts } from "../state/toasts.svelte";

  interface Props {
    items: ExportItems;
    onclose: () => void;
    /** Called after an export that saved something (e.g. to refresh downloaded state). */
    ondone?: (s: MediaExportSummary) => void;
  }
  let { items, onclose, ondone }: Props = $props();

  const n = $derived(items.targets.length);
  let running = $state(false);
  let progress = $state<MediaExportProgress | null>(null);
  let summary = $state<MediaExportSummary | null>(null);
  let requestId = "";

  $effect(() => {
    const off = api.subscribe("media-export:progress", (p) => {
      if (p.request_id === requestId) progress = p;
    });
    return off;
  });

  async function run(mode: MediaExportMode) {
    if (running) return;
    try {
      const dest = mode === "zip" ? await pickSavePath(zipName(items.label)) : await pickFolder("Save files to");
      if (!dest) return;
      running = true;
      progress = null;
      requestId = `exp-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      const s = await api.exportMediaFiles({ items: $state.snapshot(items.targets), mode, dest, request_id: requestId });
      summary = s;
      if (s.saved > 0) ondone?.(s);
      if (s.failed === 0) toasts.success(`Saved ${s.saved} file${s.saved === 1 ? "" : "s"}`, s.dest ?? undefined);
    } catch (e) {
      toasts.error(e, "Couldn't save the files");
    } finally {
      running = false;
    }
  }

</script>

<Modal title="Download {n} file{n === 1 ? '' : 's'}" {onclose} width="26rem">
  <div data-testid="export-dialog">
    {#if summary}
      {@const s = summary}
      <div class="space-y-3" data-testid="export-summary" aria-live="polite">
        <p class="flex items-center gap-2 text-[14px] font-medium">
          {#if s.failed === 0}<CircleCheck size={17} class="text-accent-text" />{:else}<CircleAlert size={17} class="text-warn" />{/if}
          <span data-testid="export-result">Saved {s.saved}, failed {s.failed}</span>
        </p>
        {#if s.dest}<p class="break-all text-xs text-muted" data-testid="export-dest">{s.dest}</p>{/if}
        {#if s.failures.length}
          <ul class="max-h-48 space-y-1 overflow-y-auto text-[12.5px] text-danger" data-testid="export-failures">
            {#each s.failures as f (f.chat_jid + f.message_id)}<li>{f.reason}</li>{/each}
          </ul>
        {/if}
      </div>
    {:else}
      <p class="mb-3 text-[13px] text-muted">
        Photos, videos and documents not yet on this computer are downloaded first. Choose how to save {n === 1 ? "it" : "them"}.
      </p>
      {#if running}
        <div class="space-y-1.5" data-testid="export-progress">
          <ProgressBar done={progress?.done ?? 0} total={progress?.total ?? 0} label="Export progress" indeterminate={!progress} />
          <p class="text-xs text-muted" aria-live="polite">{progress?.current ? `Saving ${progress.current}…` : "Preparing…"}</p>
        </div>
      {:else}
        <div class="grid gap-2">
          <button class="btn justify-start" onclick={() => void run("zip")} data-testid="export-zip"><Archive size={15} /> Save as ZIP</button>
          <button class="btn justify-start" onclick={() => void run("folder")} data-testid="export-folder"><FolderDown size={15} /> Save files to folder</button>
        </div>
      {/if}
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose} disabled={running} data-testid="export-close">{summary ? "Done" : "Cancel"}</button>
  {/snippet}
</Modal>
