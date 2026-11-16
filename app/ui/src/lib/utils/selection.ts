// Message selection helpers (pure, tested).

/** Ids between `a` and `b` inclusive, in list order. Falls back to just `b` when `a` is unknown. */
export function rangeIds(ordered: readonly string[], a: string | null, b: string): string[] {
  const j = ordered.indexOf(b);
  if (j < 0) return [];
  const i = a === null ? -1 : ordered.indexOf(a);
  if (i < 0) return [b];
  const [lo, hi] = i <= j ? [i, j] : [j, i];
  return ordered.slice(lo, hi + 1);
}

/**
 * Apply a click to the selection. Plain/ctrl click toggles `id`; shift-click adds the
 * range from `anchor`. Returns the new set and the new anchor.
 */
export function applyClick(
  current: ReadonlySet<string>,
  ordered: readonly string[],
  anchor: string | null,
  id: string,
  shift: boolean,
): { selected: Set<string>; anchor: string } {
  const next = new Set(current);
  if (shift && anchor) {
    for (const x of rangeIds(ordered, anchor, id)) next.add(x);
    return { selected: next, anchor };
  }
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return { selected: next, anchor: id };
}

export const MAX_FORWARD_TARGETS = 5;

/** Toggle a forward target; refuses to exceed `max` (returns `limited: true`). */
export function toggleTarget(
  selected: readonly string[],
  jid: string,
  max: number = MAX_FORWARD_TARGETS,
): { selected: string[]; limited: boolean } {
  if (selected.includes(jid)) return { selected: selected.filter((j) => j !== jid), limited: false };
  if (selected.length >= max) return { selected: [...selected], limited: true };
  return { selected: [...selected, jid], limited: false };
}

/** Selected ids in list order (so copy/forward preserve chronology). */
export function inOrder(ordered: readonly string[], selected: ReadonlySet<string>): string[] {
  return ordered.filter((id) => selected.has(id));
}
