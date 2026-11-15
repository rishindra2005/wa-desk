// Overlay stack: only the top-most overlay reacts to Escape, and focus is
// trapped inside it and restored when it closes.

type Closer = () => void;
const stack: { id: number; close: Closer }[] = [];
let seq = 0;

export function overlayCount(): number {
  return stack.length;
}

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"]), video[controls], audio[controls]';

export function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => !el.hasAttribute("inert") && el.getAttribute("aria-hidden") !== "true",
  );
}

/**
 * Svelte action: register `node` as a modal overlay. Escape calls `close`,
 * Tab is trapped, initial focus goes to `[data-autofocus]` or the first control.
 */
export function trapFocus(node: HTMLElement, close: Closer) {
  const id = ++seq;
  let closer = close;
  stack.push({ id, close: () => closer() });
  const previous = document.activeElement as HTMLElement | null;

  queueMicrotask(() => {
    const target = node.querySelector<HTMLElement>("[data-autofocus]") ?? focusables(node)[0] ?? node;
    target.focus({ preventScroll: true });
  });

  const onKey = (e: KeyboardEvent) => {
    const top = stack[stack.length - 1];
    if (!top || top.id !== id) return;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      top.close();
      return;
    }
    if (e.key === "Tab") {
      const els = focusables(node);
      if (els.length === 0) {
        e.preventDefault();
        return;
      }
      const first = els[0] as HTMLElement;
      const last = els[els.length - 1] as HTMLElement;
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !node.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (active === last || !node.contains(active))) {
        e.preventDefault();
        first.focus();
      }
    }
  };
  window.addEventListener("keydown", onKey, true);

  return {
    update(c: Closer) {
      closer = c;
    },
    destroy() {
      window.removeEventListener("keydown", onKey, true);
      const i = stack.findIndex((s) => s.id === id);
      if (i >= 0) stack.splice(i, 1);
      if (previous && document.contains(previous)) previous.focus({ preventScroll: true });
    },
  };
}
