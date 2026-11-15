<script lang="ts">
  import { TriangleAlert } from "@lucide/svelte";
  import { friendly } from "../errors";
  let { error, reset, title }: { error: unknown; reset?: () => void; title?: string } = $props();
  const f = $derived(friendly(error));
</script>

<div class="flex h-full min-h-40 flex-col items-center justify-center gap-2 p-8 text-center" role="alert">
  <div class="flex h-12 w-12 items-center justify-center rounded-full bg-danger-soft text-danger"><TriangleAlert size={22} /></div>
  <p class="font-medium">{title ?? f.title}</p>
  {#if f.message}<p class="max-w-md text-[13px] text-muted">{f.message}</p>{/if}
  {#if reset}<button class="btn mt-2" onclick={reset}>Try again</button>{/if}
</div>
