// Pure helpers for archived / pinned / muted chat state.

import type { ChatSummary } from "../bindings/ChatSummary";

/** WhatsApp allows at most this many pinned chats. */
export const MAX_PINS = 3;
/** `muted_until` sentinel for "muted forever". */
export const MUTE_FOREVER = -1;

export type MuteChoice = "8h" | "1w" | "always";

export const MUTE_CHOICES: { id: MuteChoice; label: string }[] = [
  { id: "8h", label: "8 hours" },
  { id: "1w", label: "1 week" },
  { id: "always", label: "Always" },
];

/** True when `mutedUntil` (epoch seconds; -1 = forever; null = not muted) is still in effect at `nowSec`. */
export function isMuted(mutedUntil: number | null | undefined, nowSec: number): boolean {
  if (mutedUntil === null || mutedUntil === undefined) return false;
  if (mutedUntil === MUTE_FOREVER) return true;
  return mutedUntil > nowSec;
}

/** The `until` argument for `set_muted` for a menu choice. */
export function muteUntil(choice: MuteChoice, nowSec: number): number {
  switch (choice) {
    case "8h": return nowSec + 8 * 3600;
    case "1w": return nowSec + 7 * 86400;
    case "always": return MUTE_FOREVER;
  }
}

/** Pinned chats first, then newest activity first (stable for ties). */
export function sortChats<T extends Pick<ChatSummary, "pinned" | "last_message_ts">>(list: T[]): T[] {
  return list
    .map((c, i) => ({ c, i }))
    .sort((a, b) => Number(b.c.pinned) - Number(a.c.pinned) || (b.c.last_message_ts ?? 0) - (a.c.last_message_ts ?? 0) || a.i - b.i)
    .map((x) => x.c);
}
