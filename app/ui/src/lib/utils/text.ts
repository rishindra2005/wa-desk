// Text helpers: UTF-8 byte ranges → JS string indices, highlight segmentation,
// link detection, initials.

/** UTF-8 length of a single code point. */
function utf8Len(cp: number): number {
  if (cp < 0x80) return 1;
  if (cp < 0x800) return 2;
  if (cp < 0x10000) return 3;
  return 4;
}

/**
 * Build a converter from UTF-8 byte offsets (as produced by Rust `str` slicing)
 * into UTF-16 code-unit indices (JS string indices).
 *
 * Offsets that fall inside a multi-byte code point snap to its start (`"floor"`)
 * or past its end (`"ceil"`). Offsets beyond the end clamp to `text.length`.
 */
export function byteOffsetMapper(text: string): (byte: number, mode?: "floor" | "ceil") => number {
  // byteStarts[i] = byte offset of the i-th code point; unitStarts[i] = its JS index.
  const byteStarts: number[] = [];
  const unitStarts: number[] = [];
  let byte = 0;
  let unit = 0;
  for (const ch of text) {
    const cp = ch.codePointAt(0) ?? 0;
    byteStarts.push(byte);
    unitStarts.push(unit);
    byte += utf8Len(cp);
    unit += ch.length;
  }
  const totalBytes = byte;
  return (b: number, mode: "floor" | "ceil" = "floor") => {
    if (!Number.isFinite(b) || b <= 0) return 0;
    if (b >= totalBytes) return text.length;
    // Binary search for the last code point starting at or before b.
    let lo = 0;
    let hi = byteStarts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if ((byteStarts[mid] ?? 0) <= b) lo = mid;
      else hi = mid - 1;
    }
    const start = byteStarts[lo] ?? 0;
    if (start === b || mode === "floor") return unitStarts[lo] ?? 0;
    return lo + 1 < unitStarts.length ? (unitStarts[lo + 1] ?? text.length) : text.length;
  };
}

/** Convert one byte range to a [start, end) JS index range (snapped outward). */
export function byteRangeToIndices(text: string, range: [number, number]): [number, number] {
  const map = byteOffsetMapper(text);
  return [map(range[0], "floor"), map(range[1], "ceil")];
}

/** JS index → UTF-8 byte offset (inverse of the mapper; used by the mock backend). */
export function indexToByteOffset(text: string, index: number): number {
  return new TextEncoder().encode(text.slice(0, index)).length;
}

export interface Segment {
  text: string;
  hit: boolean;
}

/**
 * Split `text` into plain / highlighted segments from UTF-8 byte ranges.
 * Ranges may be unsorted, overlapping or out of bounds; they are normalised.
 */
export function highlightSegments(text: string, byteRanges: ReadonlyArray<readonly [number, number]>): Segment[] {
  if (!text) return [];
  const map = byteOffsetMapper(text);
  const ranges = byteRanges
    .map(([s, e]) => [map(Math.min(s, e), "floor"), map(Math.max(s, e), "ceil")] as [number, number])
    .filter(([s, e]) => e > s)
    .sort((a, b) => a[0] - b[0]);
  const merged: [number, number][] = [];
  for (const r of ranges) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else merged.push([r[0], r[1]]);
  }
  const out: Segment[] = [];
  let pos = 0;
  for (const [s, e] of merged) {
    if (s > pos) out.push({ text: text.slice(pos, s), hit: false });
    out.push({ text: text.slice(s, e), hit: true });
    pos = e;
  }
  if (pos < text.length) out.push({ text: text.slice(pos), hit: false });
  return out;
}

/**
 * Trim long highlighted text to a snippet around the first hit, keeping
 * segment boundaries. Never cuts surrogate pairs.
 */
