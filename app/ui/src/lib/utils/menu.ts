// Pure helpers for context menus: viewport clamping and keyboard navigation.

export interface MenuItem {
  id: string;
  label?: string;
  /** Lucide component (rendered at 15px). */
  icon?: unknown;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  separator?: boolean;
  onselect?: () => void;
  /** Opens a nested list in the same menu (Enter / click / ArrowRight; ArrowLeft goes back). */
  submenu?: MenuItem[];
}

/** Place a `w`×`h` menu at (x, y), flipping/shifting so it stays inside the viewport. */
export function clampPosition(
  x: number,
  y: number,
  w: number,
  h: number,
  vw: number,
  vh: number,
  margin = 8,
): { left: number; top: number } {
  let left = x;
  let top = y;
  if (left + w + margin > vw) left = x - w; // flip to the left of the pointer
  if (left + w + margin > vw) left = vw - w - margin;
  if (top + h + margin > vh) top = y - h; // flip above
  if (top + h + margin > vh) top = vh - h - margin;
  return { left: Math.max(margin, left), top: Math.max(margin, top) };
}

/** Next enabled, non-separator index in direction `dir` (wraps). -1 when none. */
export function nextIndex(items: MenuItem[], from: number, dir: 1 | -1): number {
  const n = items.length;
  if (n === 0) return -1;
  let i = from;
  for (let step = 0; step < n; step++) {
    i = (i + dir + n) % n;
    const it = items[i];
    if (it && !it.separator && !it.disabled) return i;
  }
  return -1;
}

/** Keys that open a context menu from the keyboard. */
export function isMenuKey(e: { key: string; shiftKey: boolean }): boolean {
  return e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey);
}

/** Drop leading/trailing/duplicate separators. */
export function tidyItems(items: MenuItem[]): MenuItem[] {
  const out: MenuItem[] = [];
  for (const it of items) {
    if (it.separator && (out.length === 0 || out[out.length - 1]?.separator)) continue;
    out.push(it);
  }
  while (out[out.length - 1]?.separator) out.pop();
  return out;
}
