<!-- Export a chat (or the selected messages): format, date range, include media. -->
<script lang="ts">
  import { FolderOpen, ExternalLink, FileDown } from "@lucide/svelte";
  import * as api from "../api";
  import type { ExportFormat } from "../bindings/ExportFormat";
  import Modal from "./Modal.svelte";
  import Spinner from "./Spinner.svelte";
  import { openLocalPath, revealPath } from "../platform";
  import { app } from "../state/app.svelte";
  import { toasts } from "../state/toasts.svelte";
  import { fromDateInput } from "../utils/format";
  import { baseName } from "../utils/text";

  let { chatJid, chatName, messageIds }: { chatJid: string; chatName: string; messageIds: string[] } = $props();

  const FORMATS: { id: ExportFormat; label: string; hint: string }[] = [
    { id: "html", label: "HTML", hint: "Readable page, optional media" },
    { id: "txt", label: "Text", hint: "WhatsApp-style lines" },
    { id: "json", label: "JSON", hint: "Structured data" },
    { id: "pdf", label: "PDF", hint: "Printable transcript" },
  ];
  let format = $state<ExportFormat>("html");
  let from = $state("");
  let to = $state("");
  let includeMedia = $state(false);
  let busy = $state(false);
  let result = $state<string | null>(null);
  const selection = $derived(messageIds.length > 0);

  async function run() {
    busy = true;
    try {
      result = await api.exportChat({
        chat_jid: chatJid,
        format,
        message_ids: $state.snapshot(messageIds),
        after: !selection && from ? fromDateInput(from) : null,
        before: !selection && to ? fromDateInput(to, true) : null,
        include_media: format === "html" && includeMedia,
      });
    } catch (e) {
      toasts.error(e, "Export failed");
    } finally {
      busy = false;
    }
  }
  const close = () => (app.exportDialog = null);
  const act = (fn: (p: string) => Promise<void>) => result && fn(result).catch((e) => toasts.error(e));
</script>

<Modal title={selection ? `Export ${messageIds.length} message${messageIds.length === 1 ? "" : "s"}` : "Export chat"} onclose={close} width="30rem">
  <div class="space-y-4" data-testid="export-dialog">
    <p class="text-[13px] text-muted">{selection ? "Selected messages from" : "Chat with"} <strong class="text-fg">{chatName}</strong></p>
    {#if result}
      <div class="rounded-md border border-border bg-surface-2 p-3" data-testid="export-result">
        <p class="flex items-center gap-2 text-[13px] font-medium"><FileDown size={15} class="text-accent" /> Export ready</p>
        <p class="mt-1 break-all font-mono text-[11px] text-muted" title={result}>{baseName(result)}</p>
        <p class="break-all text-[11px] text-faint">{result}</p>
        <div class="mt-2 flex gap-2">
          <button class="btn btn-sm" onclick={() => act(openLocalPath)} data-testid="export-open"><ExternalLink size={13} /> Open</button>
          <button class="btn btn-sm" onclick={() => act(revealPath)} data-testid="export-reveal"><FolderOpen size={13} /> Reveal</button>
        </div>
      </div>
    {:else}
      <div>
        <span class="label">Format</span>
        <div class="grid grid-cols-2 gap-2" role="radiogroup" aria-label="Export format">
          {#each FORMATS as f (f.id)}
            <button
              role="radio"
              aria-checked={format === f.id}
              class="rounded-[var(--radius)] border p-2 text-left {format === f.id ? 'border-accent bg-accent-soft' : 'border-border hover:bg-surface-2'}"
              onclick={() => (format = f.id)}
              data-testid="export-format-{f.id}"
            >
              <span class="block text-[13px] font-medium">{f.label}</span>
              <span class="block text-[11px] text-muted">{f.hint}</span>
            </button>
          {/each}
        </div>
      </div>
      {#if !selection}
        <div class="grid grid-cols-2 gap-3">
          <label class="block"><span class="label">From</span><input class="input" type="date" bind:value={from} data-testid="export-from" /></label>
          <label class="block"><span class="label">To</span><input class="input" type="date" bind:value={to} data-testid="export-to" /></label>
        </div>
      {/if}
      <label class="flex items-center gap-2 text-[13px] {format === 'html' ? '' : 'opacity-50'}">
        <input type="checkbox" bind:checked={includeMedia} disabled={format !== "html"} data-testid="export-media" />
        Include downloaded media (HTML only)
      </label>
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={close}>{result ? "Done" : "Cancel"}</button>
    {#if !result}
      <button class="btn btn-primary" onclick={run} disabled={busy} data-testid="export-run">
        {#if busy}<Spinner size={14} label="Exporting" />{/if} Export
      </button>
    {/if}
  {/snippet}
</Modal>