export function snippetSegments(segments: Segment[], maxLen = 180): Segment[] {
  const total = segments.reduce((n, s) => n + s.text.length, 0);
  if (total <= maxLen) return segments;
  let firstHit = 0;
  for (const s of segments) {
    if (s.hit) break;
    firstHit += s.text.length;
  }
  let start = Math.max(0, firstHit - Math.floor(maxLen / 3));
  const end = Math.min(total, start + maxLen);
  start = Math.max(0, end - maxLen);
  const out: Segment[] = [];
  let pos = 0;
  for (const s of segments) {
    const a = Math.max(start, pos);
    const b = Math.min(end, pos + s.text.length);
    if (b > a) out.push({ text: safeSlice(s.text, a - pos, b - pos), hit: s.hit });
    pos += s.text.length;
  }
  if (start > 0 && out[0]) out[0] = { ...out[0], text: `…${out[0].text}` };
  const last = out[out.length - 1];
  if (end < total && last) out[out.length - 1] = { ...last, text: `${last.text}…` };
  return out;
}

/** Slice without splitting a surrogate pair at either edge. */
export function safeSlice(s: string, a: number, b: number): string {
  const isLow = (i: number) => {
    const c = s.charCodeAt(i);
    return c >= 0xdc00 && c <= 0xdfff;
  };
  let start = a;
  let end = b;
  if (start > 0 && start < s.length && isLow(start)) start--;
  if (end > 0 && end < s.length && isLow(end)) end++;
  return s.slice(start, end);
}

export interface LinkPart {
  text: string;
  href?: string;
}

const URL_RE = /\b((?:https?:\/\/|www\.)[^\s<>"']+[^\s<>"'.,;:!?)\]}])/gi;

/** Split text into plain text and link parts. `www.` links get an https:// href. */
export function linkify(text: string): LinkPart[] {
  const out: LinkPart[] = [];
  let last = 0;
  for (const m of text.matchAll(URL_RE)) {
    const idx = m.index ?? 0;
    const raw = m[1] ?? "";
    if (idx > last) out.push({ text: text.slice(last, idx) });
    out.push({ text: raw, href: raw.toLowerCase().startsWith("www.") ? `https://${raw}` : raw });
    last = idx + raw.length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

/** Up to two grapheme-ish initials, handling emoji/Devanagari names gracefully. */
export function initials(name: string): string {
  const clean = name.replace(/[^\p{L}\p{N}\p{Extended_Pictographic}\s+~]/gu, " ").trim();
  if (!clean) return "?";
  if (/^[+~]?\d/.test(clean)) return "#";
  const words = clean.split(/\s+/).filter(Boolean);
  const first = (w: string | undefined) => (w ? (Array.from(w)[0] ?? "") : "");
  const a = first(words[0]);
  const b = words.length > 1 ? first(words[words.length - 1]) : "";
  // Devanagari combining marks are not letters on their own; keep the base glyph only.
  return (a + b).toUpperCase();
}

/** Escape a string for use inside a RegExp. */
export function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** File extension (lowercase, without dot) of a file name or path. */
export function extOf(name: string | null | undefined): string {
  if (!name) return "";
  const base = name.split(/[\\/]/).pop() ?? "";
  const i = base.lastIndexOf(".");
  return i > 0 ? base.slice(i + 1).toLowerCase() : "";
}

/** Base name of a path. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

export type DocType = "pdf" | "word" | "sheet" | "slides" | "archive" | "text" | "other";

/** Classify a document by extension / MIME. */
export function docType(filename: string | null | undefined, mime?: string | null): DocType {
  const ext = extOf(filename);
  if (ext === "pdf" || mime === "application/pdf") return "pdf";
  if (["doc", "docx", "odt", "rtf"].includes(ext)) return "word";
  if (["xls", "xlsx", "ods", "csv"].includes(ext)) return "sheet";
  if (["ppt", "pptx", "odp"].includes(ext)) return "slides";
  if (["zip", "rar", "7z", "tar", "gz"].includes(ext)) return "archive";
  if (["txt", "md", "json", "log"].includes(ext)) return "text";
  return "other";
}

/** pdf / office / other bucket used by the Docs filter. */
export function docBucket(filename: string | null | undefined, mime?: string | null): "pdf" | "office" | "other" {
  const t = docType(filename, mime);
  if (t === "pdf") return "pdf";
  if (t === "word" || t === "sheet" || t === "slides") return "office";
  return "other";
}
