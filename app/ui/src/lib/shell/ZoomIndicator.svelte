<!-- Transient "110%" badge shown while zooming. -->
<script lang="ts">
  import { app } from "../state/app.svelte";
</script>

{#if app.zoomShown}
  <div
    class="pointer-events-none fixed left-1/2 top-4 z-[80] -translate-x-1/2 rounded-full border border-border bg-surface px-3 py-1 text-sm font-medium tabular-nums shadow-[var(--shadow-lg)] anim-in"
    role="status"
    aria-live="polite"
    data-testid="zoom-indicator"
  >
    {app.zoom}%
  </div>
{/if}
