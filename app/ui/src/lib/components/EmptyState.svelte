<script lang="ts">
  import type { Component, Snippet } from "svelte";
  interface Props {
    icon?: Component<{ size?: number }>;
    title: string;
    message?: string;
    children?: Snippet;
  }
  let { icon: Icon, title, message, children }: Props = $props();
</script>

<div class="flex h-full min-h-40 flex-col items-center justify-center gap-2 p-8 text-center">
  {#if Icon}
    <div class="mb-1 flex h-12 w-12 items-center justify-center rounded-full bg-surface-2 text-faint"><Icon size={22} /></div>
  {/if}
  <p class="font-medium">{title}</p>
  {#if message}<p class="max-w-sm text-[13px] text-muted">{message}</p>{/if}
  {@render children?.()}
</div>
