<script lang="ts">
  import { Paperclip, SendHorizontal, X, Reply, FileText } from "@lucide/svelte";
  import * as api from "../../api";
  import type { Message } from "../../bindings/Message";
  import type { ChatSummary } from "../../bindings/ChatSummary";
  import Modal from "../../components/Modal.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import { pickFile } from "../../platform";
  import { toasts } from "../../state/toasts.svelte";
  import { baseName } from "../../utils/text";

  interface Props {
    chat: ChatSummary;
    replyTo: Message | null;
    confirmSends: boolean;
    oncancelreply: () => void;
    onsent?: () => void;
  }
  let { chat, replyTo, confirmSends, oncancelreply, onsent }: Props = $props();

  let text = $state("");
  let attachment = $state<string | null>(null);
  let sending = $state(false);
  let confirming = $state(false);
  let textarea: HTMLTextAreaElement | undefined = $state();

  const canSend = $derived(!sending && (text.trim().length > 0 || attachment !== null));

  // Reset draft when switching chats.
  let lastJid = "";
  $effect(() => {
    if (chat.jid !== lastJid) {
      lastJid = chat.jid;
      text = "";
      attachment = null;
    }
  });

  $effect(() => {
    if (replyTo) textarea?.focus();
  });

  function autosize() {
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${Math.min(textarea.scrollHeight, 180)}px`;
  }

  function requestSend() {
    if (!canSend) return;
    if (confirmSends && (chat.is_group || attachment)) confirming = true;
    else void doSend();
  }

  async function doSend() {
    confirming = false;
    sending = true;
    try {
      const res = await api.sendMessage({
        recipient: chat.jid,
        text: text.trim() ? text : null,
        attachment,
        reply_to: replyTo?.id ?? null,
        mentions: [],
      });
      if (!res.ok) {
        toasts.push({ kind: "error", title: "Not sent", message: res.message });
        return;
      }
      text = "";
      attachment = null;
      oncancelreply();
      onsent?.();
      queueMicrotask(autosize);
    } catch (e) {
      toasts.error(e, "Message not sent");
    } finally {
      sending = false;
      textarea?.focus();
    }
  }

  async function attach() {
    try {
      const p = await pickFile("Attach a file");
      if (p) attachment = p;
    } catch (e) {
      toasts.error(e);
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      requestSend();
    } else if (e.key === "Escape" && replyTo) {
      e.preventDefault();
      oncancelreply();
    }
  }
</script>

<div class="border-t border-border bg-surface px-3 py-2" data-testid="composer">
  {#if replyTo || attachment}
    <div class="mb-2 flex flex-wrap gap-2">
      {#if replyTo}
        <div class="flex min-w-0 max-w-full flex-1 items-center gap-2 rounded-md border-l-[3px] border-accent bg-surface-2 px-2 py-1" data-testid="reply-chip">
          <Reply size={14} class="shrink-0 text-accent" />
          <div class="min-w-0 flex-1">
            <p class="text-xs font-semibold text-accent-text">Replying to {replyTo.is_from_me ? "yourself" : replyTo.sender_name}</p>
            <p class="truncate text-xs text-muted">{replyTo.content || replyTo.media?.filename || "Media"}</p>
          </div>
          <button class="icon-btn h-6 w-6" aria-label="Cancel reply" onclick={oncancelreply}><X size={14} /></button>
        </div>
      {/if}
      {#if attachment}
        <div class="flex min-w-0 items-center gap-2 rounded-md bg-surface-2 px-2 py-1" data-testid="attachment-chip">
          <FileText size={14} class="shrink-0 text-muted" />
          <span class="truncate text-xs" title={attachment}>{baseName(attachment)}</span>
          <button class="icon-btn h-6 w-6" aria-label="Remove attachment" onclick={() => (attachment = null)}><X size={14} /></button>
        </div>
      {/if}
    </div>
  {/if}
  <div class="flex items-end gap-2">
    <button class="icon-btn" aria-label="Attach file" onclick={attach}><Paperclip size={18} /></button>
    <textarea
      bind:this={textarea}
      bind:value={text}
      oninput={autosize}
      onkeydown={onKey}
      rows="1"
      class="input max-h-44 min-h-9 flex-1 resize-none py-2 leading-snug"
      placeholder="Message {chat.name}"
      aria-label="Message"
      dir="auto"
      data-testid="composer-input"
    ></textarea>
    <button class="btn btn-primary h-9 w-9 shrink-0 p-0" aria-label="Send" disabled={!canSend} onclick={requestSend} data-testid="send-btn">
      {#if sending}<Spinner size={16} label="Sending" />{:else}<SendHorizontal size={17} />{/if}
    </button>
  </div>
  <p class="mt-1 hidden text-[11px] text-faint sm:block">Enter to send · Shift+Enter for a new line</p>
</div>

{#if confirming}
  <Modal title="Send this message?" onclose={() => (confirming = false)}>
    <div class="space-y-2 text-[13px]" data-testid="confirm-send">
      <p>
        {#if chat.is_group}You're about to send to the group <strong>{chat.name}</strong>. Everyone in it will see this.{:else}You're about to send to <strong>{chat.name}</strong>.{/if}
      </p>
      {#if attachment}<p class="text-muted">Attachment: <span class="font-medium text-fg">{baseName(attachment)}</span></p>{/if}
      {#if text.trim()}<blockquote class="max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-surface-2 p-2 text-muted">{text}</blockquote>{/if}
      <p class="text-xs text-faint">You can turn this confirmation off in Settings.</p>
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (confirming = false)}>Cancel</button>
      <button class="btn btn-primary" data-autofocus onclick={doSend} data-testid="confirm-send-btn">Send</button>
    {/snippet}
  </Modal>
{/if}
