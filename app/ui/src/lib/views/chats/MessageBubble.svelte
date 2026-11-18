<script lang="ts">
  import { Ban, Reply, SmilePlus, Copy, Star, ChevronDown, RotateCw, X as XIcon, CircleAlert } from "@lucide/svelte";
  import type { Message } from "../../bindings/Message";
  import type { MediaRef } from "../../bindings/MediaRef";
  import MediaContent from "./MediaContent.svelte";
  import AlbumGrid from "./AlbumGrid.svelte";
  import { Download as DownloadIcon } from "@lucide/svelte";
  import Avatar from "../../components/Avatar.svelte";
  import StatusTicks, { type TickState } from "../../components/StatusTicks.svelte";
  import { linkify } from "../../utils/text";
  import { timeOfDay, displayName, dateTime } from "../../utils/format";
  import { colorHue, copyText } from "../../utils/misc";
  import type { MenuItem } from "../../utils/menu";
  import { contextMenu, ctxMenu } from "../../state/menu.svelte";
  import { openLink } from "../../platform";
  import { toasts } from "../../state/toasts.svelte";

  interface Props {
    /** The message the bubble acts on; for an album, its last item (the caption carrier). */
    m: Message;
    /** All items when this bubble is an album (photos/videos sent together). */
    album?: Message[] | null;
    isGroup: boolean;
    showName: boolean;
    first: boolean;
    highlighted: boolean;
    /** Delivery state for own messages (pending/failed for local sends). */
    tick?: TickState | null;
    selecting?: boolean;
    selected?: boolean;
    local?: boolean;
    /** Changes when the context menu asks to open the reaction picker. */
    reactRequest?: number;
    menuItems?: (m: Message) => MenuItem[];
    /** Menu for the whole album (the bubble's own menu button and right-click on the caption). */
    albumMenu?: (items: Message[]) => MenuItem[];
    /** "Download all" on an album. */
    ondownloadall?: (items: Message[]) => void;
    onreact: (m: Message, emoji: string) => void;
    onreply: (m: Message) => void;
    onjump: (id: string) => void;
    onview: (m: Message, album?: Message[]) => void;
    onpreview: (m: Message) => void;
    onmedia: (m: Message, ref: MediaRef) => void;
    onselect?: (m: Message, e: MouseEvent | KeyboardEvent) => void;
    onprofile?: (jid: string) => void;
    onretry?: (m: Message) => void;
    ondiscard?: (m: Message) => void;
  }
  let {
    m, album = null, isGroup, showName, first, highlighted, tick = null, selecting = false, selected = false, local = false, reactRequest = 0, menuItems, albumMenu, ondownloadall,
    onreact, onreply, onjump, onview, onpreview, onmedia, onselect, onprofile, onretry, ondiscard,
  }: Props = $props();

  const QUICK_EMOJIS = ["👍", "❤️", "😂", "😮", "😢", "🙏"];
  let pickerOpen = $state(false);
  $effect(() => {
    if (reactRequest) pickerOpen = true;
  });
  let moreBtn: HTMLButtonElement | undefined = $state();

  const mine = $derived(m.is_from_me);
  const sticker = $derived(m.media?.kind === "sticker" && !m.content);
  const myReaction = $derived(m.reactions.find((r) => r.is_from_me)?.emoji ?? null);
  const grouped = $derived.by(() => {
    const map = new Map<string, { emoji: string; count: number; mine: boolean; names: string[] }>();
    for (const r of album ? album.flatMap((x) => x.reactions) : m.reactions) {
      const g = map.get(r.emoji) ?? { emoji: r.emoji, count: 0, mine: false, names: [] };
      g.count++;
      g.mine ||= r.is_from_me;
      g.names.push(r.is_from_me ? "You" : r.sender_name);
      map.set(r.emoji, g);
    }
    return [...map.values()];
  });
  const nameColor = $derived(`hsl(${colorHue(m.sender_jid ?? m.sender_name)} 60% var(--name-l))`);
  const parts = $derived(m.content ? linkify(m.content) : []);
  const label = $derived(displayName(m.sender_name, m.sender_jid));

  function react(emoji: string) {
    pickerOpen = false;
    onreact(m, emoji);
  }
  /** Toggle a reaction chip: removing my reaction targets the album item that carries it. */
  function toggleChip(g: { emoji: string; mine: boolean }) {
    if (!g.mine) return react(g.emoji);
    const owner = album?.find((x) => x.reactions.some((r) => r.is_from_me && r.emoji === g.emoji)) ?? m;
    pickerOpen = false;
    onreact(owner, "");
  }
  const anyStarred = $derived(album ? album.some((x) => x.starred) : m.starred);

  async function copy() {
    if (await copyText(m.content)) toasts.success("Copied");
  }

  const items = () => (local ? null : album && albumMenu ? albumMenu(album) : menuItems ? menuItems(m) : null);
  function openMore() {
    const it = items();
    if (it && moreBtn) ctxMenu.openFor(moreBtn, it, "Message actions");
  }
  function rowClick(e: MouseEvent) {
    if (selecting) {
      e.preventDefault();
      onselect?.(m, e);
    }
  }
  function rowKey(e: KeyboardEvent) {
    if (selecting && e.target === e.currentTarget && (e.key === " " || e.key === "Enter")) {
      e.preventDefault();
      onselect?.(m, e);
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="group relative flex px-4 outline-none {selecting ? 'cursor-pointer' : ''} {selected ? 'bg-accent-soft/70' : ''} {mine ? 'justify-end' : 'justify-start'} {first ? 'pt-2' : 'pt-0.5'}"
  data-testid="message"
  data-id={m.id}
  data-album-ids={album ? album.map((x) => x.id).join(" ") : undefined}
  data-selected={selected}
  tabindex="0"
  role={selecting ? "option" : "group"}
  aria-selected={selecting ? selected : undefined}
  aria-label="{mine ? 'You' : label}: {m.content || (album ? `${album.length} photos and videos` : m.media?.kind) || ''}"
  use:contextMenu={items}
  onclick={rowClick}
  onkeydown={rowKey}
>
  {#if selecting}
    <span class="mr-3 flex w-5 shrink-0 items-center self-center" aria-hidden="true">
      <span class="flex h-[18px] w-[18px] items-center justify-center rounded border-2 {selected ? 'border-accent bg-accent text-accent-fg' : 'border-border-strong bg-surface'}" data-testid="select-box">
        {#if selected}<svg viewBox="0 0 12 12" width="10" height="10"><path d="M2 6.5l2.5 2.5L10 3.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>{/if}
      </span>
    </span>
  {/if}
  <div class="flex min-w-0 flex-1 {mine ? 'justify-end' : 'justify-start'}">
    {#if isGroup && !mine}
      <div class="mr-2 w-7 shrink-0 self-end">
        {#if showName}
          <Avatar name={label} id={m.sender_jid ?? m.sender_name} size={28} onclick={m.sender_jid && onprofile && !selecting ? () => onprofile?.(m.sender_jid as string) : undefined} label="{label} profile" />
        {/if}
      </div>
    {/if}
    <div class="relative max-w-[min(72%,560px)] {selecting ? 'pointer-events-none' : ''}">
      <div
        class="relative rounded-[10px] text-[14px] leading-snug {sticker ? '' : 'px-2.5 py-1.5 shadow-[var(--shadow)]'} {highlighted ? 'flash' : ''}"
        style={sticker ? "" : `background:${mine ? "var(--bubble-out)" : "var(--bubble-in)"}`}
        class:rounded-tr-sm={mine && first && !sticker}
        class:rounded-tl-sm={!mine && first && !sticker}
      >
        {#if isGroup && showName && !mine}
          <button
            class="mb-0.5 block max-w-full truncate text-left text-[12.5px] font-semibold hover:underline"
            style="color:{nameColor}"
            data-testid="sender-name"
            onclick={() => m.sender_jid && onprofile?.(m.sender_jid)}
            tabindex="-1"
          >{label}</button>
        {/if}
        {#if m.deleted}
          <p class="flex items-center gap-1.5 pr-10 italic text-muted" data-testid="deleted"><Ban size={14} /> This message was deleted</p>
        {:else}
          {#if m.quoted}
            <button
              class="mb-1 block w-full rounded-md border-l-[3px] border-accent bg-black/[0.05] px-2 py-1 text-left dark:bg-white/[0.06]"
              onclick={() => m.quoted && onjump(m.quoted.id)}
              data-testid="quoted"
              aria-label="Jump to quoted message"
            >
              <span class="block text-[12px] font-semibold text-accent-text">{m.quoted.sender_name ?? "Message"}</span>
              <span class="line-clamp-2 block text-[12.5px] text-muted">{m.quoted.preview || "Original message not available"}</span>
            </button>
          {/if}
          {#if m.media}
            <div class="relative {m.content ? 'mb-1' : ''}">
              {#if album}
                <AlbumGrid items={album} {onview} onupdate={onmedia} tileMenu={menuItems && !local ? menuItems : undefined} inert={selecting} />
                {#if ondownloadall && !selecting}
                  <button
                    class="absolute left-3 top-3 z-[5] flex items-center gap-1 rounded-md bg-black/55 px-1.5 py-1 text-[11px] text-white opacity-0 transition-opacity hover:bg-black/70 focus:opacity-100 group-hover:opacity-100"
                    onclick={() => ondownloadall?.(album)}
                    aria-label="Download all {album.length} items"
                    data-testid="album-download-all"
                  ><DownloadIcon size={12} /> Download all</button>
                {/if}
              {:else}
                <MediaContent message={m} media={m.media} {onview} {onpreview} onupdate={(ref) => onmedia(m, ref)} />
              {/if}
            </div>
          {/if}
          {#if m.content}
            <p class="select-text whitespace-pre-wrap break-words" dir="auto" data-testid="message-text">
              {#each parts as p, i (i)}{#if p.href}<a
                    href={p.href}
                    class="text-info underline decoration-1 underline-offset-2 hover:opacity-80"
                    onclick={(e) => {
                      e.preventDefault();
                      if (p.href) openLink(p.href).catch((err) => toasts.error(err));
                    }}>{p.text}</a
                  >{:else}{p.text}{/if}{/each}<span class="inline-block {mine ? 'w-[4.5rem]' : 'w-12'}"></span>
            </p>
          {/if}
        {/if}
        <span class="absolute bottom-1 right-2 flex items-center gap-1 text-[10.5px] text-faint {sticker ? 'rounded bg-surface px-1' : ''}">
          {#if anyStarred}<Star size={11} class="fill-warn text-warn" aria-label="Starred" data-testid="starred-icon" />{/if}
          <span title={dateTime(m.ts)}>{timeOfDay(m.ts)}</span>
          {#if mine && tick}<StatusTicks status={tick} />{/if}
        </span>
        {#if !m.content && !m.deleted && !sticker}<div class="h-3.5"></div>{/if}
      </div>
      {#if tick === "failed"}
        <div class="mt-1 flex items-center justify-end gap-2 text-xs text-danger" data-testid="send-failed">
          <CircleAlert size={13} /> Not sent
          <button class="btn btn-sm pointer-events-auto" onclick={() => onretry?.(m)} data-testid="retry-send"><RotateCw size={12} /> Retry</button>
          <button class="icon-btn pointer-events-auto h-6 w-6" aria-label="Discard" onclick={() => ondiscard?.(m)}><XIcon size={13} /></button>
        </div>
      {/if}
      {#if grouped.length}
        <div class="-mt-1 flex flex-wrap gap-1 px-1 {mine ? 'justify-end' : ''}" data-testid="reactions">
          {#each grouped as g (g.emoji)}
            <button
              class="inline-flex h-6 items-center gap-1 rounded-full border px-1.5 text-[12px] shadow-[var(--shadow)] {g.mine ? 'border-accent bg-accent-soft' : 'border-border bg-surface'}"
              title={g.names.join(", ")}
              aria-label="{g.emoji} by {g.names.join(', ')}"
              onclick={() => toggleChip(g)}
            >
              <span>{g.emoji}</span>{#if g.count > 1}<span class="text-muted">{g.count}</span>{/if}
            </button>
          {/each}
        </div>
      {/if}
      {#if !m.deleted && !selecting && !local}
        <div
          class="absolute top-0 z-10 flex items-center gap-0.5 rounded-md border border-border bg-surface p-0.5 opacity-0 shadow-[var(--shadow)] transition-opacity group-hover:opacity-100 focus-within:opacity-100 {pickerOpen ? 'opacity-100' : ''} {mine ? 'right-full mr-1' : 'left-full ml-1'}"
        >
          <button class="icon-btn h-7 w-7" aria-label="React" aria-expanded={pickerOpen} onclick={() => (pickerOpen = !pickerOpen)} data-testid="react-btn"><SmilePlus size={15} /></button>
          <button class="icon-btn h-7 w-7" aria-label="Reply" onclick={() => onreply(m)} data-testid="reply-btn"><Reply size={15} /></button>
          {#if m.content}<button class="icon-btn h-7 w-7" aria-label="Copy text" onclick={copy}><Copy size={14} /></button>{/if}
          <button bind:this={moreBtn} class="icon-btn h-7 w-7" aria-label="More actions" aria-haspopup="menu" onclick={openMore} data-testid="msg-more"><ChevronDown size={15} /></button>
        </div>
        {#if pickerOpen}
          <div
            class="anim-in absolute top-9 z-20 flex items-center gap-0.5 rounded-full border border-border bg-surface p-1 shadow-[var(--shadow-lg)] {mine ? 'right-0' : 'left-0'}"
            role="menu"
            aria-label="Reactions"
            tabindex="-1"
            data-testid="react-picker"
            onkeydown={(e) => e.key === "Escape" && (pickerOpen = false)}
          >
            {#each QUICK_EMOJIS as e (e)}
              <button role="menuitem" class="h-8 w-8 rounded-full text-[18px] transition-transform hover:scale-125 hover:bg-surface-2 {myReaction === e ? 'bg-accent-soft' : ''}" aria-label="React {e}" onclick={() => react(e)}>{e}</button>
            {/each}
            {#if myReaction}
              <button role="menuitem" class="h-8 rounded-full px-2 text-xs text-muted hover:bg-surface-2" onclick={() => react("")}>Remove</button>
            {/if}
          </div>
        {/if}
      {/if}
    </div>
  </div>
</div>
