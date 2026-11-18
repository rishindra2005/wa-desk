<script lang="ts">
  import { untrack } from "svelte";
  import { Paperclip, SendHorizontal, X, Reply, FileText, Image as ImageIcon, Loader } from "@lucide/svelte";
  import type { Message } from "../../bindings/Message";
  import type { ChatSummary } from "../../bindings/ChatSummary";
  import Modal from "../../components/Modal.svelte";
  import type { SendReq } from "../../bindings/SendReq";
  import { loadDraft, saveDraft, clearDraft } from "../../utils/drafts";
  import { pickFiles } from "../../platform";
  import * as api from "../../api";
  import { toasts } from "../../state/toasts.svelte";
  import { baseName, extOf } from "../../utils/text";
  import { overlayCount } from "../../components/overlay";
  import { addAttachments, blobToBase64, buildSendReqs, MAX_ATTACHMENTS, MAX_PASTE_BYTES, parseFileUris, type Attachment } from "../../utils/attachments";
  import { chatLabel } from "../../utils/format";

  interface Props {
    chat: ChatSummary;
    replyTo: Message | null;
    confirmSends: boolean;
    oncancelreply: () => void;
    /** Hand the request to the thread, which shows it optimistically (clock, then ticks or retry). */
    onsend: (req: SendReq, replyPreview: Message | null) => void;
  }
  let { chat, replyTo, confirmSends, oncancelreply, onsend }: Props = $props();

  let text = $state("");
  /** Files waiting in the tray, sent one after another (caption on the first). */
  let attachments = $state<Attachment[]>([]);
  /** Pasted/dropped items still being written to the outbox. */
  let busy = $state(0);
  let dragging = $state(false);
  let confirming = $state(false);
  let textarea: HTMLTextAreaElement | undefined = $state();

  const canSend = $derived((text.trim().length > 0 || attachments.length > 0) && busy === 0);

  let seq = 0;
  const newId = () => `att:${Date.now()}:${seq++}`;

  function revoke(list: Attachment[]) {
    for (const a of list) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
  }

  /** Empty the tray. Reads `attachments`, so effects must call it inside untrack(). */
  function clearTray() {
    revoke(attachments);
    attachments = [];
  }

  function addItems(items: Attachment[]) {
    const r = addAttachments(attachments, items);
    const kept = new Set(r.list.map((a) => a.id));
    revoke(items.filter((a) => !kept.has(a.id)));
    attachments = r.list;
    if (r.skippedOverLimit > 0) toasts.push({ kind: "warn", title: `Up to ${MAX_ATTACHMENTS} files per message`, message: `${r.skippedOverLimit} not added.` });
  }

  function removeAttachment(id: string) {
    const gone = attachments.find((a) => a.id === id);
    if (gone) revoke([gone]);
    attachments = attachments.filter((a) => a.id !== id);
  }

  function addPaths(paths: string[]) {
    addItems(paths.map((path) => ({ id: newId(), path, name: baseName(path) })));
  }

  /** Clipboard/drop files have no path: the backend writes them to a private outbox file. */
  async function addFiles(files: File[]) {
    busy++;
    try {
      const items: Attachment[] = [];
      for (const f of files) {
        const label = f.name || "pasted item";
        if (f.size > MAX_PASTE_BYTES) {
          toasts.push({ kind: "error", title: `Couldn't attach ${label}`, message: "Pasted or dropped items can be up to 50 MiB. Use the attach button for larger files." });
          continue;
        }
        try {
          const path = await api.stagePastedFile(f.name || null, f.type || null, await blobToBase64(f));
          const item: Attachment = { id: newId(), path, name: baseName(path) };
          if (f.type.startsWith("image/")) item.previewUrl = URL.createObjectURL(f);
          items.push(item);
        } catch (e) {
          toasts.error(e, `Couldn't attach ${label}`);
        }
      }
      addItems(items);
    } finally {
      busy--;
    }
  }

  function isEditableElsewhere(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el || el === textarea) return false;
    return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable;
  }

  function onPaste(e: ClipboardEvent) {
    if (isEditableElsewhere(e.target) || overlayCount() > 0) return;
    const cd = e.clipboardData;
    if (!cd) return;
    // Files copied in a file manager arrive as a uri-list (or file:// text).
    const paths = parseFileUris(cd.getData("text/uri-list") || cd.getData("text/plain"));
    const files = [...cd.files];
    if (paths.length > 0) {
      e.preventDefault();
      addPaths(paths);
      return;
    }
    if (files.length === 0) return; // ordinary text paste
    // Spreadsheet-style copies carry text plus a picture of it: keep the text.
    if (cd.getData("text/plain").trim() && files.every((f) => f.type.startsWith("image/"))) return;
    e.preventDefault();
    void addFiles(files);
  }

  function hasFiles(e: DragEvent): boolean {
    return [...(e.dataTransfer?.types ?? [])].includes("Files");
  }
  function onDragOver(e: DragEvent) {
    if (!hasFiles(e) || overlayCount() > 0) return;
    e.preventDefault();
    dragging = true;
  }
  function onDrop(e: DragEvent) {
    dragging = false;
    if (!hasFiles(e) || overlayCount() > 0) return;
    e.preventDefault();
    const dt = e.dataTransfer;
    if (!dt) return;
    const paths = parseFileUris(dt.getData("text/uri-list"));
    if (paths.length > 0) addPaths(paths);
    else void addFiles([...dt.files]);
  }

  // The desktop shell swallows HTML5 drops and reports absolute paths instead.
  $effect(() => {
    let off: (() => void) | undefined;
    let dead = false;
    void (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        const un = await getCurrentWebview().onDragDropEvent((ev) => {
          const p = ev.payload;
          if (p.type === "enter" || p.type === "over") dragging = overlayCount() === 0;
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            if (overlayCount() === 0 && p.paths.length > 0) addPaths(p.paths);
          }
        });
        if (dead) un();
        else off = un;
      } catch {
        /* not running inside Tauri (or no webview): HTML5 handlers cover it */
      }
    })();
    return () => {
      dead = true;
      off?.();
    };
  });

  $effect(() => () => untrack(clearTray));

  // Reset draft when switching chats.
  let lastJid = "";
  $effect(() => {
    if (chat.jid !== lastJid) {
      lastJid = chat.jid;
      text = loadDraft(chat.jid);
      untrack(clearTray);
      queueMicrotask(autosize);
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
    if (confirmSends && (chat.is_group || attachments.length > 0)) confirming = true;
    else void doSend();
  }

  function doSend() {
    confirming = false;
    const reqs = buildSendReqs(chat.jid, text, attachments, replyTo?.id ?? null);
    const quoted = replyTo;
    text = "";
    // The files now live in the thread's send queue; drop the tray (thumbnails included).
    clearTray();
    clearDraft(chat.jid);
    oncancelreply();
    queueMicrotask(autosize);
    reqs.forEach((req, i) => onsend(req, i === 0 ? quoted : null));
    textarea?.focus();
  }

  async function attach() {
    try {
      const picked = await pickFiles("Attach files");
      if (picked.length > 0) addPaths(picked);
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

<svelte:window onpaste={onPaste} ondragover={onDragOver} ondrop={onDrop} ondragleave={(e) => { if (!e.relatedTarget) dragging = false; }} />

<div class="border-t border-border bg-surface px-3 py-2" data-testid="composer">
  {#if replyTo}
    <div class="mb-2 flex flex-wrap gap-2">
      <div class="flex min-w-0 max-w-full flex-1 items-center gap-2 rounded-md border-l-[3px] border-accent bg-surface-2 px-2 py-1" data-testid="reply-chip">
        <Reply size={14} class="shrink-0 text-accent" />
        <div class="min-w-0 flex-1">
          <p class="text-xs font-semibold text-accent-text">Replying to {replyTo.is_from_me ? "yourself" : replyTo.sender_name}</p>
          <p class="truncate text-xs text-muted">{replyTo.content || replyTo.media?.filename || "Media"}</p>
        </div>
        <button class="icon-btn h-6 w-6" aria-label="Cancel reply" onclick={oncancelreply}><X size={14} /></button>
      </div>
    </div>
  {/if}
  {#if attachments.length > 0 || busy > 0}
    <div class="mb-2" data-testid="attachment-tray">
      <div class="mb-1 flex items-center justify-between text-xs text-muted">
        <span data-testid="attachment-count">{attachments.length} file{attachments.length === 1 ? "" : "s"}{attachments.length > 1 && text.trim() ? " · caption goes on the first" : ""}</span>
        {#if attachments.length > 1}<button class="hover:text-fg" onclick={clearTray} data-testid="attachment-clear">Remove all</button>{/if}
      </div>
      <ul class="flex max-h-28 flex-wrap gap-2 overflow-y-auto">
        {#each attachments as a (a.id)}
          <li class="flex min-w-0 max-w-56 items-center gap-2 rounded-md bg-surface-2 py-1 pl-1 pr-1" data-testid="attachment-chip">
            {#if a.previewUrl}
              <img src={a.previewUrl} alt="" class="h-8 w-8 shrink-0 rounded object-cover" />
            {:else if ["jpg", "jpeg", "png", "webp", "gif", "heic", "bmp"].includes(extOf(a.name))}
              <ImageIcon size={16} class="mx-2 shrink-0 text-muted" />
            {:else}
              <FileText size={16} class="mx-2 shrink-0 text-muted" />
            {/if}
            <span class="truncate text-xs" title={a.path}>{a.name}</span>
            <button class="icon-btn h-6 w-6 shrink-0" aria-label="Remove {a.name}" onclick={() => removeAttachment(a.id)} data-testid="attachment-remove"><X size={14} /></button>
          </li>
        {/each}
        {#if busy > 0}
          <li class="flex items-center gap-2 rounded-md bg-surface-2 px-2 py-1 text-xs text-muted" data-testid="attachment-busy"><Loader size={14} class="animate-spin" /> Adding…</li>
        {/if}
      </ul>
    </div>
  {/if}
  {#if dragging}
    <div class="mb-2 rounded-md border-2 border-dashed border-accent bg-accent/10 px-3 py-3 text-center text-sm text-accent-text" data-testid="drop-hint">Drop files to attach</div>
  {/if}
  <div class="flex items-end gap-2">
    <button class="icon-btn" aria-label="Attach files" onclick={attach}><Paperclip size={18} /></button>
    <textarea
      bind:this={textarea}
      bind:value={text}
      oninput={() => {
        autosize();
        saveDraft(chat.jid, text);
      }}
      onkeydown={onKey}
      rows="1"
      class="input max-h-44 min-h-9 flex-1 resize-none py-2 leading-snug"
      placeholder="Message {chatLabel(chat)}"
      aria-label="Message"
      dir="auto"
      data-testid="composer-input"
    ></textarea>
    <button class="btn btn-primary h-9 w-9 shrink-0 p-0" aria-label="Send" disabled={!canSend} onclick={requestSend} data-testid="send-btn">
      <SendHorizontal size={17} />
    </button>
  </div>
  <p class="mt-1 hidden text-[11px] text-faint sm:block">Enter to send · Shift+Enter for a new line</p>
</div>

{#if confirming}
  <Modal title={attachments.length > 1 ? `Send ${attachments.length} files?` : "Send this message?"} onclose={() => (confirming = false)}>
    <div class="space-y-2 text-[13px]" data-testid="confirm-send">
      <p>
        {#if chat.is_group}You're about to send to the group <strong>{chatLabel(chat)}</strong>. Everyone in it will see this.{:else}You're about to send to <strong>{chatLabel(chat)}</strong>.{/if}
      </p>
      {#if attachments.length === 1}<p class="text-muted">Attachment: <span class="font-medium text-fg">{attachments[0]?.name}</span></p>{/if}
      {#if attachments.length > 1}
        <ul class="max-h-32 list-inside list-disc overflow-auto text-muted" data-testid="confirm-files">{#each attachments as a (a.id)}<li class="truncate"><span class="font-medium text-fg">{a.name}</span></li>{/each}</ul>
        <p class="text-xs text-faint">They are sent one after another{text.trim() ? ", with your text as the caption of the first" : ""}.</p>
      {/if}
      {#if text.trim()}<blockquote class="max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-surface-2 p-2 text-muted">{text}</blockquote>{/if}
      <p class="text-xs text-faint">You can turn this confirmation off in Settings.</p>
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (confirming = false)}>Cancel</button>
      <button class="btn btn-primary" data-autofocus onclick={doSend} data-testid="confirm-send-btn">Send</button>
    {/snippet}
  </Modal>
{/if}
