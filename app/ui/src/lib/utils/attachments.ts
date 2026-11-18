// Pure helpers for the composer's multi-file attachment tray.

import type { SendReq } from "../bindings/SendReq";

/** Most files one send can carry (WhatsApp allows 100 media; we stay well below). */
export const MAX_ATTACHMENTS = 30;
/** Largest clipboard/drag payload we read into memory (matches the Rust side). */
export const MAX_PASTE_BYTES = 50 * 1024 * 1024;

/** One file waiting in the tray. `path` is always an absolute path the backend can stage. */
export interface Attachment {
  id: string;
  path: string;
  name: string;
  /** Object URL for a thumbnail (pasted or dropped images only). */
  previewUrl?: string;
}

/** Append `items`, skipping paths already present and respecting the cap. */
export function addAttachments(list: Attachment[], items: Attachment[]): { list: Attachment[]; skippedDuplicates: number; skippedOverLimit: number } {
  const out = [...list];
  const seen = new Set(out.map((a) => a.path));
  let skippedDuplicates = 0;
  let skippedOverLimit = 0;
  for (const it of items) {
    if (seen.has(it.path)) skippedDuplicates++;
    else if (out.length >= MAX_ATTACHMENTS) skippedOverLimit++;
    else {
      seen.add(it.path);
      out.push(it);
    }
  }
  return { list: out, skippedDuplicates, skippedOverLimit };
}

/**
 * Build one request per file, in order. The typed text is the caption of the
 * FIRST file only (the rest are sent bare) and only the first file quotes the
 * reply target, so a batch reads as one message followed by its siblings.
 * With no files the text is sent on its own.
 */
export function buildSendReqs(recipient: string, text: string, files: { path: string }[], replyTo: string | null): SendReq[] {
  const caption = text.trim() ? text : null;
  if (files.length === 0) return [{ recipient, text: caption, attachment: null, reply_to: replyTo, mentions: [] }];
  return files.map((f, i) => ({
    recipient,
    text: i === 0 ? caption : null,
    attachment: f.path,
    reply_to: i === 0 ? replyTo : null,
    mentions: [],
  }));
}

/**
 * Local paths from `text/uri-list` (or plain text) clipboard/drop data. Returns
 * [] unless every non-comment line is a `file://` URI, so ordinary pasted text
 * and web links are never mistaken for files.
 */
export function parseFileUris(data: string): string[] {
  const lines = data
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"));
  if (lines.length === 0) return [];
  const out: string[] = [];
  for (const line of lines) {
    const m = /^file:\/\/(?:localhost)?(\/.*)$/i.exec(line);
    if (!m?.[1]) return [];
    let path: string;
    try {
      path = decodeURIComponent(m[1]);
    } catch {
      return [];
    }
    if (path.includes("\0") || path.endsWith("/")) return [];
    out.push(path);
  }
  return out;
}

/** Base64 of a blob without the data-URL prefix. */
export function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onerror = () => reject(r.error ?? new Error("Could not read the pasted item"));
    r.onload = () => {
      const s = typeof r.result === "string" ? r.result : "";
      resolve(s.slice(s.indexOf(",") + 1));
    };
    r.readAsDataURL(blob);
  });
}
