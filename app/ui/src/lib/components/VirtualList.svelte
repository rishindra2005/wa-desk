<!--
  Variable-height virtual list.
  - Rows are measured with a shared ResizeObserver; unknown rows use `estimate`.
  - Scroll position is anchored to the first visible row's key, so prepending
    rows (older messages) or re-measuring rows above the viewport never jumps.
  - `stickToBottom` keeps the view pinned to the end while the user is there.
-->
<script lang="ts" generics="T">
  import type { Snippet } from "svelte";
  import { onDestroy, untrack } from "svelte";
  import type { VirtualListApi } from "./virtual";

  interface Props {
    items: T[];
    getKey: (item: T, index: number) => string;
    estimate?: number | ((item: T) => number);
    overscan?: number;
    row: Snippet<[T, number]>;
    stickToBottom?: boolean;
    /** Start scrolled to the bottom on first non-empty render. */
    startAtBottom?: boolean;
    onNearTop?: () => void;
    onNearBottom?: () => void;
    onScroll?: (atBottom: boolean) => void;
    nearPx?: number;
    /** Rows usable as scroll anchors (default: all). Unstable rows like day separators should return false. */
    canAnchor?: (item: T) => boolean;
    class?: string;
    innerClass?: string;
    role?: string;
    ariaLabel?: string;
    api?: VirtualListApi | undefined;
    footer?: Snippet;
  }

  let {
    items,
    getKey,
    estimate = 56,
    overscan = 500,
    row,
    stickToBottom = false,
    startAtBottom = false,
    onNearTop,
    onNearBottom,
    onScroll,
    nearPx = 300,
    canAnchor,
    class: cls = "",
    innerClass = "",
    role = "list",
    ariaLabel,
    api = $bindable(),
    footer,
  }: Props = $props();

  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewportH = $state(0);
  let measureVersion = $state(0);
  const heights = new Map<string, number>();
  let anchor: { key: string; delta: number } | null = null;
  let atBottom = true;
  let didInitialScroll = false;
  let programmatic = false;

  const est = (item: T) => (typeof estimate === "function" ? estimate(item) : estimate);

  const keys = $derived(items.map((it, i) => getKey(it, i)));
  const indexByKey = $derived.by(() => {
    const m = new Map<string, number>();
    keys.forEach((k, i) => m.set(k, i));
    return m;
  });
  const offsets = $derived.by(() => {
    void measureVersion;
    const offs = new Float64Array(items.length + 1);
    for (let i = 0; i < items.length; i++) {
      const k = keys[i] as string;
      offs[i + 1] = (offs[i] ?? 0) + (heights.get(k) ?? est(items[i] as T));
    }
    return offs;
  });
  const total = $derived(offsets[items.length] ?? 0);

  function findIndex(y: number): number {
    let lo = 0;
    let hi = items.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if ((offsets[mid] ?? 0) <= y) lo = mid;
      else hi = mid - 1;
    }
    return Math.max(0, lo);
  }

  const range = $derived.by(() => {
    if (items.length === 0) return { start: 0, end: -1 };
    const h = viewportH || 800;
    const start = findIndex(Math.max(0, scrollTop - overscan));
    const end = findIndex(scrollTop + h + overscan);
    return { start, end: Math.min(items.length - 1, end) };
  });

  const visible = $derived.by(() => {
    const out: { item: T; index: number; key: string; top: number }[] = [];
    for (let i = range.start; i <= range.end; i++) {
      out.push({ item: items[i] as T, index: i, key: keys[i] as string, top: offsets[i] ?? 0 });
    }
    return out;
  });

  function recordAnchor() {
    if (!viewport || items.length === 0) return;
    const st = viewport.scrollTop;
    let i = findIndex(st);
    if (canAnchor) {
      while (i < items.length - 1 && !canAnchor(items[i] as T)) i++;
    }
    const k = keys[i];
    if (k !== undefined) anchor = { key: k, delta: st - (offsets[i] ?? 0) };
    atBottom = viewport.scrollHeight - st - viewport.clientHeight < 32;
  }

  function setScroll(y: number) {
    if (!viewport) return;
    programmatic = true;
    viewport.scrollTop = y;
    scrollTop = viewport.scrollTop;
  }

  function handleScroll() {
    if (!viewport) return;
    scrollTop = viewport.scrollTop;
    if (programmatic) {
      programmatic = false;
    }
    recordAnchor();
    onScroll?.(atBottom);
    if (scrollTop < nearPx) onNearTop?.();
    if (viewport.scrollHeight - scrollTop - viewport.clientHeight < nearPx) onNearBottom?.();
  }

  // After every layout change, restore the anchor (or stick to the bottom).
  $effect(() => {
    void offsets;
    void total;
    const vp = viewport;
    if (!vp) return;
    untrack(() => {
      if (items.length === 0) {
        didInitialScroll = false;
        anchor = null;
        return;
      }
      if (!didInitialScroll) {
        didInitialScroll = true;
        if (startAtBottom) {
          setScroll(total);
          atBottom = true;
        }
        recordAnchor();
        return;
      }
      if (stickToBottom && atBottom) {
        setScroll(total);
        return;
      }
      if (anchor) {
        const i = indexByKey.get(anchor.key);
        if (i !== undefined) {
          const target = (offsets[i] ?? 0) + anchor.delta;
          if (Math.abs(vp.scrollTop - target) > 0.5) setScroll(target);
        }
      }
      recordAnchor();
    });
  });

  // Shared resize observer for rows + viewport.
  let flushQueued = false;
  const ro =
    typeof ResizeObserver !== "undefined"
      ? new ResizeObserver((entries) => {
          let changed = false;
          for (const e of entries) {
            const el = e.target as HTMLElement;
            if (el === viewport) {
              viewportH = el.clientHeight;
              continue;
            }
            const key = el.dataset.vkey;
            if (!key) continue;
            const h = el.offsetHeight;
            if (h > 0 && heights.get(key) !== h) {
              heights.set(key, h);
              changed = true;
            }
          }
          if (changed && !flushQueued) {
            flushQueued = true;
            queueMicrotask(() => {
              flushQueued = false;
              measureVersion++;
            });
          }
        })
      : null;

  function measure(node: HTMLElement) {
    const h = node.offsetHeight;
    const key = node.dataset.vkey;
    if (key && h > 0 && heights.get(key) !== h) {
      heights.set(key, h);
      measureVersion++;
    }
    ro?.observe(node);
    return { destroy: () => ro?.unobserve(node) };
  }

  $effect(() => {
    if (!viewport) return;
    viewportH = viewport.clientHeight;
    ro?.observe(viewport);
    const vp = viewport;
    return () => ro?.unobserve(vp);
  });

  onDestroy(() => ro?.disconnect());

  api = {
    scrollToKey(key, align = "center") {
      const i = indexByKey.get(key);
      if (i === undefined || !viewport) return false;
      const h = heights.get(key) ?? est(items[i] as T);
      const vh = viewport.clientHeight || viewportH;
      const top = offsets[i] ?? 0;
      const y = align === "start" ? top : align === "end" ? top - vh + h : top - vh / 2 + h / 2;
      setScroll(Math.max(0, y));
      anchor = { key, delta: viewport.scrollTop - top };
      atBottom = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < 32;
      return true;
    },
    scrollToBottom() {
      setScroll(total + 10_000);
      atBottom = true;
      recordAnchor();
    },
    scrollToTop() {
      setScroll(0);
      recordAnchor();
    },
    isAtBottom: () => atBottom,
  };
</script>

<div
  bind:this={viewport}
  class="relative overflow-y-auto overflow-x-hidden {cls}"
  onscroll={handleScroll}
  {role}
  aria-label={ariaLabel}
>
  <div class="relative w-full {innerClass}" style="height: {total}px">
    {#each visible as v (v.key)}
      <div class="absolute left-0 right-0 top-0" style="transform: translateY({v.top}px)" data-vkey={v.key} use:measure>
        {@render row(v.item, v.index)}
      </div>
    {/each}
  </div>
  {@render footer?.()}
</div>
