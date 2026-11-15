// Client-side rule validation (mirrors what `save_rule` rejects with `Invalid`)
// and a reference evaluator used by the mock backend's `test_rule`.

import type { Action } from "../bindings/Action";
import type { Message } from "../bindings/Message";
import type { Rule } from "../bindings/Rule";
import type { RuleEval } from "../bindings/RuleEval";
import type { Trigger } from "../bindings/Trigger";
import { isHHMM } from "./format";

/** Field → message for every validation problem (empty = valid). */
export type RuleErrors = Partial<Record<"name" | "regex" | "hours" | "actions" | "cooldown" | `action.${number}`, string>>;

/** Returns null when the pattern compiles, otherwise the engine's message. */
export function regexError(pattern: string | null): string | null {
  if (!pattern) return null;
  try {
    new RegExp(pattern, "u");
    return null;
  } catch (e) {
    return e instanceof Error ? e.message.replace(/^Invalid regular expression: /, "") : "Invalid pattern";
  }
}

export function isHttpUrl(s: string): boolean {
  try {
    const u = new URL(s);
    return (u.protocol === "http:" || u.protocol === "https:") && !!u.hostname;
  } catch {
    return false;
  }
}

export function validateAction(a: Action): string | null {
  switch (a.type) {
    case "reply":
      return a.text.trim() ? null : "Reply text is empty";
    case "forward":
      return a.to_jid.trim() ? null : "Pick a chat to forward to";
    case "tag":
      return a.label.trim() ? null : "Tag label is empty";
    case "webhook":
      return isHttpUrl(a.url) ? null : "Webhook URL must start with http:// or https://";
    default:
      return null;
  }
}

export function validateRule(rule: Rule): RuleErrors {
  const errs: RuleErrors = {};
  if (!rule.name.trim()) errs.name = "Give the rule a name";
  const re = regexError(rule.trigger.regex);
  if (re) errs.regex = re;
  const { active_from: f, active_to: t } = rule.trigger;
  if ((f && !t) || (!f && t)) errs.hours = "Set both start and end, or neither";
  else if ((f && !isHHMM(f)) || (t && !isHHMM(t))) errs.hours = "Use HH:MM (24 h)";
  if (rule.actions.length === 0) errs.actions = "Add at least one action";
  rule.actions.forEach((a, i) => {
    const e = validateAction(a);
    if (e) errs[`action.${i}`] = e;
  });
  if (!Number.isFinite(rule.cooldown_secs) || rule.cooldown_secs < 0) errs.cooldown = "Cooldown must be ≥ 0";
  return errs;
}

export function hasErrors(e: RuleErrors): boolean {
  return Object.keys(e).length > 0;
}

export function emptyTrigger(): Trigger {
  return {
    chat_jids: [],
    include_groups: false,
    senders: [],
    keywords: [],
    regex: null,
    media_kinds: [],
    active_from: null,
    active_to: null,
    mention_only: false,
  };
}

export function newRule(): Rule {
  return {
    id: null,
    name: "",
    enabled: true,
    dry_run: true,
    trigger: emptyTrigger(),
    actions: [{ type: "notify", title: null }],
    cooldown_secs: 600,
    fired_count: 0,
    last_fired_at: null,
  };
}

/** Expand reply template variables. */
export function renderTemplate(tpl: string, vars: Record<string, string>): string {
  return tpl.replace(/\{(\w+)\}/g, (m, k: string) => vars[k] ?? m);
}

export const TEMPLATE_VARS = ["{sender_name}", "{chat_name}", "{time}", "{text}"] as const;

function minutesOf(hhmm: string): number {
  const [h = "0", m = "0"] = hhmm.split(":");
  return Number(h) * 60 + Number(m);
}

/** Whether local time `d` is in [from, to) (wraps past midnight). */
export function inWindow(d: Date, from: string, to: string): boolean {
  const now = d.getHours() * 60 + d.getMinutes();
  const a = minutesOf(from);
  const b = minutesOf(to);
  return a <= b ? now >= a && now < b : now >= a || now < b;
}

export function describeAction(a: Action, vars: Record<string, string>): string {
  switch (a.type) {
    case "reply":
      return `Reply: “${renderTemplate(a.text, vars)}”`;
    case "download_media":
      return "Download media";
    case "notify":
      return `Desktop notification${a.title ? `: ${renderTemplate(a.title, vars)}` : ""}`;
    case "forward":
      return `Forward to ${a.to_jid}`;
    case "tag":
      return `Tag as “${a.label}”`;
    case "webhook":
      return `POST to ${a.url}`;
  }
}

/** Reference evaluation of a rule against a message (local time). */
export function evaluateRule(rule: Rule, msg: Message, chatName: string, now = new Date()): RuleEval {
  const t = rule.trigger;
  const reasons: string[] = [];
  let ok = true;
  const check = (pass: boolean, yes: string, no: string) => {
    reasons.push(`${pass ? "✓" : "✗"} ${pass ? yes : no}`);
    if (!pass) ok = false;
  };
  const isGroup = msg.chat_jid.endsWith("@g.us");
  if (msg.is_from_me) check(false, "", "Message is from me (rules only react to incoming messages)");
  if (isGroup) {
    check(t.include_groups, "Groups are included", "Message is in a group and groups are not included");
    check(
      t.chat_jids.includes(msg.chat_jid),
      `Group “${chatName}” is listed`,
      `Group “${chatName}” is not in the chat list`,
    );
    if (t.mention_only) {
      check(msg.content.includes("@"), "Message mentions someone", "Mention-only: we are not @mentioned");
    }
  } else if (t.chat_jids.length) {
    check(t.chat_jids.includes(msg.chat_jid), `Chat “${chatName}” is listed`, `Chat “${chatName}” is not listed`);
  } else {
    reasons.push("✓ Any direct chat");
  }
  if (t.senders.length) {
    const s = msg.sender_jid ?? "";
    const hit = t.senders.some((x) => s === x || s.startsWith(`${x.replace(/^\+/, "")}@`));
    check(hit, `Sender ${msg.sender_name} matches`, `Sender ${msg.sender_name} is not listed`);
  }
  const text = msg.content.toLowerCase();
  if (t.keywords.length) {
    const k = t.keywords.find((kw) => text.includes(kw.toLowerCase()));
    check(!!k, `Keyword “${k ?? ""}” found`, `None of the keywords (${t.keywords.join(", ")}) found`);
  }
  if (t.regex) {
    let m = false;
    try {
      m = new RegExp(t.regex, "iu").test(msg.content);
    } catch {
      m = false;
    }
    check(m, `Regex /${t.regex}/ matches`, `Regex /${t.regex}/ does not match`);
  }
  if (t.media_kinds.length) {
    const k = msg.media?.kind;
    check(!!k && t.media_kinds.includes(k), `Media kind ${k ?? ""} matches`, `Media kind ${k ?? "none"} not in ${t.media_kinds.join(", ")}`);
  }
  if (t.active_from && t.active_to) {
    check(
      inWindow(now, t.active_from, t.active_to),
      `Within active hours ${t.active_from}–${t.active_to}`,
      `Outside active hours ${t.active_from}–${t.active_to}`,
    );
  }
  if (!rule.enabled) reasons.push("ℹ Rule is disabled (would not fire live)");
  const vars = {
    sender_name: msg.sender_name,
    chat_name: chatName,
    time: new Date(msg.ts * 1000).toTimeString().slice(0, 5),
    text: msg.content,
  };
  const planned = ok ? rule.actions.map((a) => (rule.dry_run ? "[dry-run] " : "") + describeAction(a, vars)) : [];
  return { matched: ok, reasons, planned };
}
