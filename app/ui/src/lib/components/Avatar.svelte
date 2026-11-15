<script lang="ts">
  import { Users } from "@lucide/svelte";
  import { colorHue } from "../utils/misc";
  import { initials } from "../utils/text";

  interface Props {
    name: string;
    id?: string;
    size?: number;
    group?: boolean;
  }
  let { name, id, size = 40, group = false }: Props = $props();
  const hue = $derived(colorHue(id ?? name));
  const text = $derived(initials(name));
</script>

<div
  class="flex shrink-0 select-none items-center justify-center rounded-full font-semibold text-white"
  style="width:{size}px;height:{size}px;font-size:{Math.round(size * 0.38)}px;background:hsl({hue} var(--avatar-s) var(--avatar-l))"
  aria-hidden="true"
>
  {#if group && text === "#"}
    <Users size={Math.round(size * 0.45)} />
  {:else}
    {text}
  {/if}
</div>
