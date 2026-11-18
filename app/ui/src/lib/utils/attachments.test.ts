import { describe, expect, it } from "vitest";
import { addAttachments, buildSendReqs, MAX_ATTACHMENTS, parseFileUris, type Attachment } from "./attachments";

const att = (path: string): Attachment => ({ id: path, path, name: path.split("/").pop() ?? path });

describe("addAttachments", () => {
  it("appends in order and skips duplicates", () => {
    const r = addAttachments([att("/a")], [att("/b"), att("/a"), att("/c")]);
    expect(r.list.map((a) => a.path)).toEqual(["/a", "/b", "/c"]);
    expect(r.skippedDuplicates).toBe(1);
  });
  it("respects the cap", () => {
    const many = Array.from({ length: MAX_ATTACHMENTS + 3 }, (_, i) => att(`/f${i}`));
    const r = addAttachments([], many);
    expect(r.list).toHaveLength(MAX_ATTACHMENTS);
    expect(r.skippedOverLimit).toBe(3);
  });
});

describe("buildSendReqs", () => {
  it("sends text alone when there are no files", () => {
    expect(buildSendReqs("r", "hi", [], "q")).toEqual([{ recipient: "r", text: "hi", attachment: null, reply_to: "q", mentions: [] }]);
  });
  it("puts the caption and the reply on the first file only", () => {
    const reqs = buildSendReqs("r", "nice trip", [{ path: "/a.jpg" }, { path: "/b.jpg" }, { path: "/c.pdf" }], "q");
    expect(reqs.map((r) => r.attachment)).toEqual(["/a.jpg", "/b.jpg", "/c.pdf"]);
    expect(reqs.map((r) => r.text)).toEqual(["nice trip", null, null]);
    expect(reqs.map((r) => r.reply_to)).toEqual(["q", null, null]);
  });
  it("treats blank text as no caption", () => {
    expect(buildSendReqs("r", "   ", [{ path: "/a" }], null)[0]?.text).toBeNull();
  });
});

describe("parseFileUris", () => {
  it("decodes file URIs from a uri-list", () => {
    expect(parseFileUris("# copied\r\nfile:///home/u/My%20Pic.png\r\nfile://localhost/tmp/a.txt\r\n")).toEqual(["/home/u/My Pic.png", "/tmp/a.txt"]);
  });
  it("returns nothing for ordinary text or web links", () => {
    expect(parseFileUris("hello world")).toEqual([]);
    expect(parseFileUris("https://example.com/x.png")).toEqual([]);
    expect(parseFileUris("file:///a\nplain line")).toEqual([]);
    expect(parseFileUris("")).toEqual([]);
  });
  it("rejects folders, remote hosts and bad escapes", () => {
    expect(parseFileUris("file:///home/u/dir/")).toEqual([]);
    expect(parseFileUris("file://server/share/x")).toEqual([]);
    expect(parseFileUris("file:///bad%zz")).toEqual([]);
    expect(parseFileUris("file:///a%00b")).toEqual([]);
  });
});
