<script lang="ts">
  import { FileText, FileSpreadsheet, Presentation, FileArchive, File, FileType } from "@lucide/svelte";
  import { docType, extOf } from "../utils/text";

  let { filename, mime = null, size = 36 }: { filename: string | null; mime?: string | null; size?: number } = $props();
  const t = $derived(docType(filename, mime));
  const map = {
    pdf: { icon: FileType, color: "#e5484d" },
    word: { icon: FileText, color: "#3b82f6" },
    sheet: { icon: FileSpreadsheet, color: "#16a34a" },
    slides: { icon: Presentation, color: "#f97316" },
    archive: { icon: FileArchive, color: "#a16207" },
    text: { icon: FileText, color: "#64748b" },
    other: { icon: File, color: "#64748b" },
  } as const;
  const entry = $derived(map[t]);
  const ext = $derived(extOf(filename).toUpperCase().slice(0, 4));
</script>

<div class="relative flex shrink-0 items-center justify-center rounded-md" style="width:{size}px;height:{size}px;background:color-mix(in srgb, {entry.color} 14%, transparent);color:{entry.color}" aria-hidden="true">
  <entry.icon size={Math.round(size * 0.55)} />
  {#if ext && size >= 32}
    <span class="absolute -bottom-1 rounded px-1 text-[8px] font-bold leading-3 text-white" style="background:{entry.color}">{ext}</span>
  {/if}
</div>
