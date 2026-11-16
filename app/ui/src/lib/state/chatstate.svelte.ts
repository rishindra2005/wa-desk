// Archive / pin / mute actions with optimistic updates. Lists render `apply(chat)`, which layers
// in-flight patches over server data; a failed call rolls its patch back and shows a toast.

import { Archive, ArchiveRestore, Bell, BellOff, Pin, PinOff } from "@lucide/svelte";
import * as api from "../api";
import type { ChatSummary } from "../bindings/ChatSummary";
import { isMuted, muteUntil, MUTE_CHOICES } from "../utils/chatstate";
import type { MenuItem } from "../utils/menu";
import { toasts } from "./toasts.svelte";

type Patch = Partial<Pick<ChatSummary, "archived" | "pinned" | "muted_until">>;

class ChatStateStore {
  /** Optimistic patches by chat JID (kept until the next list load confirms them). */
  patches = $state<Record<string, Patch>>({});
  private pending = new Set<string>();
  /** Chats in the archive (for the "Archived" row). */
  archiveCount = $state(0);

  /** The chat with any optimistic patch applied. */
  apply<T extends ChatSummary>(c: T): T {
    const p = this.patches[c.jid];
    return p ? { ...c, ...p } : c;
  }

  async refreshArchiveCount(): Promise<void> {
    try {
      this.archiveCount = await api.archiveCount();
    } catch {
      /* non-critical */
    }
  }

  /** Called after a list load: drop patches whose request has finished. */
  settle(): void {
    const next: Record<string, Patch> = {};
    for (const [jid, p] of Object.entries(this.patches)) if (this.pending.has(jid)) next[jid] = p;
    this.patches = next;
  }

  private async run(c: ChatSummary, patch: Patch, call: () => Promise<void>, ok: string): Promise<void> {
    const before = this.patches[c.jid];
    const wasArchived = this.apply(c).archived;
    this.patches = { ...this.patches, [c.jid]: { ...before, ...patch } };
    if (patch.archived !== undefined && patch.archived !== wasArchived) this.archiveCount += patch.archived ? 1 : -1;
    this.pending.add(c.jid);
    try {
      await call();
      toasts.success(ok);
    } catch (e) {
      const rest = { ...this.patches };
      if (before) rest[c.jid] = before;
      else delete rest[c.jid];
      this.patches = rest;
      toasts.error(e);
    } finally {
      this.pending.delete(c.jid);
      void this.refreshArchiveCount();
    }
  }

  setArchived(c: ChatSummary, archived: boolean): Promise<void> {
    // WhatsApp unpins on archive.
    const patch: Patch = archived ? { archived, pinned: false } : { archived };
    return this.run(c, patch, () => api.setArchived(c.jid, archived), archived ? "Chat archived" : "Chat unarchived");
  }

  setPinned(c: ChatSummary, pinned: boolean): Promise<void> {
    return this.run(c, { pinned }, () => api.setPinned(c.jid, pinned), pinned ? "Chat pinned" : "Chat unpinned");
  }

  setMuted(c: ChatSummary, until: number | null): Promise<void> {
    return this.run(c, { muted_until: until }, () => api.setMuted(c.jid, until), until === null ? "Chat unmuted" : "Chat muted");
  }

  /** Archive / Pin / Mute entries for the chat context menu and the thread ⋮ menu. */
  menuItems(chat: ChatSummary): MenuItem[] {
    const c = this.apply(chat);
    const muted = isMuted(c.muted_until, Math.floor(Date.now() / 1000));
    const items: MenuItem[] = [
      { id: "archive", label: c.archived ? "Unarchive" : "Archive", icon: c.archived ? ArchiveRestore : Archive, onselect: () => void this.setArchived(c, !c.archived) },
      { id: "pin", label: c.pinned ? "Unpin" : "Pin", icon: c.pinned ? PinOff : Pin, onselect: () => void this.setPinned(c, !c.pinned) },
    ];
    if (muted) items.push({ id: "unmute", label: "Unmute", icon: Bell, onselect: () => void this.setMuted(c, null) });
    else {
      items.push({
        id: "mute",
        label: "Mute",
        icon: BellOff,
        submenu: MUTE_CHOICES.map((m) => ({
          id: `mute-${m.id}`,
          label: m.label,
          onselect: () => void this.setMuted(c, muteUntil(m.id, Math.floor(Date.now() / 1000))),
        })),
      });
    }
    return items;
  }
}

export const chatState = new ChatStateStore();
