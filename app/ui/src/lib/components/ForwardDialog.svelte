<!-- Forward messages: pick up to 5 chats, then see a result per target. -->
<script lang="ts">
  import { Search, Check, CircleX } from "@lucide/svelte";
  import * as api from "../api";
  import type { ChatSummary } from "../bindings/ChatSummary";
  import type { SendResult } from "../bindings/SendResult";
  import Avatar from "./Avatar.svelte";
  import Modal from "./Modal.svelte";
  import Spinner from "./Spinner.svelte";
  import { app } from "../state/app.svelte";
  import { toasts } from "../state/toasts.svelte";
  import { chatLabel } from "../utils/format";
  import { debounce } from "../utils/misc";
  import { MAX_FORWARD_TARGETS, toggleTarget } from "../utils/selection";

  let { chatJid, messageIds }: { chatJid: string; messageIds: string[] } = $props();

  let query = $state("");
  let chats = $state<ChatSummary[]>([]);
  let selected = $state<string[]>([]);
  let limitHit = $state(false);
  let busy = $state(false);
  let results = $state<{ jid: string; name: string; ok: boolean; message: string }[] | null>(null);
  const names = new Map<string, string>();

  async function load(q: string) {
    try {
      const p = await api.listChats({ filter: q || null, kind: "all", unread_only: false, cursor: null, limit: 80 });
      chats = p.items.filter((c) => c.chat_type !== "channel");
      for (const c of p.items) names.set(c.jid, chatLabel(c));
    } catch (e) {
      toasts.error(e);
    }
  }
  const search = debounce((q: string) => void load(q), 150);
  $effect(() => {
    search(query);
    return () => search.cancel();
  });

  function toggle(jid: string) {
    const r = toggleTarget(selected, jid);
    selected = r.selected;
    limitHit = r.limited;
  }

  async function send() {
    busy = true;
    try {
      const res: SendResult[] = await api.forwardMessages({ from_chat: chatJid, message_ids: $state.snapshot(messageIds), to: $state.snapshot(selected) });
      // One result per message × target, target-major order.
      const per = messageIds.length || 1;
      results = selected.map((jid, i) => {
        const slice = res.slice(i * per, i * per + per);
        const bad = slice.find((r) => !r.ok);
        return { jid, name: names.get(jid) ?? jid, ok: !bad, message: bad?.message ?? slice[0]?.message ?? "Forwarded" };
      });
    } catch (e) {
      toasts.error(e, "Forward failed");
    } finally {
      busy = false;
    }
  }
  const close = () => (app.forwardDialog = null);
</script>

<Modal title="Forward {messageIds.length} message{messageIds.length === 1 ? '' : 's'}" onclose={close} width="28rem">
  <div class="space-y-3" data-testid="forward-dialog">
    {#if results}
      <ul class="space-y-1.5" aria-label="Forward results">
        {#each results as r (r.jid)}
          <li class="flex items-center gap-2 rounded-md bg-surface-2 px-2.5 py-1.5 text-[13px]" data-testid="forward-result" data-ok={r.ok}>
            {#if r.ok}<Check size={15} class="shrink-0 text-accent" />{:else}<CircleX size={15} class="shrink-0 text-danger" />{/if}
            <span class="min-w-0 flex-1 truncate font-medium">{r.name}</span>
            <span class="truncate text-xs {r.ok ? 'text-muted' : 'text-danger'}">{r.message}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <div class="relative">
        <Search size={14} class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
        <input class="input pl-8" placeholder="Search chats" aria-label="Search chats" bind:value={query} data-autofocus data-testid="forward-search" />
      </div>
      <p class="text-xs {limitHit ? 'text-danger' : 'text-muted'}" data-testid="forward-count" aria-live="polite">
        {selected.length} of {MAX_FORWARD_TARGETS} chats selected{limitHit ? " — you can forward to at most 5 chats" : ""}
      </p>
      <ul class="max-h-80 overflow-y-auto rounded-md border border-border" aria-label="Chats">
        {#each chats as c (c.jid)}
          {@const on = selected.includes(c.jid)}
          <li>
            <label class="flex cursor-pointer items-center gap-2.5 px-2.5 py-1.5 hover:bg-surface-2 {on ? 'bg-accent-soft' : ''}" data-testid="forward-target">
              <input type="checkbox" checked={on} onchange={(e) => {
                toggle(c.jid);
                e.currentTarget.checked = selected.includes(c.jid); // refuse the 6th target visually too
              }} aria-label={chatLabel(c)} />
              <Avatar name={chatLabel(c)} id={c.jid} url={c.avatar_url} type={c.chat_type} size={30} />
              <span class="min-w-0 flex-1 truncate text-[13px]">{chatLabel(c)}</span>
            </label>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={close}>{results ? "Done" : "Cancel"}</button>
    {#if !results}
      <button class="btn btn-primary" disabled={busy || selected.length === 0} onclick={send} data-testid="forward-send">
        {#if busy}<Spinner size={14} label="Forwarding" />{/if} Forward
      </button>
    {/if}
  {/snippet}
</Modal>
