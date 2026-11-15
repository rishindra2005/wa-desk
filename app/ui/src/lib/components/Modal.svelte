<script lang="ts">
  import type { Snippet } from "svelte";
  import { X } from "@lucide/svelte";
  import { trapFocus } from "./overlay";

  interface Props {
    title: string;
    onclose: () => void;
    children: Snippet;
    footer?: Snippet;
    width?: string;
    describedBy?: string;
  }
  let { title, onclose, children, footer, width = "28rem", describedBy }: Props = $props();
  const id = `m${Math.random().toString(36).slice(2, 8)}`;
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 anim-in" role="presentation" onmousedown={(e) => e.target === e.currentTarget && onclose()}>
  <div
    class="flex max-h-[90vh] w-full flex-col overflow-hidden rounded-[10px] border border-border bg-surface shadow-[var(--shadow-lg)]"
    style="max-width:{width}"
    role="dialog"
    aria-modal="true"
    aria-labelledby="{id}-title"
    aria-describedby={describedBy}
    tabindex="-1"
    use:trapFocus={onclose}
  >
    <header class="flex items-center justify-between border-b border-border px-4 py-3">
      <h2 id="{id}-title" class="text-[15px] font-semibold">{title}</h2>
      <button class="icon-btn" aria-label="Close" onclick={onclose}><X size={16} /></button>
    </header>
    <div class="min-h-0 overflow-y-auto px-4 py-4">
      {@render children()}
    </div>
    {#if footer}
      <footer class="flex justify-end gap-2 border-t border-border bg-surface-2/50 px-4 py-3">
        {@render footer()}
      </footer>
    {/if}
  </div>
</div>
