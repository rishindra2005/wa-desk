<script lang="ts">
  let { rows = 6, avatar = false }: { rows?: number; avatar?: boolean } = $props();
</script>

<div class="flex flex-col gap-3 p-3" aria-busy="true" aria-label="Loading">
  {#each Array.from({ length: rows }, (_, i) => i) as i (i)}
    <div class="flex items-center gap-3">
      {#if avatar}<div class="skeleton h-10 w-10 shrink-0 rounded-full"></div>{/if}
      <div class="flex-1 space-y-2">
        <div class="skeleton h-3" style="width:{40 + ((i * 37) % 45)}%"></div>
        <div class="skeleton h-2.5" style="width:{55 + ((i * 23) % 35)}%"></div>
      </div>
    </div>
  {/each}
</div>
