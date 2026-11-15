<script lang="ts">
  import { pct } from "../utils/format";
  interface Props {
    done: number;
    total: number;
    label?: string;
    indeterminate?: boolean;
    tone?: "accent" | "danger" | "muted";
  }
  let { done, total, label = "Progress", indeterminate = false, tone = "accent" }: Props = $props();
  const p = $derived(pct(done, total));
  const color = $derived(tone === "danger" ? "var(--danger)" : tone === "muted" ? "var(--text-faint)" : "var(--accent)");
</script>

<div
  class="h-1.5 w-full overflow-hidden rounded-full bg-surface-3"
  role="progressbar"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={indeterminate || total <= 0 ? undefined : Math.round(p)}
>
  {#if indeterminate || total <= 0}
    <div class="skeleton h-full w-full"></div>
  {:else}
    <div class="h-full rounded-full transition-[width] duration-300" style="width:{p}%;background:{color}"></div>
  {/if}
</div>
