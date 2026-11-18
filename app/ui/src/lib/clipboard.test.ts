import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { clearMocks } from "@tauri-apps/api/mocks";
import * as api from "./api";
import { type MockBackend } from "./mock/backend";
import { installMock } from "./mock/install";
import { app } from "./state/app.svelte";

let b: MockBackend;

beforeEach(() => {
  b = installMock({ now: 1_760_000_000, quiet: true, latency: 0, speed: 50 });
});
afterEach(() => {
  b.stop();
  clearMocks();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("stage_pasted_file (mock)", () => {
  it("returns an absolute outbox path and keeps the name", async () => {
    const p = await api.stagePastedFile("image.png", "image/png", "aGk=");
    expect(p).toMatch(/^\/.+\/outbox\/[0-9a-f-]{36}\/image\.png$/);
    expect(b.staged).toHaveLength(1);
    // A name with path parts collapses to the base name.
    expect(await api.stagePastedFile("../../etc/x.png", null, "aGk=")).toMatch(/\/x\.png$/);
  });
  it("names nameless items from their MIME type", async () => {
    expect(await api.stagePastedFile(null, "image/jpeg", "aGk=")).toMatch(/\.jpg$/);
  });
  it("refuses empty data, secrets and read-only mode", async () => {
    await expect(api.stagePastedFile("a.png", null, "")).rejects.toMatchObject({ kind: "invalid" });
    await expect(api.stagePastedFile("messages.db", null, "aGk=")).rejects.toMatchObject({ kind: "forbidden" });
    b.readOnly = true;
    await expect(api.stagePastedFile("a.png", null, "aGk=")).rejects.toMatchObject({ kind: "forbidden" });
  });
});

describe("copy_image (mock)", () => {
  it("copies downloaded images only", async () => {
    const c = b.chats.find((x) => x.messages.some((m) => m.media?.kind === "image" && m.media.url))!;
    const img = c.messages.find((m) => m.media?.kind === "image" && m.media.url)!;
    await api.copyImage(c.jid, img.id);
    expect(b.copiedImages).toEqual([{ chatJid: c.jid, messageId: img.id }]);
    const doc = b.chats.flatMap((x) => x.messages.map((m) => ({ x, m }))).find(({ m }) => m.media?.kind === "document" && m.media.url)!;
    await expect(api.copyImage(doc.x.jid, doc.m.id)).rejects.toMatchObject({ kind: "invalid" });
  });
});

describe("zoom", () => {
  it("set_zoom clamps and is stored in the settings", async () => {
    expect(await api.setZoom(120)).toBe(120);
    expect(await api.setZoom(5)).toBe(50);
    expect(await api.setZoom(900)).toBe(200);
    expect((await api.getSettings()).zoom_percent).toBe(200);
  });

  it("app.zoomBy steps like a browser, persists and shows the indicator", async () => {
    app.zoom = 100;
    await app.zoomBy("in");
    expect(app.zoom).toBe(110);
    expect(app.zoomShown).toBe(true);
    await app.zoomBy("in");
    await app.zoomBy("out");
    await app.zoomBy("out");
    await app.zoomBy("out");
    expect(app.zoom).toBe(90);
    await app.zoomBy("reset");
    expect(app.zoom).toBe(100);
    expect(b.zooms).toEqual([110, 125, 110, 100, 90, 100]);
    for (let i = 0; i < 20; i++) await app.zoomBy("in");
    expect(app.zoom).toBe(200);
  });
});
