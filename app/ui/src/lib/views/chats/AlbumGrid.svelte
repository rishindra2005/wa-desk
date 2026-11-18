<!-- WhatsApp-style photo/video album: 2 side by side, 3 as one large + two, 4+ as a 2x2 grid with "+N". -->
<script lang="ts">
  import { Download, Play } from "@lucide/svelte";
  import type { Message } from "../../bindings/Message";
  import type { MediaRef } from "../../bindings/MediaRef";
  import Spinner from "../../components/Spinner.svelte";
  import { albumLayout, albumTiles } from "../../utils/albums";
  import { downloadMedia } from "../../media";
  import { contextMenu } from "../../state/menu.svelte";
  import type { MenuItem } from "../../utils/menu";

  interface Props {
    items: Message[];
    /** Open the viewer at this item (only items that are on disk are viewable). */
    onview: (m: Message, album: Message[]) => void;
    onupdate: (m: Message, ref: MediaRef) => void;
    /** Context menu for one item (reply, react, star, save...). */
    tileMenu?: (m: Message) => MenuItem[] | null;
    /** Disable clicks (selection mode). */
    inert?: boolean;
  }
  let { items, onview, onupdate, tileMenu, inert = false }: Props = $props();

  const layout = $derived(albumLayout(items.length));
  const tiles = $derived(albumTiles(items));
  let busy = $state<Set<string>>(new Set());

  const isMissing = (m: Message) => !m.media?.url || m.media.source === "missing";

  async function click(m: Message) {
    if (!isMissing(m)) return onview(m, items);
    if (busy.has(m.id)) return;
    busy = new Set([...busy, m.id]);
    const ref = await downloadMedia(m.chat_jid, m.id);
    busy = new Set([...busy].filter((x) => x !== m.id));
    if (ref) onupdate(m, ref);
  }

  function label(m: Message, i: number): string {
    const kind = m.media?.kind === "video" ? "video" : "photo";
    return isMissing(m) ? `Download ${kind} ${i + 1} of ${items.length}` : `View ${kind} ${i + 1} of ${items.length}`;
  }
</script>

<div
  class="grid w-72 max-w-full gap-0.5 overflow-hidden rounded-md {layout === 'pair' ? 'aspect-[2/1] grid-cols-2' : 'aspect-square grid-cols-2 grid-rows-2'}"
  data-testid="album"
  data-count={items.length}
  data-layout={layout}
>
  {#each tiles.shown as m, i (m.id)}
    {@const last = i === tiles.shown.length - 1 && tiles.extra > 0}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <button
      class="relative block min-h-0 min-w-0 overflow-hidden bg-surface-3 {layout === 'trio' && i === 0 ? 'row-span-2' : ''} {inert ? 'pointer-events-none' : ''}"
      onclick={() => void click(m)}
      aria-label={label(m, i)}
      data-testid="album-tile"
      data-id={m.id}
      data-missing={isMissing(m)}
      use:contextMenu={() => (tileMenu ? tileMenu(m) : null)}
    >
      {#if isMissing(m)}
        <span class="flex h-full w-full flex-col items-center justify-center gap-1 text-muted">
          {#if busy.has(m.id)}<Spinner size={18} label="Downloading" />{:else}<Download size={18} />{/if}
          <span class="text-[11px]">{m.media?.kind === "video" ? "Video" : "Photo"}</span>
        </span>
      {:else}
        <img
          src={m.media?.thumb_url ?? m.media?.url}
          alt={m.content || m.media?.filename || "Photo"}
          loading="lazy"
          class="h-full w-full object-cover"
        />
        {#if m.media?.kind === "video"}
          <span class="absolute inset-0 flex items-center justify-center"><span class="rounded-full bg-black/55 p-1.5 text-white"><Play size={14} /></span></span>
        {/if}
      {/if}
      {#if last}
        <span class="absolute inset-0 flex items-center justify-center bg-black/55 text-[26px] font-medium text-white" data-testid="album-more">+{tiles.extra}</span>
      {/if}
    </button>
  {/each}
</div>

