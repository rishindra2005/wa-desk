// Single global context menu. Components call `ctxMenu.open(...)` (pointer) or
// `ctxMenu.openFor(el, ...)` (keyboard / ⋮ button); `ContextMenu.svelte` renders it.

import { isMenuKey, tidyItems, type MenuItem } from "../utils/menu";

class CtxMenu {
  items = $state<MenuItem[]>([]);
  x = $state(0);
  y = $state(0);
  isOpen = $state(false);
  /** Element to restore focus to on close. */
  returnTo: HTMLElement | null = null;
  label = $state("Context menu");

  open(items: MenuItem[], x: number, y: number, label = "Context menu"): void {
    const t = tidyItems(items);
    if (t.length === 0) return;
    this.returnTo = (document.activeElement as HTMLElement | null) ?? null;
    this.items = t;
    this.x = x;
    this.y = y;
    this.label = label;
    this.isOpen = true;
  }

  /** Open anchored to an element's bottom-left (keyboard and button triggers). */
  openFor(el: Element, items: MenuItem[], label?: string): void {
    const r = el.getBoundingClientRect();
    this.open(items, r.left + Math.min(24, r.width / 2), r.bottom - Math.min(8, r.height / 2), label);
  }

  close(restoreFocus = true): void {
    if (!this.isOpen) return;
    this.isOpen = false;
    const el = this.returnTo;
    this.returnTo = null;
    if (restoreFocus && el && document.contains(el)) el.focus({ preventScroll: true });
  }
}

export const ctxMenu = new CtxMenu();

/**
 * Svelte action: right-click and Shift+F10 / ContextMenu key open `getItems()`.
 * The element must be focusable (tabindex) for keyboard access.
 */
export function contextMenu(node: HTMLElement, getItems: () => MenuItem[] | null) {
  let get = getItems;
  const onCtx = (e: MouseEvent) => {
    const items = get();
    if (!items) return;
    e.preventDefault();
    e.stopPropagation();
    // Keyboard-initiated contextmenu events report (0,0) or the element centre.
    if (e.clientX === 0 && e.clientY === 0) ctxMenu.openFor(node, items);
    else ctxMenu.open(items, e.clientX, e.clientY);
  };
  const onKey = (e: KeyboardEvent) => {
    if (!isMenuKey(e) || e.target !== node) return;
    const items = get();
    if (!items) return;
    e.preventDefault();
    e.stopPropagation();
    ctxMenu.openFor(node, items);
  };
  node.addEventListener("contextmenu", onCtx);
  node.addEventListener("keydown", onKey);
  return {
    update(g: () => MenuItem[] | null) {
      get = g;
    },
    destroy() {
      node.removeEventListener("contextmenu", onCtx);
      node.removeEventListener("keydown", onKey);
    },
  };
}
