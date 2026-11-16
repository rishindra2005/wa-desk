<!--
  PDF viewer on pdf.js (legacy build for WebKitGTK). Pages are virtualised:
  only pages near the viewport get a canvas; others are sized placeholders.
  The worker is a bundled file (CSP: worker-src 'self' blob:).
-->
<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { ZoomIn, ZoomOut, ChevronUp, ChevronDown, MoveHorizontal } from "@lucide/svelte";
  import type { PDFDocumentLoadingTask, PDFDocumentProxy, RenderTask, TextLayer } from "pdfjs-dist/legacy/build/pdf.mjs";
  import { getPdfjs } from "./pdfjs";
  import Spinner from "./Spinner.svelte";
  import { clamp } from "../utils/misc";

  let { url, title = "Document", placeholder = null }: { url: string; title?: string; placeholder?: string | null } = $props();

  const CSS_UNITS = 96 / 72;
  let doc: PDFDocumentProxy | null = null;
  let docTask: PDFDocumentLoadingTask | null = null;
  let pageSizes = $state<{ w: number; h: number }[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let scale = $state(1);
  let current = $state(1);
  let pageInput = $state("1");
  let container: HTMLDivElement | undefined = $state();
  let near = $state<Set<number>>(new Set());
  let loadSeq = 0;

  async function load(src: string) {
    const seq = ++loadSeq;
    loading = true;
    error = null;
    pageSizes = [];
    near = new Set();
    try {
      const { pdfjs, worker } = await getPdfjs();
      const task = pdfjs.getDocument({ url: src, worker });
      const d = await task.promise;
      if (seq !== loadSeq) {
        void task.destroy();
        return;
      }
      await docTask?.destroy();
      docTask = task;
      doc = d;
      const first = await d.getPage(1);
      const vp = first.getViewport({ scale: 1 });
      pageSizes = Array.from({ length: d.numPages }, () => ({ w: vp.width, h: vp.height }));
      fitWidth();
      near = new Set([1]); // render page 1 immediately, without waiting for the observer
      loading = false;
    } catch (e) {
      if (seq !== loadSeq) return;
      error = e instanceof Error ? e.message : String(e);
      loading = false;
    }
  }

  $effect(() => {
    const u = url;
    untrack(() => void load(u));
  });

  onDestroy(() => {
    loadSeq++;
    void docTask?.destroy();
    io?.disconnect();
  });

  function fitWidth() {
    const w = container?.clientWidth ?? 0;
    const p = pageSizes[0];
    if (w > 0 && p) scale = clamp((w - 32) / (p.w * CSS_UNITS), 0.25, 3);
  }

  // Track pages near the viewport.
  let io: IntersectionObserver | null = null;
  function observe(node: HTMLElement, n: number) {
    if (typeof IntersectionObserver === "undefined") {
      near = new Set([...near, n]);
      return {};
    }
    io ??= new IntersectionObserver(
      (entries) => {
        const next = new Set(near);
        for (const e of entries) {
          const num = Number((e.target as HTMLElement).dataset.page);
          if (e.isIntersecting) next.add(num);
          else next.delete(num);
        }
        near = next;
      },
      { root: container ?? null, rootMargin: "800px 0px" },
    );
    io.observe(node);
    return { destroy: () => io?.unobserve(node) };
  }

  // Render a page into a canvas; re-render when the scale changes.
  function renderPage(canvas: HTMLCanvasElement, params: { n: number; scale: number }) {
    let task: RenderTask | null = null;
    let alive = true;
    const run = async ({ n, scale: s }: { n: number; scale: number }) => {
      task?.cancel();
      if (!doc) return;
      try {
        const page = await doc.getPage(n);
        if (!alive) return;
        const dpr = window.devicePixelRatio || 1;
        const viewport = page.getViewport({ scale: s * CSS_UNITS * dpr });
        canvas.width = Math.floor(viewport.width);
        canvas.height = Math.floor(viewport.height);
        const base = page.getViewport({ scale: 1 });
        const size = pageSizes[n - 1];
        if (size && (size.w !== base.width || size.h !== base.height)) pageSizes[n - 1] = { w: base.width, h: base.height };
        task = page.render({ canvas, viewport });
        await task.promise;
        canvas.dataset.rendered = "true";
      } catch {
        /* cancelled or failed; placeholder stays */
      }
    };
    void run(params);
    return {
      update: (p: { n: number; scale: number }) => void run(p),
      destroy: () => {
        alive = false;
        task?.cancel();
      },
    };
  }

  // Selectable text on top of the canvas.
  function textLayer(node: HTMLDivElement, params: { n: number; scale: number }) {
    let layer: TextLayer | null = null;
    let alive = true;
    const run = async ({ n, scale: s }: { n: number; scale: number }) => {
      layer?.cancel();
      if (!doc) return;
      try {
        const { pdfjs } = await getPdfjs();
        const page = await doc.getPage(n);
        if (!alive) return;
        node.replaceChildren();
        node.style.setProperty("--total-scale-factor", String(s * CSS_UNITS));
        layer = new pdfjs.TextLayer({
          textContentSource: page.streamTextContent(),
          container: node,
          viewport: page.getViewport({ scale: s * CSS_UNITS }),
        });
        await layer.render();
        node.dataset.ready = "true";
      } catch {
        /* cancelled */
      }
    };
    void run(params);
    return {
      update: (p: { n: number; scale: number }) => void run(p),
      destroy: () => {
        alive = false;
        layer?.cancel();
      },
    };
  }

  function onScroll() {
    if (!container) return;
    const mid = container.scrollTop + 40;
    let y = 16;
    for (let i = 0; i < pageSizes.length; i++) {
      const h = (pageSizes[i]?.h ?? 0) * CSS_UNITS * scale + 16;
      if (mid < y + h) {
        current = i + 1;
        pageInput = String(current);
        return;
      }
      y += h;
    }
  }

  function goTo(n: number) {
    const target = clamp(n, 1, pageSizes.length || 1);
    container?.querySelector(`[data-page="${target}"]`)?.scrollIntoView({ block: "start" });
    current = target;
    pageInput = String(target);
  }

  function setZoom(s: number) {
    const ratio = container ? container.scrollTop / Math.max(1, container.scrollHeight) : 0;
    scale = clamp(s, 0.25, 4);
    requestAnimationFrame(() => {
      if (container) container.scrollTop = ratio * container.scrollHeight;
    });
  }
</script>

<div class="flex h-full min-h-0 flex-col" data-testid="pdf-viewer">
  <div class="flex items-center gap-1 border-b border-border bg-surface px-2 py-1.5">
    <button class="icon-btn h-7 w-7" aria-label="Previous page" disabled={current <= 1} onclick={() => goTo(current - 1)}><ChevronUp size={16} /></button>
    <button class="icon-btn h-7 w-7" aria-label="Next page" disabled={current >= pageSizes.length} onclick={() => goTo(current + 1)}><ChevronDown size={16} /></button>
    <input
      class="input h-7 w-12 px-1 text-center"
      aria-label="Page number"
      bind:value={pageInput}
      onkeydown={(e) => e.key === "Enter" && goTo(Number(pageInput) || 1)}
    />
    <span class="text-xs text-muted">/ {pageSizes.length || "–"}</span>
    <div class="flex-1"></div>
    <button class="icon-btn h-7 w-7" aria-label="Zoom out" onclick={() => setZoom(scale / 1.2)}><ZoomOut size={16} /></button>
    <span class="w-11 text-center text-xs tabular-nums text-muted" data-testid="pdf-zoom">{Math.round(scale * 100)}%</span>
    <button class="icon-btn h-7 w-7" aria-label="Zoom in" onclick={() => setZoom(scale * 1.2)}><ZoomIn size={16} /></button>
    <button class="icon-btn h-7 w-7" aria-label="Fit width" onclick={fitWidth}><MoveHorizontal size={16} /></button>
  </div>
  <div bind:this={container} class="relative min-h-0 flex-1 overflow-auto bg-surface-3/60" onscroll={onScroll} role="document" aria-label={title}>
    {#if loading}
      <div class="relative flex h-full items-center justify-center gap-2 text-sm text-muted">
        {#if placeholder}<img src={placeholder} alt="" aria-hidden="true" class="max-h-full max-w-full object-contain" data-testid="pdf-placeholder" />{/if}
        <span class="late-in {placeholder ? 'absolute' : ''} flex items-center gap-2 rounded-md bg-surface/80 px-2 py-1"><Spinner /> Loading PDF…</span>
      </div>
    {:else if error}
      <div class="flex h-full items-center justify-center p-6 text-center text-sm text-danger" role="alert">Couldn't open this PDF: {error}</div>
    {:else}
      <div class="flex flex-col items-center gap-4 p-4">
        {#each pageSizes as size, i (i)}
          <div
            class="relative shrink-0 bg-white shadow-[var(--shadow)]"
            style="width:{size.w * CSS_UNITS * scale}px;height:{size.h * CSS_UNITS * scale}px"
            data-page={i + 1}
            use:observe={i + 1}
          >
            {#if i === 0 && placeholder}
              <img src={placeholder} alt="" aria-hidden="true" class="absolute inset-0 h-full w-full object-fill" data-testid="pdf-placeholder" />
            {/if}
            {#if near.has(i + 1)}
              <canvas class="relative block h-full w-full" aria-label="Page {i + 1}" use:renderPage={{ n: i + 1, scale }}></canvas>
              <div class="textLayer" data-testid="pdf-text-layer" use:textLayer={{ n: i + 1, scale }}></div>
            {:else}
              <div class="flex h-full items-center justify-center text-xs text-slate-400">Page {i + 1}</div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>
