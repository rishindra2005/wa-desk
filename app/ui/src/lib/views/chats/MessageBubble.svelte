<script lang="ts">
  import { Ban, Reply, SmilePlus, Copy } from "@lucide/svelte";
  import type { Message } from "../../bindings/Message";
  import type { MediaRef } from "../../bindings/MediaRef";
  import MediaContent from "./MediaContent.svelte";
  import { linkify } from "../../utils/text";
  import { timeOfDay, displayName, dateTime } from "../../utils/format";
  import { colorHue, copyText } from "../../utils/misc";
  import { openLink } from "../../platform";
  import { toasts } from "../../state/toasts.svelte";

  interface Props {
    m: Message;
    isGroup: boolean;
    showName: boolean;
    first: boolean;
    highlighted: boolean;
    onreact: (m: Message, emoji: string) => void;
    onreply: (m: Message) => void;
    onjump: (id: string) => void;
    onview: (m: Message) => void;
    onpreview: (m: Message) => void;
    onmedia: (m: Message, ref: MediaRef) => void;
  }
  let { m, isGroup, showName, first, highlighted, onreact, onreply, onjump, onview, onpreview, onmedia }: Props = $props();

  const QUICK_EMOJIS = ["👍", "❤️", "😂", "😮", "😢", "🙏"];
  let pickerOpen = $state(false);

  const mine = $derived(m.is_from_me);
  const sticker = $derived(m.media?.kind === "sticker" && !m.content);
  const myReaction = $derived(m.reactions.find((r) => r.is_from_me)?.emoji ?? null);
  const grouped = $derived.by(() => {
    const map = new Map<string, { emoji: string; count: number; mine: boolean; names: string[] }>();
    for (const r of m.reactions) {
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

  function react(emoji: string) {
    pickerOpen = false;
    onreact(m, emoji);
  }

  async function copy() {
    if (await copyText(m.content)) toasts.success("Copied");
  }
</script>

<div class="group relative flex px-4 {mine ? 'justify-end' : 'justify-start'} {first ? 'pt-2' : 'pt-0.5'}" data-testid="message" data-id={m.id}>
  <div class="relative max-w-[min(72%,560px)]">
    <div
      class="relative rounded-[10px] text-[14px] leading-snug {sticker ? '' : 'px-2.5 py-1.5 shadow-[var(--shadow)]'} {highlighted ? 'flash' : ''}"
      style={sticker ? "" : `background:${mine ? "var(--bubble-out)" : "var(--bubble-in)"}`}
      class:rounded-tr-sm={mine && first && !sticker}
      class:rounded-tl-sm={!mine && first && !sticker}
    >
      {#if isGroup && showName && !mine}
        <p class="mb-0.5 text-[12.5px] font-semibold" style="color:{nameColor}" data-testid="sender-name">{displayName(m.sender_name, m.sender_jid)}</p>
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
          <div class={m.content ? "mb-1" : ""}>
            <MediaContent message={m} media={m.media} {onview} {onpreview} onupdate={(ref) => onmedia(m, ref)} />
          </div>
        {/if}
        {#if m.content}
          <p class="whitespace-pre-wrap break-words" dir="auto">
            {#each parts as p, i (i)}{#if p.href}<a
                  href={p.href}
                  class="text-info underline decoration-1 underline-offset-2 hover:opacity-80"
                  onclick={(e) => {
                    e.preventDefault();
                    if (p.href) openLink(p.href).catch((err) => toasts.error(err));
                  }}>{p.text}</a
                >{:else}{p.text}{/if}{/each}<span class="inline-block w-12"></span>
          </p>
        {/if}
      {/if}
      <span class="absolute bottom-1 right-2 text-[10.5px] text-faint {sticker ? 'rounded bg-surface px-1' : ''}" title={dateTime(m.ts)}>{timeOfDay(m.ts)}</span>
      {#if !m.content && !m.deleted && !sticker}<div class="h-3.5"></div>{/if}
    </div>
    {#if grouped.length}
      <div class="-mt-1 flex flex-wrap gap-1 px-1 {mine ? 'justify-end' : ''}" data-testid="reactions">
        {#each grouped as g (g.emoji)}
          <button
            class="inline-flex h-6 items-center gap-1 rounded-full border px-1.5 text-[12px] shadow-[var(--shadow)] {g.mine ? 'border-accent bg-accent-soft' : 'border-border bg-surface'}"
            title={g.names.join(", ")}
            aria-label="{g.emoji} by {g.names.join(', ')}"
            onclick={() => react(g.mine ? "" : g.emoji)}
          >
            <span>{g.emoji}</span>{#if g.count > 1}<span class="text-muted">{g.count}</span>{/if}
          </button>
        {/each}
      </div>
    {/if}
    {#if !m.deleted}
      <div
        class="absolute top-0 z-10 flex items-center gap-0.5 rounded-md border border-border bg-surface p-0.5 opacity-0 shadow-[var(--shadow)] transition-opacity group-hover:opacity-100 focus-within:opacity-100 {pickerOpen ? 'opacity-100' : ''} {mine ? 'right-full mr-1' : 'left-full ml-1'}"
      >
        <button class="icon-btn h-7 w-7" aria-label="React" aria-expanded={pickerOpen} onclick={() => (pickerOpen = !pickerOpen)} data-testid="react-btn"><SmilePlus size={15} /></button>
        <button class="icon-btn h-7 w-7" aria-label="Reply" onclick={() => onreply(m)} data-testid="reply-btn"><Reply size={15} /></button>
        {#if m.content}<button class="icon-btn h-7 w-7" aria-label="Copy text" onclick={copy}><Copy size={14} /></button>{/if}
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
