// Shared media actions with consistent toasts.

import * as api from "./api";
import type { MediaRef } from "./bindings/MediaRef";
import { pickSavePath } from "./platform";
import { toasts } from "./state/toasts.svelte";

/** Download (ensure) a media file; returns the updated ref or null on failure (toast shown). */
export async function downloadMedia(chatJid: string, messageId: string): Promise<MediaRef | null> {
  try {
    const ref = await api.ensureMedia(chatJid, messageId);
    if (ref.source === "missing") {
      toasts.push({ kind: "warn", title: "Still not available", message: "The phone didn't send the file. Try again later." });
      return null;
    }
    return ref;
  } catch (e) {
    toasts.error(e, "Download failed");
    return null;
  }
}

/** Ask for a destination and save a copy. */
export async function saveMedia(chatJid: string, messageId: string, filename: string | null): Promise<string | null> {
  try {
    const dest = await pickSavePath(filename ?? "download");
    if (!dest) return null;
    const saved = await api.saveMediaAs(chatJid, messageId, dest);
    toasts.success("Saved", saved);
    return saved;
  } catch (e) {
    toasts.error(e, "Couldn't save");
    return null;
  }
}

export async function openExternal(chatJid: string, messageId: string): Promise<void> {
  try {
    await api.openMediaExternal(chatJid, messageId);
  } catch (e) {
    toasts.error(e, "Couldn't open");
  }
}

export function mediaLabel(m: MediaRef): string {
  return { image: "Photo", video: "Video", audio: "Voice message", document: "Document", sticker: "Sticker" }[m.kind];
}

/** Put the actual image (not a path) on the system clipboard so it pastes into other apps. */
export async function copyImageToClipboard(chatJid: string, messageId: string): Promise<boolean> {
  try {
    await api.copyImage(chatJid, messageId);
    toasts.success("Image copied");
    return true;
  } catch (e) {
    toasts.error(e, "Couldn't copy the image");
    return false;
  }
}
