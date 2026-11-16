<!-- "Convert" button + popover for a message's media: quick presets, "More options", inline job progress and outputs. -->
<script lang="ts">
  import { Wand2, ChevronDown, SlidersHorizontal } from "@lucide/svelte";
  import * as api from "../api";
  import type { ConvertKind } from "../bindings/ConvertKind";
  import type { MediaKind } from "../bindings/MediaKind";
  import JobCard from "./JobCard.svelte";
  import Spinner from "./Spinner.svelte";
  import { app } from "../state/app.svelte";
  import { toasts } from "../state/toasts.svelte";
  import { convertOptions, convertParams } from "../utils/convert";

  interface Props {
    chatJid: string;
    messageId: string;
    kind: MediaKind;
    filename: string | null;
    /** Dark surface (lightbox). */
    dark?: boolean;
    /** Render the panel in place (inside a dialog) instead of as a popover. */
    inline?: boolean;
  }
  let { chatJid, messageId, kind, filename, dark = false, inline = false }: Props = $props();

  const options = $derived(convertOptions(kind, filename));
  let popOpen = $state(false);
  const open = $derived(inline || popOpen);
  let selected = $state("");
  let advanced = $state(false);
  let params = $state<Record<string, number>>({});
  let starting = $state(false);
  let jobId = $state<number | null>(null);
  let root: HTMLDivElement | undefined = $state();

  const current = $derived(options.find((o) => o.id === selected) ?? options[0]);
  const job = $derived(jobId !== null ? (app.jobs.find((j) => j.id === jobId) ?? null) : null);
  const fields = $derived(current ? convertParams(current.kind) : []);

  $effect(() => {
    // Reset when the target media changes.
    void messageId;
    jobId = null;
    selected = options[0]?.id ?? "";
    advanced = false;
    popOpen = false;
  });
  $effect(() => {
    void current;
    params = {};
  });

  function finalKind(): ConvertKind {
    const base = current?.kind;
    if (!base) throw new Error("No conversion selected");
    return { ...base, ...params } as ConvertKind;
  }

  async function start() {
    starting = true;
    try {
      const j = await api.startJob({
        type: "convert",
        input: { type: "message", chat_jid: chatJid, message_id: messageId },
        kind: finalKind(),
        output_dir: null,
      });
      app.upsertJob(j);
      jobId = j.id;
    } catch (e) {
      toasts.error(e, "Couldn't start conversion");
    } finally {
      starting = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && popOpen) {
      e.stopPropagation();
      popOpen = false;
    }
  }
  function onDown(e: PointerEvent) {
    if (popOpen && root && !root.contains(e.target as Node)) popOpen = false;
  }
</script>

<svelte:window onpointerdown={onDown} />

{#if options.length > 0}
  <div class="relative" bind:this={root} onkeydown={onKey} role="presentation">
    {#if !inline}<button
      class="btn btn-sm {dark ? 'border-white/20 bg-white/10 text-white hover:bg-white/20' : ''}"
      aria-haspopup="dialog"
      aria-expanded={open}
      onclick={() => (popOpen = !popOpen)}
      data-testid="convert-btn"
    >
      <Wand2 size={13} /> Convert <ChevronDown size={12} />
    </button>{/if}
    {#if open}
      <div class={inline ? "" : "anim-in absolute right-0 top-full z-[80] mt-1 w-80 rounded-[10px] border border-border bg-surface p-3 text-fg shadow-[var(--shadow-lg)]"} role="dialog" aria-label="Convert" data-testid="convert-panel">
        {#if job}
          <JobCard {job} />
          <button class="btn btn-sm mt-2" onclick={() => (jobId = null)}>Convert again</button>
        {:else}
          <div class="space-y-1" role="radiogroup" aria-label="Conversion">
            {#each options as o (o.id)}
              <button
                role="radio"
                aria-checked={current?.id === o.id}
                class="block w-full rounded-md border px-2.5 py-1.5 text-left {current?.id === o.id ? 'border-accent bg-accent-soft' : 'border-transparent hover:bg-surface-2'}"
                onclick={() => (selected = o.id)}
                data-testid="convert-opt-{o.id}"
              >
                <span class="block text-[13px] font-medium">{o.label}</span>
                <span class="block text-[11px] text-muted">{o.hint}</span>
              </button>
            {/each}
          </div>
          {#if fields.length}
            <button class="mt-2 flex items-center gap-1 text-xs text-muted hover:text-fg" aria-expanded={advanced} onclick={() => (advanced = !advanced)} data-testid="convert-more">
              <SlidersHorizontal size={12} /> More options
            </button>
            {#if advanced && current}
              <div class="mt-2 space-y-2">
                {#each fields as f (f.key)}
                  {@const base = (current.kind as unknown as Record<string, number>)[f.key] ?? f.min}
                  <label class="block text-xs">
                    <span class="flex justify-between text-muted"><span>{f.label}</span><span class="tabular-nums">{params[f.key] ?? base}</span></span>
                    <input type="range" class="w-full accent-[var(--accent)]" min={f.min} max={f.max} step={f.step} value={params[f.key] ?? base} oninput={(e) => (params[f.key] = Number(e.currentTarget.value))} />
                  </label>
                {/each}
              </div>
            {/if}
          {/if}
          <div class="mt-3 flex justify-end">
            <button class="btn btn-primary btn-sm" onclick={start} disabled={starting} data-testid="convert-start">
              {#if starting}<Spinner size={12} label="Starting" />{/if} Start conversion
            </button>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}
