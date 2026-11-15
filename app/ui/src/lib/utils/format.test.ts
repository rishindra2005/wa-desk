import { describe, expect, it } from "vitest";
import {
  dayLabel, formatClock, formatCount, formatDuration, formatPhone, formatSize, fromDateInput, isHHMM, pct,
  relativeTime, shortDate, timeAgo, timeOfDay, toDateInput, displayName,
} from "./format";

const at = (y: number, mo: number, d: number, h = 12, mi = 0) => new Date(y, mo - 1, d, h, mi).getTime();
const sec = (ms: number) => Math.floor(ms / 1000);

describe("relativeTime", () => {
  const now = at(2025, 3, 12, 15, 0); // Wednesday
  it("returns empty for null", () => expect(relativeTime(null, now)).toBe(""));
  it("says now under a minute", () => expect(relativeTime(sec(now) - 20, now)).toBe("now"));
  it("uses minutes under an hour", () => expect(relativeTime(sec(now) - 5 * 60, now)).toBe("5m"));
  it("uses hours for recent same-day", () => expect(relativeTime(sec(now) - 3 * 3600, now)).toBe("3h"));
  it("uses clock time earlier today", () => expect(relativeTime(sec(at(2025, 3, 12, 1, 5)), now)).toBe("01:05"));
  it("says Yesterday", () => expect(relativeTime(sec(at(2025, 3, 11, 23, 0)), now)).toBe("Yesterday"));
  it("uses weekday within a week", () => expect(relativeTime(sec(at(2025, 3, 8, 10)), now)).toBe("Sat"));
  it("uses short date this year", () => expect(relativeTime(sec(at(2025, 1, 2)), now)).toBe("2 Jan"));
  it("includes year for older dates", () => expect(relativeTime(sec(at(2023, 11, 30)), now)).toBe("30 Nov 2023"));
  it("shows clock time for future timestamps", () => expect(relativeTime(sec(now) + 120, now)).toBe(timeOfDay(sec(now) + 120)));
});

describe("dates", () => {
  const now = at(2025, 3, 12, 15);
  it("labels days", () => {
    expect(dayLabel(sec(at(2025, 3, 12, 9)), now)).toBe("Today");
    expect(dayLabel(sec(at(2025, 3, 11, 9)), now)).toBe("Yesterday");
    expect(dayLabel(sec(at(2025, 3, 9, 9)), now)).toBe("Sunday");
    expect(dayLabel(sec(at(2024, 2, 29, 9)), now)).toBe("Thursday, 29 February 2024");
  });
  it("formats short dates", () => expect(shortDate(sec(at(2025, 7, 4)), now)).toBe("4 Jul"));
  it("pads time of day", () => expect(timeOfDay(sec(at(2025, 1, 1, 7, 3)))).toBe("07:03"));
  it("round-trips date inputs", () => {
    const ts = fromDateInput("2024-05-06");
    expect(ts).not.toBeNull();
    expect(toDateInput(ts)).toBe("2024-05-06");
    expect(fromDateInput("2024-05-06", true)! - ts!).toBe(86_399);
    expect(fromDateInput("nope")).toBeNull();
  });
  it("time ago", () => {
    const n = at(2025, 1, 1, 12);
    expect(timeAgo(null, n)).toBe("never");
    expect(timeAgo(sec(n) - 10, n)).toBe("just now");
    expect(timeAgo(sec(n) - 3600 * 5, n)).toBe("5 hours ago");
    expect(timeAgo(sec(n) - 86_400, n)).toBe("1 day ago");
  });
});

describe("formatSize", () => {
  it.each([
    [0, "0 B"], [999, "999 B"], [1024, "1 KB"], [1536, "1.5 KB"], [10 * 1024 * 1024, "10 MB"],
    [1.07 * 1024 ** 3, "1.07 GB"], [123.4 * 1024 * 1024, "123 MB"],
  ])("%d → %s", (n, s) => expect(formatSize(n)).toBe(s));
  it("handles null/negative", () => {
    expect(formatSize(null)).toBe("—");
    expect(formatSize(-1)).toBe("—");
  });
});

describe("durations and counts", () => {
  it("formats clock", () => {
    expect(formatClock(65)).toBe("1:05");
    expect(formatClock(3723)).toBe("1:02:03");
  });
  it("formats durations", () => {
    expect(formatDuration(0.85)).toBe("850 ms");
    expect(formatDuration(12)).toBe("12 s");
    expect(formatDuration(184)).toBe("3 min 4 s");
    expect(formatDuration(7500)).toBe("2 h 5 min");
    expect(formatDuration(null)).toBe("—");
  });
  it("formats counts", () => expect(formatCount(1234567)).toBe("1,234,567"));
  it("pct guards zero", () => {
    expect(pct(5, 0)).toBe(0);
    expect(pct(5, 10)).toBe(50);
    expect(pct(20, 10)).toBe(100);
  });
  it("validates HH:MM", () => {
    expect(isHHMM("23:59")).toBe(true);
    expect(isHHMM("24:00")).toBe(false);
    expect(isHHMM("7:00")).toBe(false);
  });
});

describe("formatPhone", () => {
  it("formats Indian numbers", () => expect(formatPhone("919876543210@s.whatsapp.net")).toBe("+91 98765 43210"));
  it("formats US numbers", () => expect(formatPhone("14155550123")).toBe("+1 415 555 0123"));
  it("marks LIDs", () => expect(formatPhone("204958372615402@lid")).toBe("~204958372615402"));
  it("leaves groups alone", () => expect(formatPhone("120363@g.us")).toBe("120363@g.us"));
  it("leaves non-numbers alone", () => expect(formatPhone("hello")).toBe("hello"));
  it("displayName prefers names", () => {
    expect(displayName("Ananya", "91@s.whatsapp.net")).toBe("Ananya");
    expect(displayName("204958372615402", "204958372615402@lid")).toBe("~204958372615402");
  });
});
