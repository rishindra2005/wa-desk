// Small generic helpers: debounce, colour hashing, PRNG.

export interface Debounced<A extends unknown[]> {
  (...args: A): void;
  cancel(): void;
  flush(): void;
  readonly pending: boolean;
}

/** Trailing-edge debounce with cancel/flush. */
export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number): Debounced<A> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastArgs: A | null = null;
  const run = () => {
    timer = null;
    const args = lastArgs;
    lastArgs = null;
    if (args) fn(...args);
  };
  const d = ((...args: A) => {
    lastArgs = args;
    if (timer) clearTimeout(timer);
    timer = setTimeout(run, ms);
  }) as Debounced<A>;
  d.cancel = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    lastArgs = null;
  };
  d.flush = () => {
    if (timer) {
      clearTimeout(timer);
      run();
    }
  };
  Object.defineProperty(d, "pending", { get: () => timer !== null });
  return d;
}

/** 32-bit FNV-1a hash of a string. */
export function hashString(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Avatar / sender colour palette (hue degrees), tuned for both themes. */
export const PALETTE_HUES = [4, 24, 38, 88, 142, 168, 192, 212, 232, 262, 290, 322] as const;

/** Stable palette index for a key (JID or name). */
export function colorIndex(key: string): number {
  return hashString(key) % PALETTE_HUES.length;
}

/** Stable hue (0..360) for a key. */
export function colorHue(key: string): number {
  return PALETTE_HUES[colorIndex(key)] ?? 142;
}

/** Deterministic PRNG (mulberry32). Returns floats in [0, 1). */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Clamp a number. */
export function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/** Resolve after `ms`. */
export function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

/** Copy text to the clipboard; resolves false when unavailable. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
