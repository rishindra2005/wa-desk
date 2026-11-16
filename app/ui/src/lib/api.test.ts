import { afterEach, describe, expect, it, vi } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import * as api from "./api";
import { ApiError, toAppError } from "./api";
import { friendly } from "./errors";

afterEach(() => clearMocks());

describe("toAppError", () => {
  it("keeps contract errors", () => {
    const e = toAppError({ kind: "bridge_down", message: "offline", detail: "x" });
    expect(e).toBeInstanceOf(ApiError);
    expect(e).toBeInstanceOf(Error);
    expect(e.kind).toBe("bridge_down");
    expect(e.message).toBe("offline");
    expect(e.detail).toBe("x");
    expect(e.toJSON()).toEqual({ kind: "bridge_down", message: "offline", detail: "x" });
  });
  it("parses JSON strings", () => {
    expect(toAppError('{"kind":"invalid","message":"bad regex"}').kind).toBe("invalid");
  });
  it("wraps plain strings and Errors as internal", () => {
    expect(toAppError("boom")).toMatchObject({ kind: "internal", message: "boom" });
    expect(toAppError(new TypeError("nope"))).toMatchObject({ kind: "internal", message: "nope" });
  });
  it("handles unknown kinds, nulls and weird values", () => {
    expect(toAppError({ kind: "weird", message: "m" })).toMatchObject({ kind: "internal", message: "m" });
    expect(toAppError(null).kind).toBe("internal");
    expect(toAppError(42).detail).toBe("42");
    expect(toAppError({ kind: "timeout" }).message).toBe("timeout");
  });
  it("is idempotent", () => {
    const e = toAppError({ kind: "busy", message: "b" });
    expect(toAppError(e)).toBe(e);
  });
});

describe("friendly errors", () => {
  it("maps bridge_down to an action to System", () => {
    const f = friendly({ kind: "bridge_down", message: "bridge_down" });
    expect(f.title).toBe("WhatsApp bridge is offline");
    expect(f.action?.view).toBe("system");
  });
  it("keeps specific messages", () => {
    expect(friendly({ kind: "invalid", message: "Days must be ≥ 1" }).message).toBe("Days must be ≥ 1");
  });
});

describe("commands", () => {
  it("passes exact names and camelCase args", async () => {
    const calls: [string, unknown][] = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args]);
      return null;
    });
    await api.getMessageContext("c@s.whatsapp.net", "m1", 20);
    await api.markRead("c");
    await api.saveMediaAs("c", "m", "/tmp/x");
    await api.exportIssueBundle(null);
    expect(calls).toEqual([
      ["get_message_context", { chatJid: "c@s.whatsapp.net", messageId: "m1", radius: 20 }],
      ["mark_read", { chatJid: "c" }],
      ["save_media_as", { chatJid: "c", messageId: "m", dest: "/tmp/x" }],
      ["export_issue_bundle", { ids: null }],
    ]);
  });

  it("normalises rejections to ApiError", async () => {
    mockIPC(() => {
      throw { kind: "not_found", message: "Chat not found" };
    });
    await expect(api.getChat("x")).rejects.toMatchObject({ kind: "not_found", message: "Chat not found" });
    await expect(api.getChat("x")).rejects.toBeInstanceOf(ApiError);
  });

  it("lists every contract command once", () => {
    expect(new Set(api.COMMANDS).size).toBe(api.COMMANDS.length);
    expect(api.COMMANDS.length).toBe(50);
  });
});

describe("events", () => {
  it("onEvent delivers typed payloads via listen()", async () => {
    mockIPC(() => null, { shouldMockEvents: true });
    const cb = vi.fn();
    const un = await api.onEvent("db:changed", cb);
    await emit("db:changed", { chat_jids: ["a"] });
    expect(cb).toHaveBeenCalledWith({ chat_jids: ["a"] });
    await un();
  });

  it("subscribe disposes even before listen resolves", async () => {
    mockIPC(() => null, { shouldMockEvents: true });
    const cb = vi.fn();
    const dispose = api.subscribe("job:progress", cb);
    dispose();
    await new Promise((r) => setTimeout(r, 0));
    await emit("job:progress", { id: 1, done: 1, total: 2, phase: "x", message: null });
    expect(cb).not.toHaveBeenCalled();
  });
});
