import { describe, it, expect } from "vitest";
import { isMuted, muteUntil, sortChats, MUTE_FOREVER } from "./chatstate";

describe("isMuted", () => {
  it("handles null, forever, future and expired", () => {
    expect(isMuted(null, 1000)).toBe(false);
    expect(isMuted(MUTE_FOREVER, 1000)).toBe(true);
    expect(isMuted(2000, 1000)).toBe(true);
    expect(isMuted(1000, 1000)).toBe(false);
    expect(isMuted(500, 1000)).toBe(false);
  });
});

describe("muteUntil", () => {
  it("computes durations", () => {
    expect(muteUntil("8h", 100)).toBe(100 + 28800);
    expect(muteUntil("1w", 100)).toBe(100 + 604800);
    expect(muteUntil("always", 100)).toBe(-1);
  });
});

describe("sortChats", () => {
  it("puts pinned first then newest, stably", () => {
    const l = [
      { id: "a", pinned: false, last_message_ts: 30 },
      { id: "b", pinned: true, last_message_ts: 10 },
      { id: "c", pinned: false, last_message_ts: 40 },
      { id: "d", pinned: true, last_message_ts: 20 },
      { id: "e", pinned: false, last_message_ts: null },
    ];
    expect(sortChats(l).map((x) => x.id)).toEqual(["d", "b", "c", "a", "e"]);
  });
});
