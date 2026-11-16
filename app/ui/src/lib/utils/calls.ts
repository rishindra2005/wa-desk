// Pure helpers for the call log, in-thread call pills, the ringing banner and the missed badge.

import type { CallMedia } from "../bindings/CallMedia";
import type { CallQuery } from "../bindings/CallQuery";
import type { CallRecord } from "../bindings/CallRecord";
import { formatDuration, timeOfDay } from "./format";

/** Shown wherever a user might expect a call button. */
export const CALL_FROM_PHONE_HINT = "Calls can only be placed from your phone";

export type CallFilter = "all" | "missed" | "video";

/** Query for a log filter (All / Missed / Video). */
export function filterQuery(filter: CallFilter, chatJid: string | null = null): CallQuery {
  return { chat_jid: chatJid, missed_only: filter === "missed", media: filter === "video" ? "video" : null, cursor: null, limit: 100 };
}

/** A call the user did not get to take: missed or declined, and not placed by us. */
export function isMissedForMe(c: Pick<CallRecord, "result" | "is_from_me">): boolean {
  return !c.is_from_me && (c.result === "missed" || c.result === "rejected");
}

/** Ringing right now for us (incoming, not yet answered or ended). */
export function isRinging(c: Pick<CallRecord, "result" | "is_from_me">): boolean {
  return !c.is_from_me && c.result === "in_progress";
}

/** Missed calls newer than the last visit to the Calls view. */
export function missedSince(calls: readonly Pick<CallRecord, "result" | "is_from_me" | "ts">[], lastSeenTs: number): number {
  return calls.filter((c) => isMissedForMe(c) && c.ts > lastSeenTs).length;
}

/** "voice" / "video". */
export function mediaLabel(m: CallMedia): string {
  return m === "video" ? "video" : "voice";
}

export type Direction = "missed" | "declined" | "incoming" | "outgoing" | "ringing";

/** How the call looks in the list (drives icon and colour). */
export function direction(c: CallRecord): Direction {
  if (c.result === "in_progress") return c.is_from_me ? "outgoing" : "ringing";
  if (c.is_from_me) return "outgoing";
  if (c.result === "missed") return "missed";
  if (c.result === "rejected") return "declined";
  return "incoming";
}

/** Call duration for rows and pills: "45 sec", "3 min", "1 h 5 min". Empty when unknown or zero. */
export function callDuration(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs) || secs <= 0) return "";
  const r = Math.round(secs);
  if (r < 60) return `${r} sec`;
  if (r < 3600) return `${Math.max(1, Math.round(r / 60))} min`;
  return formatDuration(secs);
}

/** Descriptive title: "Missed voice call", "Video call", "Unanswered voice call", "Group video call". */
export function callTitle(c: CallRecord): string {
  const kind = mediaLabel(c.media);
  const group = c.is_group ? "group " : "";
  const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
  switch (direction(c)) {
    case "missed":
      return `Missed ${group}${kind} call`;
    case "declined":
      return `Declined ${group}${kind} call`;
    case "ringing":
      return cap(`${c.is_group ? "group " : "incoming "}${kind} call`);
    case "outgoing":
      return c.result === "missed" || c.result === "rejected" ? `Unanswered ${kind} call` : cap(`outgoing ${group}${kind} call`);
    default:
      return cap(`${group}${kind} call`);
  }
}

/** Centered thread pill text: "📞 Missed voice call · 10:42" / "📹 Video call · 3 min". */
export function pillText(c: CallRecord): string {
  const icon = c.media === "video" ? "📹" : "📞";
  const dur = c.result === "answered" || c.result === "ended" ? callDuration(c.duration_secs) : "";
  return `${icon} ${callTitle(c)} · ${dur || timeOfDay(c.ts)}`;
}

/** Secondary line in the log: result and duration. */
export function callDetail(c: CallRecord): string {
  const dur = callDuration(c.duration_secs);
  switch (direction(c)) {
    case "missed":
      return "Missed";
    case "declined":
      return "Declined";
    case "ringing":
      return "Ringing…";
    case "outgoing":
      if (c.result === "in_progress") return "Calling…";
      return dur ? `Outgoing · ${dur}` : "Outgoing · no answer";
    default:
      return dur ? `Incoming · ${dur}` : "Incoming";
  }
}

/** Banner label: "Voice call", "Video call", "Group voice call". */
export function ringLabel(c: CallRecord): string {
  const kind = mediaLabel(c.media);
  return c.is_group ? `Group ${kind} call` : `${kind.charAt(0).toUpperCase()}${kind.slice(1)} call`;
}

/** Newest first by time then id (stable for equal timestamps). */
export function sortCalls(calls: readonly CallRecord[]): CallRecord[] {
  return [...calls].sort((a, b) => b.ts - a.ts || (a.call_id < b.call_id ? 1 : -1));
}

/** Insert or replace a call by id, keeping newest-first order. */
export function upsertCall(calls: readonly CallRecord[], c: CallRecord): CallRecord[] {
  return sortCalls([...calls.filter((x) => !(x.call_id === c.call_id && x.chat_jid === c.chat_jid)), c]);
}
