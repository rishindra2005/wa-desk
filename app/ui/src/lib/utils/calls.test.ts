import { describe, expect, it } from "vitest";
import type { CallRecord } from "../bindings/CallRecord";
import {
  callDetail, callDuration, callTitle, direction, filterQuery, isMissedForMe, isRinging, missedSince, pillText, ringLabel, sortCalls, upsertCall,
} from "./calls";
import { timeOfDay } from "./format";

const base: CallRecord = {
  call_id: "C1", chat_jid: "919845012345@s.whatsapp.net", chat_name: "Ananya Rao", caller_jid: "919845012345@s.whatsapp.net",
  caller_name: "Ananya Rao", avatar_url: "", is_from_me: false, is_group: false, media: "voice", result: "answered", ts: 1_760_000_000,
  duration_secs: 185, ended_at: 1_760_000_185, reason: null,
};
const mk = (o: Partial<CallRecord>): CallRecord => ({ ...base, ...o });

describe("call duration formatting", () => {
  it("handles seconds, minutes, hours", () => {
    expect(callDuration(45)).toBe("45 sec");
    expect(callDuration(185)).toBe("3 min");
    expect(callDuration(59.6)).toBe("1 min");
    expect(callDuration(3725)).toBe("1 h 2 min");
  });
  it("is empty for missing / zero / invalid", () => {
    expect(callDuration(null)).toBe("");
    expect(callDuration(0)).toBe("");
    expect(callDuration(-5)).toBe("");
    expect(callDuration(Number.NaN)).toBe("");
  });
});

describe("call rows", () => {
  it("classifies direction", () => {
    expect(direction(mk({}))).toBe("incoming");
    expect(direction(mk({ result: "missed" }))).toBe("missed");
    expect(direction(mk({ result: "rejected" }))).toBe("declined");
    expect(direction(mk({ result: "in_progress" }))).toBe("ringing");
    expect(direction(mk({ is_from_me: true, result: "missed" }))).toBe("outgoing");
  });
  it("titles", () => {
    expect(callTitle(mk({ result: "missed" }))).toBe("Missed voice call");
    expect(callTitle(mk({ result: "missed", media: "video" }))).toBe("Missed video call");
    expect(callTitle(mk({ media: "video" }))).toBe("Video call");
    expect(callTitle(mk({ is_group: true, media: "video", result: "ended" }))).toBe("Group video call");
    expect(callTitle(mk({ is_group: true, result: "missed" }))).toBe("Missed group voice call");
    expect(callTitle(mk({ is_from_me: true }))).toBe("Outgoing voice call");
    expect(callTitle(mk({ is_from_me: true, result: "missed" }))).toBe("Unanswered voice call");
    expect(callTitle(mk({ result: "in_progress" }))).toBe("Incoming voice call");
  });
  it("details", () => {
    expect(callDetail(mk({}))).toBe("Incoming · 3 min");
    expect(callDetail(mk({ duration_secs: null }))).toBe("Incoming");
    expect(callDetail(mk({ result: "missed" }))).toBe("Missed");
    expect(callDetail(mk({ is_from_me: true, duration_secs: 0, result: "ended" }))).toBe("Outgoing · no answer");
  });
  it("thread pill uses the duration when the call connected, else the time", () => {
    expect(pillText(mk({ media: "video" }))).toBe("📹 Video call · 3 min");
    expect(pillText(mk({ result: "missed" }))).toBe(`📞 Missed voice call · ${timeOfDay(base.ts)}`);
    expect(pillText(mk({ result: "ended", duration_secs: 0 }))).toBe(`📞 Voice call · ${timeOfDay(base.ts)}`);
  });
  it("banner label", () => {
    expect(ringLabel(mk({}))).toBe("Voice call");
    expect(ringLabel(mk({ media: "video" }))).toBe("Video call");
    expect(ringLabel(mk({ is_group: true, media: "video" }))).toBe("Group video call");
  });
});

describe("missed badge logic", () => {
  const calls = [
    mk({ call_id: "a", result: "missed", ts: 100 }),
    mk({ call_id: "b", result: "rejected", ts: 200 }),
    mk({ call_id: "c", result: "answered", ts: 300 }),
    mk({ call_id: "d", result: "missed", ts: 400, is_from_me: true }),
    mk({ call_id: "e", result: "missed", ts: 500 }),
  ];
  it("counts only missed/declined incoming calls after the last visit", () => {
    expect(missedSince(calls, 0)).toBe(3);
    expect(missedSince(calls, 100)).toBe(2);
    expect(missedSince(calls, 500)).toBe(0);
  });
  it("missed and ringing predicates", () => {
    expect(isMissedForMe(calls[3] as CallRecord)).toBe(false);
    expect(isMissedForMe(calls[1] as CallRecord)).toBe(true);
    expect(isRinging(mk({ result: "in_progress" }))).toBe(true);
    expect(isRinging(mk({ result: "in_progress", is_from_me: true }))).toBe(false);
  });
});

describe("queries and ordering", () => {
  it("builds filter queries", () => {
    expect(filterQuery("all")).toMatchObject({ missed_only: false, media: null, chat_jid: null });
    expect(filterQuery("missed")).toMatchObject({ missed_only: true, media: null });
    expect(filterQuery("video", "x@g.us")).toMatchObject({ missed_only: false, media: "video", chat_jid: "x@g.us" });
  });
  it("sorts newest first and upserts by id", () => {
    const a = mk({ call_id: "a", ts: 1 });
    const b = mk({ call_id: "b", ts: 2 });
    expect(sortCalls([a, b]).map((c) => c.call_id)).toEqual(["b", "a"]);
    const out = upsertCall([b, a], mk({ call_id: "a", ts: 1, result: "missed" }));
    expect(out).toHaveLength(2);
    expect(out.find((c) => c.call_id === "a")?.result).toBe("missed");
    expect(upsertCall(out, mk({ call_id: "z", ts: 9 }))[0]?.call_id).toBe("z");
  });
});
