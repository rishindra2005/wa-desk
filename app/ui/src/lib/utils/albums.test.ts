import { describe, expect, it } from "vitest";
import { ALBUM_MAX, albumLayout, albumTiles, groupAlbums, type AlbumLike } from "./albums";
import { toggleGroup } from "./selection";

let n = 0;
function img(ts: number, over: Partial<AlbumLike> = {}, kind = "image"): AlbumLike {
  return { id: `m${n++}`, ts, is_from_me: false, sender_jid: "a@s", content: "", deleted: false, media: { kind }, ...over };
}
const text = (ts: number, over: Partial<AlbumLike> = {}): AlbumLike => img(ts, { media: null, content: "hi", ...over });
const shape = (g: (AlbumLike | AlbumLike[])[]) => g.map((x) => (Array.isArray(x) ? x.length : 1));

describe("groupAlbums", () => {
  it("leaves singles and text alone", () => {
    const a = img(0);
    const t = text(5);
    expect(groupAlbums([a, t])).toEqual([a, t]);
    expect(groupAlbums([])).toEqual([]);
  });

  it("groups consecutive images and videos (mixed types)", () => {
    expect(shape(groupAlbums([img(0), img(10, {}, "video"), img(20)]))).toEqual([3]);
  });

  it("splits on sender change, including me vs them", () => {
    expect(shape(groupAlbums([img(0), img(5, { sender_jid: "b@s" }), img(9, { sender_jid: "b@s" })]))).toEqual([1, 2]);
    expect(shape(groupAlbums([img(0), img(5, { is_from_me: true, sender_jid: null })]))).toEqual([1, 1]);
  });

  it("splits when the gap exceeds the window", () => {
    expect(shape(groupAlbums([img(0), img(120), img(241)]))).toEqual([2, 1]);
    expect(shape(groupAlbums([img(0), img(100), img(200), img(300)]))).toEqual([4]);
  });

  it("lets a caption end the album, never continue it", () => {
    const g = groupAlbums([img(0), img(1), img(2, { content: "Trip!" }), img(3), img(4)]);
    expect(shape(g)).toEqual([3, 2]);
    expect((g[0] as AlbumLike[])[2]?.content).toBe("Trip!");
    expect(shape(groupAlbums([img(0, { content: "cap" }), img(1)]))).toEqual([1, 1]);
  });

  it("breaks on text, audio, stickers, documents and deleted messages", () => {
    expect(shape(groupAlbums([img(0), text(1), img(2)]))).toEqual([1, 1, 1]);
    expect(shape(groupAlbums([img(0), img(1, {}, "audio"), img(2)]))).toEqual([1, 1, 1]);
    expect(shape(groupAlbums([img(0), img(1, {}, "sticker"), img(2)]))).toEqual([1, 1, 1]);
    expect(shape(groupAlbums([img(0), img(1, {}, "document"), img(2)]))).toEqual([1, 1, 1]);
    expect(shape(groupAlbums([img(0), img(1, { deleted: true }), img(2), img(3)]))).toEqual([1, 1, 2]);
  });

  it("groups more than four items and splits at the maximum", () => {
    expect(shape(groupAlbums(Array.from({ length: 6 }, (_, i) => img(i))))).toEqual([6]);
    expect(shape(groupAlbums(Array.from({ length: ALBUM_MAX + 3 }, (_, i) => img(i))))).toEqual([ALBUM_MAX, 3]);
    expect(shape(groupAlbums(Array.from({ length: ALBUM_MAX + 1 }, (_, i) => img(i))))).toEqual([ALBUM_MAX, 1]);
  });

  it("honours breakBefore (day change, unread marker)", () => {
    const items = [img(0), img(1), img(2), img(3)];
    expect(shape(groupAlbums(items, { breakBefore: (m) => m === items[2] }))).toEqual([2, 2]);
  });

  it("preserves order and identity", () => {
    const items = [img(0), img(1), text(2), img(3), img(4)];
    expect(groupAlbums(items).flat()).toEqual(items);
  });
});

describe("album layout", () => {
  it("picks a layout by size", () => {
    expect([2, 3, 4, 5, 10].map(albumLayout)).toEqual(["pair", "trio", "grid", "grid", "grid"]);
  });
  it("shows up to four tiles and counts the rest", () => {
    expect(albumTiles([1, 2, 3])).toEqual({ shown: [1, 2, 3], extra: 0 });
    expect(albumTiles([1, 2, 3, 4])).toEqual({ shown: [1, 2, 3, 4], extra: 0 });
    expect(albumTiles([1, 2, 3, 4, 5, 6, 7])).toEqual({ shown: [1, 2, 3, 4], extra: 3 });
  });
});

describe("toggleGroup", () => {
  it("selects all, then deselects all", () => {
    const a = toggleGroup(new Set(["x"]), ["1", "2"]);
    expect([...a].sort()).toEqual(["1", "2", "x"]);
    expect([...toggleGroup(a, ["1", "2"])]).toEqual(["x"]);
  });
  it("completes a partial selection", () => {
    expect([...toggleGroup(new Set(["1"]), ["1", "2"])].sort()).toEqual(["1", "2"]);
  });
});
