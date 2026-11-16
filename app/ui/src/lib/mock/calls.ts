// Deterministic call log for the mock backend (kept separate so the chat seed sequence is unchanged).

import type { CallMedia } from "../bindings/CallMedia";
import type { CallRecord } from "../bindings/CallRecord";
import type { CallResult } from "../bindings/CallResult";
import { mulberry32 } from "../utils/misc";
import type { MockChat } from "./seed";

export const MY_JID = "919000011111@s.whatsapp.net";

function hex(n: number): string {
  return n.toString(16).toUpperCase().padStart(8, "0");
}

interface Spec {
  chat: MockChat;
  ts: number;
  media: CallMedia;
  result: CallResult;
  fromMe: boolean;
  duration: number | null;
  caller?: { jid: string; name: string };
}

function build(spec: Spec, id: number): CallRecord {
  const c = spec.chat;
  const caller = spec.fromMe ? { jid: MY_JID, name: "Me" } : (spec.caller ?? { jid: c.jid, name: c.name });
  const finished = spec.result !== "in_progress";
  return {
    call_id: `3ACALL${hex(id)}`,
    chat_jid: c.jid,
    chat_name: c.name,
    caller_jid: caller.jid,
    caller_name: caller.name,
    avatar_url: "",
    is_from_me: spec.fromMe,
    is_group: c.is_group,
    media: spec.media,
    result: spec.result,
    ts: spec.ts,
    duration_secs: finished ? (spec.duration ?? 0) : null,
    ended_at: finished ? spec.ts + (spec.duration ?? 0) + (spec.result === "missed" ? 30 : 2) : null,
    reason: spec.result === "missed" ? "timeout" : spec.result === "rejected" ? "declined" : null,
  };
}

/** ~40 calls: voice/video, incoming/outgoing, missed/declined/answered, plus group calls. */
export function generateCalls(now: number, chats: MockChat[]): CallRecord[] {
  const rnd = mulberry32(2024);
  const pick = <T>(xs: T[]): T => xs[Math.floor(rnd() * xs.length)] as T;
  const directs = chats.filter((c) => c.chat_type === "direct" && c.messages.length > 5).slice(0, 14);
  const groups = chats.filter((c) => c.chat_type === "group" && c.messages.length > 5).slice(0, 5);
  const specs: Spec[] = [];

  // Fixed calls in the newest part of the first chat, so a thread always shows call pills.
  const first = directs[0];
  if (first) {
    const last = (first.messages[first.messages.length - 1]?.ts ?? now) - 5;
    specs.push({ chat: first, ts: last - 60, media: "voice", result: "missed", fromMe: false, duration: 0 });
    specs.push({ chat: first, ts: last - 20, media: "video", result: "ended", fromMe: false, duration: 185 });
  }

  for (let i = 0; i < 38; i++) {
    const isGroup = i % 8 === 5 && groups.length > 0;
    const chat = isGroup ? pick(groups) : pick(directs.length ? directs : chats);
    const msg = pick(chat.messages);
    const ts = Math.min(now - 90, msg.ts + 15 + Math.floor(rnd() * 120));
    const media: CallMedia = rnd() < (isGroup ? 0.5 : 0.3) ? "video" : "voice";
    const roll = rnd();
    let result: CallResult;
    let fromMe = false;
    let duration: number | null = null;
    if (roll < 0.34) {
      result = rnd() < 0.85 ? "ended" : "answered";
      duration = isGroup ? 240 + Math.floor(rnd() * 3200) : 12 + Math.floor(rnd() * 1500);
    } else if (roll < 0.6) {
      result = "missed";
    } else if (roll < 0.68) {
      result = "rejected";
    } else if (roll < 0.93) {
      fromMe = !isGroup;
      result = "ended";
      duration = 20 + Math.floor(rnd() * 900);
    } else {
      fromMe = true;
      result = "missed";
    }
    const part = isGroup ? chat.participants.filter((p) => !p.is_me) : [];
    const who = part.length ? pick(part) : undefined;
    specs.push({
      chat,
      ts,
      media,
      result,
      fromMe,
      duration,
      ...(who ? { caller: { jid: who.jid, name: who.name || who.phone || "Member" } } : {}),
    });
  }
  return specs.sort((a, b) => a.ts - b.ts).map((s, i) => build(s, i + 1));
}

/** A new incoming call that is ringing right now. */
export function ringingCall(chat: MockChat, id: number, ts: number, media: CallMedia): CallRecord {
  const who = chat.is_group ? chat.participants.find((p) => !p.is_me) : undefined;
  return build(
    {
      chat,
      ts,
      media,
      result: "in_progress",
      fromMe: false,
      duration: null,
      ...(who ? { caller: { jid: who.jid, name: who.name || who.phone || "Member" } } : {}),
    },
    id,
  );
}
