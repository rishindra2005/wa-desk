<script lang="ts">
  import { MessagesSquare } from "@lucide/svelte";
  import * as api from "../../api";
  import type { ChatSummary } from "../../bindings/ChatSummary";
  import ChatList from "./ChatList.svelte";
  import Thread from "./Thread.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import { app } from "../../state/app.svelte";
  import { toasts } from "../../state/toasts.svelte";

  let chat = $state<ChatSummary | null>(null);
  let wasUnread = $state(false);
  let listRef: ChatList | undefined = $state();

  async function markRead(c: ChatSummary) {
    if (!c.unread) return;
    listRef?.markLocalRead(c.jid);
    try {
      await api.markRead(c.jid);
      void app.refreshUnread();
    } catch {
      /* non-critical */
    }
  }

  function select(c: ChatSummary) {
    app.openChatJid = c.jid;
    wasUnread = c.unread;
    chat = c;
    if (app.profileJid && app.profileJid !== c.jid) app.openProfile(c.jid);
    void markRead(c);
  }

  // Resolve app.openChatJid (from search, palette, automations) into a summary.
  $effect(() => {
    const jid = app.openChatJid;
    if (!jid || chat?.jid === jid) return;
    api.getChat(jid).then(
      (c) => {
        if (app.openChatJid === jid) {
          wasUnread = c.unread;
          chat = c;
          if (app.profileJid && app.profileJid !== c.jid) app.openProfile(c.jid);
          void markRead(c);
        }
      },
      (e) => toasts.error(e, "Couldn't open chat"),
    );
  });

  const jump = $derived(app.jump && chat && app.jump.chatJid === chat.jid ? app.jump : null);
</script>

<div class="flex h-full min-h-0">
  <div class="w-[340px] shrink-0 border-r border-border bg-surface">
    <ChatList bind:this={listRef} selected={chat?.jid ?? null} onselect={select} />
  </div>
  <div class="min-w-0 flex-1">
    {#if chat}
      <Thread {chat} {wasUnread} jumpId={jump?.messageId ?? null} jumpNonce={jump?.nonce ?? 0} />
    {:else}
      <div class="h-full" style="background: var(--thread-bg)">
        <EmptyState icon={MessagesSquare} title="Pick a conversation" message="Choose a chat on the left, or press Ctrl+K to jump to one." />
      </div>
    {/if}
  </div>
</div>
