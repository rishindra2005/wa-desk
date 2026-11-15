<script lang="ts">
  import { LoaderCircle } from "@lucide/svelte";
  let { size = 16, label = "Loading", class: cls = "" }: { size?: number; label?: string; class?: string } = $props();
</script>

<span role="status" aria-label={label} class="inline-flex {cls}"><LoaderCircle {size} class="spin" /></span>
