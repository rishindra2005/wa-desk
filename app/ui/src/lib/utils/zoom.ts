// App zoom: browser-like steps, clamped to 50-200 %, driven by keyboard and Ctrl+wheel.

export const ZOOM_MIN = 50;
export const ZOOM_MAX = 200;
export const ZOOM_DEFAULT = 100;
/** The same ladder Chrome and Firefox use, within our range. */
export const ZOOM_STEPS = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200] as const;

export type ZoomAction = "in" | "out" | "reset";

export function clampZoom(p: number): number {
  if (!Number.isFinite(p)) return ZOOM_DEFAULT;
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(p)));
}

/** The next step above (`in`) or below (`out`) `current`; stays put at the ends. */
export function stepZoom(current: number, dir: "in" | "out"): number {
  const c = clampZoom(current);
  if (dir === "in") return ZOOM_STEPS.find((s) => s > c) ?? ZOOM_MAX;
  return [...ZOOM_STEPS].reverse().find((s) => s < c) ?? ZOOM_MIN;
}

/** Zoom level after `action` from `current`. */
export function applyZoomAction(current: number, action: ZoomAction): number {
  return action === "reset" ? ZOOM_DEFAULT : stepZoom(current, action);
}

type KeyLike = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey">;

/**
 * Which zoom action a key press means, or null. Ctrl/Cmd + `=`, `+` (also
 * Ctrl+Shift+=, which reports `+`, and the numpad plus), `-` / numpad minus,
 * `0` / numpad 0. Alt combinations are left alone (Alt+digit switches views).
 */
export function zoomActionForKey(e: KeyLike): ZoomAction | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return null;
  if (e.key === "+" || e.key === "=" || e.code === "Equal" || e.code === "NumpadAdd") return "in";
  if (e.key === "-" || e.key === "_" || e.code === "Minus" || e.code === "NumpadSubtract") return "out";
  if (e.key === "0" || e.code === "Digit0" || e.code === "Numpad0") return "reset";
  return null;
}

/** Direction of a Ctrl+wheel gesture (wheel up zooms in), or null without Ctrl/Cmd. */
export function zoomActionForWheel(e: Pick<WheelEvent, "deltaY" | "ctrlKey" | "metaKey">): "in" | "out" | null {
  if (!(e.ctrlKey || e.metaKey) || e.deltaY === 0) return null;
  return e.deltaY < 0 ? "in" : "out";
}
