// Group consecutive photos/videos into WhatsApp-style albums (pure, tested).

/** Longest gap between two items of one album, in seconds. */
export const ALBUM_GAP_SECS = 120;
/** Most items in one album (WhatsApp sends albums of up to 10). */
export const ALBUM_MAX = 10;
/** Tiles drawn in the grid; any further items are counted as "+N" on the last tile. */
export const ALBUM_TILES = 4;

/** The fields grouping needs; `Message` satisfies this. */
export interface AlbumLike {
  id: string;
  ts: number;
  is_from_me: boolean;
  sender_jid: string | null;
  content: string;
  deleted: boolean;
  media: { kind: string } | null;
}

export interface AlbumOptions {
  /** Maximum seconds between neighbours (default 120). */
  gap?: number;
  /** Maximum items per album (default 10). */
  max?: number;
  /** Force a new group at `m` (day change, unread divider, a call in between...). */
  breakBefore?: (m: AlbumLike, prev: AlbumLike) => boolean;
}

/** A photo or video that can be part of an album. */
export function isAlbumMedia(m: AlbumLike): boolean {
  return !m.deleted && (m.media?.kind === "image" || m.media?.kind === "video");
}

/**
 * Split an ascending message list into singles and albums. An album is two or
 * more consecutive photos/videos from the same sender, each within `gap`
 * seconds of the previous one. A caption ends the album (it belongs to the last
 * item, like WhatsApp), so a captioned item never has a successor in its group.
 * Runs longer than `max` are split. Order is preserved.
 */
export function groupAlbums<T extends AlbumLike>(msgs: readonly T[], opts: AlbumOptions = {}): (T | T[])[] {
  const gap = opts.gap ?? ALBUM_GAP_SECS;
  const max = opts.max ?? ALBUM_MAX;
  const out: (T | T[])[] = [];
  let run: T[] = [];
  const flush = () => {
    if (run.length === 1) out.push(run[0] as T);
    else if (run.length > 1) out.push(run);
    run = [];
  };
  for (const m of msgs) {
    if (!isAlbumMedia(m)) {
      flush();
      out.push(m);
      continue;
    }
    const last = run[run.length - 1];
    const joins =
      !!last &&
      last.content.trim() === "" &&
      last.is_from_me === m.is_from_me &&
      last.sender_jid === m.sender_jid &&
      m.ts - last.ts >= 0 &&
      m.ts - last.ts <= gap &&
      run.length < max &&
      !opts.breakBefore?.(m, last);
    if (!joins) flush();
    run.push(m);
  }
  flush();
  return out;
}

export type TileLayout = "pair" | "trio" | "grid";

/** Layout for an album of `n` (>= 2) items. */
export function albumLayout(n: number): TileLayout {
  return n <= 2 ? "pair" : n === 3 ? "trio" : "grid";
}

/** Items drawn as tiles and how many more are hidden behind "+N". */
export function albumTiles<T>(items: readonly T[]): { shown: T[]; extra: number } {
  const shown = items.slice(0, ALBUM_TILES);
  return { shown, extra: Math.max(0, items.length - ALBUM_TILES) };
}
