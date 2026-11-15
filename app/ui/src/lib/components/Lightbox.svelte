<!-- Full-screen media viewer with zoom/pan and keyboard navigation. -->
<script lang="ts" module>
  export interface LightboxItem {
    key: string;
    url: string;
    kind: "image" | "video" | "sticker";
    title?: string;
    subtitle?: string;
    poster?: string | null;
  }
</script>

<script lang="ts">
  import { ChevronLeft, ChevronRight, X, ZoomIn, ZoomOut, Maximize, ExternalLink } from "@lucide/svelte";
  import { trapFocus } from "./overlay";
  import { clamp } from "../utils/misc";

  interface Props {
    items: LightboxItem[];
    index: number;
    onclose: () => void;
    onopen?: (item: LightboxItem) => void;
  }
  let { items, index = $bindable(), onclose, onopen }: Props = $props();

  let scale = $state(1);
  let tx = $state(0);
  let ty = $state(0);
  let drag: { x: number; y: number; tx: number; ty: number } | null = null;

  const item = $derived(items[index]);

  function reset() {
    scale = 1;
    tx = 0;
    ty = 0;
  }
  function go(d: number) {
    if (items.length === 0) return;
    index = (index + d + items.length) % items.length;
    reset();
  }
  function zoom(f: number) {
    scale = clamp(scale * f, 0.5, 8);
    if (scale <= 1) {
      tx = 0;
      ty = 0;
    }
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowLeft") go(-1);
    else if (e.key === "ArrowRight") go(1);
    else if (e.key === "+" || e.key === "=") zoom(1.25);
    else if (e.key === "-") zoom(0.8);
    else if (e.key === "0") reset();
    else return;
    e.preventDefault();
  }
  function onWheel(e: WheelEvent) {
    if (item?.kind === "video") return;
    e.preventDefault();
    zoom(e.deltaY < 0 ? 1.15 : 0.87);
  }
  function down(e: PointerEvent) {
    if (scale <= 1) return;
    drag = { x: e.clientX, y: e.clientY, tx, ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    tx = drag.tx + (e.clientX - drag.x);
    ty = drag.ty + (e.clientY - drag.y);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="fixed inset-0 z-[60] flex flex-col bg-black/90 text-white anim-in"
  role="dialog"
  aria-modal="true"
  aria-label="Media viewer"
  tabindex="-1"
  use:trapFocus={onclose}
  onkeydown={onKey}
  data-testid="lightbox"
>
  <header class="flex items-center gap-2 px-4 py-2">
    <div class="min-w-0 flex-1">
      <p class="truncate text-sm font-medium">{item?.title ?? ""}</p>
      {#if item?.subtitle}<p class="truncate text-xs text-white/60">{item.subtitle}</p>{/if}
    </div>
    <span class="text-xs text-white/60" aria-live="polite">{index + 1} / {items.length}</span>
    {#if item?.kind !== "video"}
      <button class="icon-btn text-white/80 hover:bg-white/10 hover:text-white" aria-label="Zoom out" onclick={() => zoom(0.8)}><ZoomOut size={18} /></button>
      <span class="w-12 text-center text-xs tabular-nums" data-testid="zoom-level">{Math.round(scale * 100)}%</span>
      <button class="icon-btn text-white/80 hover:bg-white/10 hover:text-white" aria-label="Zoom in" onclick={() => zoom(1.25)}><ZoomIn size={18} /></button>
      <button class="icon-btn text-white/80 hover:bg-white/10 hover:text-white" aria-label="Reset zoom" onclick={reset}><Maximize size={18} /></button>
    {/if}
    {#if onopen && item}
      <button class="icon-btn text-white/80 hover:bg-white/10 hover:text-white" aria-label="Open externally" onclick={() => item && onopen?.(item)}><ExternalLink size={18} /></button>
    {/if}
    <button class="icon-btn text-white/80 hover:bg-white/10 hover:text-white" aria-label="Close viewer" data-autofocus onclick={onclose}><X size={20} /></button>
  </header>
  <div class="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden" onwheel={onWheel} role="presentation">
    {#if items.length > 1}
      <button class="icon-btn absolute left-3 z-10 h-11 w-11 rounded-full bg-white/10 text-white hover:bg-white/20" aria-label="Previous" onclick={() => go(-1)}><ChevronLeft size={22} /></button>
      <button class="icon-btn absolute right-3 z-10 h-11 w-11 rounded-full bg-white/10 text-white hover:bg-white/20" aria-label="Next" onclick={() => go(1)}><ChevronRight size={22} /></button>
    {/if}
    {#if item}
      {#key item.key}
        {#if item.kind === "video"}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video src={item.url} poster={item.poster ?? undefined} controls autoplay class="max-h-full max-w-full"></video>
        {:else}
          <img
            src={item.url}
            alt={item.title ?? "Image"}
            class="max-h-full max-w-full select-none {drag ? '' : 'transition-transform duration-100'} {scale > 1 ? 'cursor-grab' : ''}"
            style="transform: translate({tx}px, {ty}px) scale({scale})"
            draggable="false"
            onpointerdown={down}
            onpointermove={move}
            onpointerup={() => (drag = null)}
            ondblclick={() => (scale > 1 ? reset() : zoom(2))}
          />
        {/if}
      {/key}
    {/if}
  </div>
</div>
