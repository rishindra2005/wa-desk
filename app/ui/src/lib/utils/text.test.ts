import { describe, expect, it } from "vitest";
import {
  byteOffsetMapper, byteRangeToIndices, docBucket, docType, extOf, highlightSegments, indexToByteOffset, initials,
  linkify, safeSlice, snippetSegments, baseName,
} from "./text";

/** Byte range of `needle` in `hay` (as Rust would report it). */
function bytesOf(hay: string, needle: string): [number, number] {
  const i = hay.indexOf(needle);
  return [indexToByteOffset(hay, i), indexToByteOffset(hay, i + needle.length)];
}
const hits = (text: string, ranges: [number, number][]) => highlightSegments(text, ranges).filter((s) => s.hit).map((s) => s.text);

describe("byte offsets → string indices", () => {
  it("is identity for ASCII", () => {
    const map = byteOffsetMapper("hello world");
    expect(map(0)).toBe(0);
    expect(map(5)).toBe(5);
    expect(map(11)).toBe(11);
  });

  it("handles Devanagari (3 bytes per code unit)", () => {
    const t = "कल मिलते हैं";
    expect(byteRangeToIndices(t, bytesOf(t, "मिलते"))).toEqual([3, 8]);
    expect(hits(t, [bytesOf(t, "मिलते")])).toEqual(["मिलते"]);
  });

  it("handles astral emoji (4 bytes, 2 code units)", () => {
    const t = "Sunset 🌅 today 😂 lol";
    expect(hits(t, [bytesOf(t, "today")])).toEqual(["today"]);
    expect(hits(t, [bytesOf(t, "😂")])).toEqual(["😂"]);
  });

  it("handles ZWJ sequences", () => {
    const t = "👨‍👩‍👧 family photo";
    expect(hits(t, [bytesOf(t, "family")])).toEqual(["family"]);
    expect(hits(t, [bytesOf(t, "👨‍👩‍👧")])).toEqual(["👨‍👩‍👧"]);
  });

  it("snaps offsets inside a code point outward", () => {
    const t = "a😂b";
    // 😂 occupies bytes 1..5; a range starting mid-emoji widens to include it.
    expect(byteRangeToIndices(t, [2, 4])).toEqual([1, 3]);
  });

  it("clamps out-of-range offsets", () => {
    const t = "abc";
    expect(byteRangeToIndices(t, [-5, 99])).toEqual([0, 3]);
  });

  it("indexToByteOffset is the inverse", () => {
    const t = "नमस्ते 😊 hi";
    const map = byteOffsetMapper(t);
    for (let i = 0; i <= t.length; i++) {
      const isLow = i > 0 && i < t.length && t.charCodeAt(i) >= 0xdc00 && t.charCodeAt(i) <= 0xdfff;
      if (!isLow) expect(map(indexToByteOffset(t, i))).toBe(i);
    }
  });
});

describe("highlightSegments", () => {
  it("merges overlapping and unsorted ranges", () => {
    const segs = highlightSegments("abcdefgh", [[4, 6], [1, 3], [2, 5]]);
    expect(segs).toEqual([
      { text: "a", hit: false },
      { text: "bcdef", hit: true },
      { text: "gh", hit: false },
    ]);
  });
  it("ignores empty ranges and empty text", () => {
    expect(highlightSegments("abc", [[1, 1]])).toEqual([{ text: "abc", hit: false }]);
    expect(highlightSegments("", [[0, 3]])).toEqual([]);
  });
  it("preserves the full text", () => {
    const t = "राहुल 👍 said hi";
    const segs = highlightSegments(t, [bytesOf(t, "said"), bytesOf(t, "👍")]);
    expect(segs.map((s) => s.text).join("")).toBe(t);
  });
});

describe("snippetSegments", () => {
  it("keeps short text unchanged", () => {
    const segs = highlightSegments("short", [[0, 2]]);
    expect(snippetSegments(segs, 50)).toEqual(segs);
  });
  it("centers long text on the first hit with ellipses", () => {
    const t = `${"x".repeat(300)}NEEDLE${"y".repeat(300)}`;
    const out = snippetSegments(highlightSegments(t, [bytesOf(t, "NEEDLE")]), 60);
    const joined = out.map((s) => s.text).join("");
    expect(joined.startsWith("…")).toBe(true);
    expect(joined.endsWith("…")).toBe(true);
    expect(out.find((s) => s.hit)?.text).toBe("NEEDLE");
  });
  it("safeSlice never splits surrogate pairs", () => {
    const s = "a😂b";
    expect(safeSlice(s, 2, 4)).toBe("😂b");
    expect(safeSlice(s, 0, 2)).toBe("a😂");
  });
});

describe("linkify", () => {
  it("finds http and www links and trims trailing punctuation", () => {
    const parts = linkify("See https://example.com/a?b=1. Or www.test.org/menu, thanks");
    const links = parts.filter((p) => p.href);
    expect(links.map((l) => l.text)).toEqual(["https://example.com/a?b=1", "www.test.org/menu"]);
    expect(links[1]?.href).toBe("https://www.test.org/menu");
    expect(parts.map((p) => p.text).join("")).toBe("See https://example.com/a?b=1. Or www.test.org/menu, thanks");
  });
  it("returns plain text when there are no links", () => expect(linkify("no links")).toEqual([{ text: "no links" }]));
});

describe("initials", () => {
  it.each([
    ["Ananya Rao", "AR"],
    ["Mom ❤️", "M"],
    ["राहुल शर्मा", "रश"],
    ["🎉", "🎉"],
    ["+91 98765 43210", "#"],
    ["", "?"],
    ["Lena Müller", "LM"],
  ])("%s → %s", (name, want) => expect(initials(name)).toBe(want));
});

describe("file helpers", () => {
  it("extracts extensions and base names", () => {
    expect(extOf("/a/b/Report.Final.PDF")).toBe("pdf");
    expect(extOf("noext")).toBe("");
    expect(baseName("/home/u/x.docx")).toBe("x.docx");
  });
  it("classifies documents", () => {
    expect(docType("a.pdf")).toBe("pdf");
    expect(docType("a.xlsx")).toBe("sheet");
    expect(docType("x", "application/pdf")).toBe("pdf");
    expect(docBucket("deck.pptx")).toBe("office");
    expect(docBucket("a.zip")).toBe("other");
  });
});
