import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clamp, colorHue, colorIndex, debounce, hashString, mulberry32, PALETTE_HUES } from "./misc";

describe("debounce", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("calls once with the latest args after the delay", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d(1);
    d(2);
    d(3);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(99);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith(3);
  });

  it("cancel drops the pending call", () => {
    const fn = vi.fn();
    const d = debounce(fn, 50);
    d();
    expect(d.pending).toBe(true);
    d.cancel();
    expect(d.pending).toBe(false);
    vi.advanceTimersByTime(100);
    expect(fn).not.toHaveBeenCalled();
  });

  it("flush runs immediately", () => {
    const fn = vi.fn();
    const d = debounce(fn, 50);
    d("x");
    d.flush();
    expect(fn).toHaveBeenCalledWith("x");
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("restarts the timer on each call", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d();
    vi.advanceTimersByTime(80);
    d();
    vi.advanceTimersByTime(80);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(20);
    expect(fn).toHaveBeenCalledTimes(1);
  });
});

describe("colour hash", () => {
  it("is stable for the same key", () => {
    expect(colorIndex("919845012345@s.whatsapp.net")).toBe(colorIndex("919845012345@s.whatsapp.net"));
    expect(hashString("abc")).toBe(hashString("abc"));
  });
  it("stays within the palette", () => {
    for (const k of ["a", "b", "राहुल", "😂", "", "x".repeat(500)]) {
      expect(colorIndex(k)).toBeGreaterThanOrEqual(0);
      expect(colorIndex(k)).toBeLessThan(PALETTE_HUES.length);
      expect(PALETTE_HUES).toContain(colorHue(k));
    }
  });
  it("spreads different keys", () => {
    const set = new Set(Array.from({ length: 60 }, (_, i) => colorIndex(`contact-${i}`)));
    expect(set.size).toBeGreaterThan(6);
  });
});

describe("mulberry32 / clamp", () => {
  it("is deterministic per seed", () => {
    const a = mulberry32(42);
    const b = mulberry32(42);
    const xs = Array.from({ length: 5 }, () => a());
    expect(xs).toEqual(Array.from({ length: 5 }, () => b()));
    expect(xs.every((x) => x >= 0 && x < 1)).toBe(true);
  });
  it("clamps", () => {
    expect(clamp(5, 0, 3)).toBe(3);
    expect(clamp(-1, 0, 3)).toBe(0);
  });
});
