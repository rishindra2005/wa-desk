import { describe, expect, it } from "vitest";
import { applyZoomAction, clampZoom, stepZoom, zoomActionForKey, zoomActionForWheel } from "./zoom";

const key = (o: Partial<KeyboardEvent>) => ({ key: "", code: "", ctrlKey: true, metaKey: false, altKey: false, ...o });

describe("zoom steps", () => {
  it("walks the browser ladder and stops at the ends", () => {
    expect(stepZoom(100, "in")).toBe(110);
    expect(stepZoom(110, "in")).toBe(125);
    expect(stepZoom(100, "out")).toBe(90);
    expect(stepZoom(200, "in")).toBe(200);
    expect(stepZoom(50, "out")).toBe(50);
  });
  it("snaps off-ladder values to the next step", () => {
    expect(stepZoom(103, "in")).toBe(110);
    expect(stepZoom(103, "out")).toBe(100);
  });
  it("clamps and sanitises", () => {
    expect(clampZoom(10)).toBe(50);
    expect(clampZoom(999)).toBe(200);
    expect(clampZoom(Number.NaN)).toBe(100);
    expect(clampZoom(110.4)).toBe(110);
  });
  it("reset goes to 100", () => {
    expect(applyZoomAction(175, "reset")).toBe(100);
    expect(applyZoomAction(100, "in")).toBe(110);
  });
});

describe("zoomActionForKey", () => {
  it("maps every zoom-in spelling", () => {
    expect(zoomActionForKey(key({ key: "=", code: "Equal" }))).toBe("in");
    expect(zoomActionForKey(key({ key: "+", code: "Equal" }))).toBe("in"); // Ctrl+Shift+=
    expect(zoomActionForKey(key({ key: "+", code: "NumpadAdd" }))).toBe("in");
  });
  it("maps zoom out and reset", () => {
    expect(zoomActionForKey(key({ key: "-", code: "Minus" }))).toBe("out");
    expect(zoomActionForKey(key({ key: "-", code: "NumpadSubtract" }))).toBe("out");
    expect(zoomActionForKey(key({ key: "0", code: "Digit0" }))).toBe("reset");
    expect(zoomActionForKey(key({ key: "0", code: "Numpad0" }))).toBe("reset");
  });
  it("needs Ctrl or Cmd and ignores Alt chords", () => {
    expect(zoomActionForKey(key({ key: "=", code: "Equal", ctrlKey: false }))).toBeNull();
    expect(zoomActionForKey(key({ key: "=", code: "Equal", ctrlKey: false, metaKey: true }))).toBe("in");
    expect(zoomActionForKey(key({ key: "1", code: "Digit1", altKey: true }))).toBeNull();
    expect(zoomActionForKey(key({ key: "0", code: "Digit0", altKey: true }))).toBeNull();
    expect(zoomActionForKey(key({ key: "k", code: "KeyK" }))).toBeNull();
  });
});

describe("zoomActionForWheel", () => {
  it("only reacts with Ctrl held", () => {
    expect(zoomActionForWheel({ deltaY: -100, ctrlKey: true, metaKey: false })).toBe("in");
    expect(zoomActionForWheel({ deltaY: 100, ctrlKey: true, metaKey: false })).toBe("out");
    expect(zoomActionForWheel({ deltaY: -100, ctrlKey: false, metaKey: false })).toBeNull();
    expect(zoomActionForWheel({ deltaY: 0, ctrlKey: true, metaKey: false })).toBeNull();
  });
});
