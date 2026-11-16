<!-- Renders the global context menu: keyboard navigable, closes on Esc / outside click, viewport-clamped. -->
<script lang="ts">
  import { tick } from "svelte";
  import { ChevronRight } from "@lucide/svelte";
  import { ctxMenu } from "../state/menu.svelte";
  import { clampPosition, nextIndex } from "../utils/menu";

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ left: 0, top: 0 });
  let active = $state(-1);

  $effect(() => {
    if (!ctxMenu.isOpen) return;
    const { x, y } = ctxMenu;
    pos = { left: x, top: y };
    active = -1;
    void tick().then(() => {
      if (!el) return;
      const r = el.getBoundingClientRect();
      pos = clampPosition(x, y, r.width, r.height, window.innerWidth, window.innerHeight);
      active = nextIndex(ctxMenu.items, -1, 1);
      el.focus({ preventScroll: true });
    });
  });

  function choose(i: number) {
    const it = ctxMenu.items[i];
    if (!it || it.disabled || it.separator) return;
    if (it.submenu) {
      ctxMenu.enter(it.submenu, it.label ?? "Submenu");
      active = nextIndex(ctxMenu.items, -1, 1);
      el?.focus({ preventScroll: true });
      return;
    }
    ctxMenu.close();
    it.onselect?.();
  }

  function onKey(e: KeyboardEvent) {
    const k = e.key;
    if (k === "Escape") ctxMenu.close();
    else if (k === "ArrowDown") active = nextIndex(ctxMenu.items, active, 1);
    else if (k === "ArrowUp") active = nextIndex(ctxMenu.items, active, -1);
    else if (k === "ArrowRight" && ctxMenu.items[active]?.submenu) choose(active);
    else if (k === "ArrowLeft" && ctxMenu.back()) {
      active = nextIndex(ctxMenu.items, -1, 1);
      el?.focus({ preventScroll: true });
    }
    else if (k === "Home") active = nextIndex(ctxMenu.items, -1, 1);
    else if (k === "End") active = nextIndex(ctxMenu.items, 0, -1);
    else if (k === "Enter" || k === " ") choose(active);
    else if (k === "Tab") ctxMenu.close();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  function onDown(e: PointerEvent) {
    if (ctxMenu.isOpen && el && !el.contains(e.target as Node)) ctxMenu.close(false);
  }
</script>

<svelte:window onpointerdown={onDown} onresize={() => ctxMenu.close(false)} onblur={() => ctxMenu.close(false)} />

{#if ctxMenu.isOpen}
  <div
    bind:this={el}
    class="anim-in fixed z-[70] min-w-[200px] max-w-[300px] rounded-[10px] border border-border bg-surface py-1.5 shadow-[var(--shadow-lg)] outline-none"
    style="left:{pos.left}px;top:{pos.top}px"
    role="menu"
    tabindex="-1"
    aria-label={ctxMenu.label}
    data-testid="context-menu"
    onkeydown={onKey}
    oncontextmenu={(e) => e.preventDefault()}
  >
    {#each ctxMenu.items as it, i (it.id)}
      {#if it.separator}
        <div class="my-1 h-px bg-border" role="separator"></div>
      {:else}
        {@const Icon = it.icon as typeof import("@lucide/svelte").Check | undefined}
        <button
          role="menuitem"
          class="flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-[13px] disabled:opacity-45 {it.danger ? 'text-danger' : ''} {active === i ? 'bg-surface-2' : ''}"
          disabled={it.disabled}
          tabindex="-1"
          data-testid="menu-{it.id}"
          aria-haspopup={it.submenu ? "menu" : undefined}
          onpointerenter={() => !it.disabled && (active = i)}
          onclick={() => choose(i)}
        >
          <span class="flex w-4 shrink-0 justify-center text-muted">{#if Icon}<Icon size={15} />{/if}</span>
          <span class="min-w-0 flex-1 truncate">{it.label}</span>
          {#if it.hint}<span class="text-[11px] text-faint">{it.hint}</span>{/if}
          {#if it.submenu}<ChevronRight size={14} class="text-faint" aria-hidden="true" />{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}
