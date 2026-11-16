import { describe, expect, it } from "vitest";
import type { Message } from "../bindings/Message";
import { evaluateRule, hasErrors, inWindow, newRule, regexError, renderTemplate, validateRule } from "./rules";

const msg = (over: Partial<Message> = {}): Message => ({
  id: "1",
  chat_jid: "919845012345@s.whatsapp.net",
  sender_jid: "919845012345@s.whatsapp.net",
  sender_name: "Ananya",
  is_from_me: false,
  ts: 1_700_000_000,
  content: "Urgent: please call me",
  media: null,
  quoted: null,
  reactions: [],
  deleted: false,
  status: null,
  starred: false,
  ...over,
});

describe("validateRule", () => {
  it("requires a name and actions", () => {
    const r = { ...newRule(), actions: [] };
    const e = validateRule(r);
    expect(e.name).toBeDefined();
    expect(e.actions).toBeDefined();
  });
  it("flags invalid regex", () => {
    const r = newRule();
    r.name = "x";
    r.trigger.regex = "(unclosed";
    expect(validateRule(r).regex).toBeDefined();
    expect(regexError("a+b")).toBeNull();
  });
  it("requires both active hours", () => {
    const r = newRule();
    r.name = "x";
    r.trigger.active_from = "22:00";
    expect(validateRule(r).hours).toMatch(/both/);
    r.trigger.active_to = "25:00";
    expect(validateRule(r).hours).toMatch(/HH:MM/);
  });
  it("validates webhook URLs and empty replies", () => {
    const r = newRule();
    r.name = "x";
    r.actions = [{ type: "webhook", url: "ftp://x" }, { type: "reply", text: " " }];
    const e = validateRule(r);
    expect(e["action.0"]).toBeDefined();
    expect(e["action.1"]).toBeDefined();
  });
  it("accepts a good rule", () => {
    const r = newRule();
    r.name = "ok";
    expect(hasErrors(validateRule(r))).toBe(false);
  });
});

describe("evaluateRule", () => {
  it("matches keywords case-insensitively and plans actions", () => {
    const r = newRule();
    r.name = "n";
    r.trigger.keywords = ["URGENT"];
    r.actions = [{ type: "reply", text: "Hi {sender_name}" }];
    const ev = evaluateRule(r, msg(), "Ananya");
    expect(ev.matched).toBe(true);
    expect(ev.planned[0]).toContain("Hi Ananya");
    expect(ev.planned[0]).toContain("[dry-run]");
  });
  it("explains non-matches", () => {
    const r = newRule();
    r.trigger.keywords = ["invoice"];
    const ev = evaluateRule(r, msg(), "Ananya");
    expect(ev.matched).toBe(false);
    expect(ev.reasons.some((x) => x.startsWith("✗"))).toBe(true);
    expect(ev.planned).toEqual([]);
  });
  it("skips own messages and unlisted groups", () => {
    const r = newRule();
    expect(evaluateRule(r, msg({ is_from_me: true }), "x").matched).toBe(false);
    expect(evaluateRule(r, msg({ chat_jid: "1@g.us" }), "G").matched).toBe(false);
  });
  it("respects active hours (wrapping midnight)", () => {
    const at = (h: number) => new Date(2025, 0, 1, h, 30);
    expect(inWindow(at(23), "22:00", "07:00")).toBe(true);
    expect(inWindow(at(3), "22:00", "07:00")).toBe(true);
    expect(inWindow(at(12), "22:00", "07:00")).toBe(false);
    expect(inWindow(at(9), "09:00", "17:00")).toBe(true);
  });
  it("renders templates, leaving unknown vars", () => {
    expect(renderTemplate("{a} {b}", { a: "1" })).toBe("1 {b}");
  });
});
