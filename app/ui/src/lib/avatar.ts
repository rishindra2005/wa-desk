// Avatar URL helpers and a small in-memory cache of pictures that failed to load,
// so a chat without a profile picture is requested once (per retry window), not on every render.

import type { ChatType } from "./bindings/ChatType";

/** Contract URL of a profile picture (404 when the chat has none). */
export function contractAvatarUrl(jid: string): string {
  return `wa-media://localhost/a/${encodeURIComponent(jid)}`;
}

let resolver: ((jid: string) => string) | null = null;

/** The mock backend installs a resolver that returns generated pictures. */
export function setAvatarResolver(fn: ((jid: string) => string) | null): void {
  resolver = fn;
}

/** Best avatar URL for a JID when only the JID is known (search hits, senders). */
export function avatarUrl(jid: string, given?: string | null): string {
  if (given) return given;
  return resolver ? resolver(jid) : contractAvatarUrl(jid);
}

/** Full-size picture URL (`?full=1`); data/blob URLs are returned unchanged. */
export function fullAvatarUrl(url: string): string {
  if (/^(data|blob):/.test(url)) return url;
  return url + (url.includes("?") ? "&" : "?") + "full=1";
}

/**
 * A failed picture is not requested again for this long. It is retried after
 * that (on the next render), so a transient failure (bridge offline or slow at
 * startup) does not hide a logo for the rest of the session.
 */
export const AVATAR_RETRY_MS = 5 * 60 * 1000;

const failed = new Map<string, number>();
const loaded = new Set<string>();

export function avatarFailed(url: string, now = Date.now()): boolean {
  const at = failed.get(url);
  if (at === undefined) return false;
  if (now - at < AVATAR_RETRY_MS) return true;
  failed.delete(url);
  return false;
}
export function markAvatarFailed(url: string, now = Date.now()): void {
  failed.set(url, now);
  loaded.delete(url);
}
export function avatarLoaded(url: string): boolean {
  return loaded.has(url);
}
export function markAvatarLoaded(url: string): void {
  loaded.add(url);
}
/** Forget cached results (e.g. after "Refresh names"). */
export function resetAvatarCache(): void {
  failed.clear();
  loaded.clear();
}

export function isGroupLike(t: ChatType | undefined | null): boolean {
  return t === "group" || t === "community" || t === "channel" || t === "broadcast";
}
