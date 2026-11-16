import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { clearMocks } from "@tauri-apps/api/mocks";
import { installMock } from "./mock/install";
import type { MockBackend } from "./mock/backend";
import ChatList from "./views/chats/ChatList.svelte";
import Composer from "./views/chats/Composer.svelte";
import RuleEditor from "./views/automations/RuleEditor.svelte";
import Toggle from "./components/Toggle.svelte";
import { newRule } from "./utils/rules";
import type { ChatSummary } from "./bindings/ChatSummary";
import { toasts } from "./state/toasts.svelte";

let b: MockBackend;
beforeEach(() => {
  b = installMock({ now: 1_760_000_000, quiet: true, latency: 0, speed: 50 });
});
afterEach(() => {
  cleanup();
  b.stop();
  clearMocks();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  toasts.items = [];
});

const names = () => screen.queryAllByTestId("chat-name").map((e) => e.textContent);

describe("ChatList", () => {
  it("renders chats and filters by text", async () => {
    render(ChatList, { selected: null, onselect: () => {} });
    await waitFor(() => expect(names().length).toBeGreaterThan(5));
    await fireEvent.input(screen.getByTestId("chat-filter"), { target: { value: "priya" } });
    await waitFor(() => expect(names().every((n) => n?.toLowerCase().includes("priya"))).toBe(true));
    expect(names()).toContain("Priya Patel");
  });

  it("filters groups via tabs and unread toggle", async () => {
    render(ChatList, { selected: null, onselect: () => {} });
    await waitFor(() => expect(names().length).toBeGreaterThan(5));
    await fireEvent.click(screen.getByRole("tab", { name: "Groups" }));
    await waitFor(() => expect(names()).toContain("Family 🏠"));
    await waitFor(() => expect(names()).not.toContain("Ananya Rao"));
    await fireEvent.click(screen.getByRole("tab", { name: "All" }));
    await fireEvent.click(screen.getByRole("button", { name: "Unread" }));
    await waitFor(() => expect(screen.queryAllByTestId("unread-dot").length).toBe(names().length));
  });

  it("shows an empty state when nothing matches", async () => {
    render(ChatList, { selected: null, onselect: () => {} });
    await fireEvent.input(screen.getByTestId("chat-filter"), { target: { value: "zzzz-nothing" } });
    await waitFor(() => expect(screen.getByText("No chats match")).toBeTruthy());
  });

  it("calls onselect with the chat", async () => {
    const onselect = vi.fn();
    render(ChatList, { selected: null, onselect });
    await waitFor(() => expect(names().length).toBeGreaterThan(0));
    await fireEvent.click(screen.getAllByTestId("chat-row")[0]!);
    expect(onselect).toHaveBeenCalledWith(expect.objectContaining({ jid: b.chats[0]!.jid }));
  });
});

function summary(i: number): ChatSummary {
  const c = b.chats[i]!;
  return b.summary(c);
}

describe("Composer", () => {
  beforeEach(() => localStorage.clear());

  it("sends on Enter, keeps newline on Shift+Enter", async () => {
    const onsend = vi.fn();
    render(Composer, { chat: summary(0), replyTo: null, confirmSends: true, oncancelreply: () => {}, onsend });
    const input = screen.getByTestId("composer-input") as HTMLTextAreaElement;
    await fireEvent.input(input, { target: { value: "hello there" } });
    await fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(onsend).not.toHaveBeenCalled();
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onsend).toHaveBeenCalledWith(expect.objectContaining({ recipient: b.chats[0]!.jid, text: "hello there" }), null);
    expect(input.value).toBe("");
  });

  it("asks for confirmation in groups when confirm_sends is on", async () => {
    const onsend = vi.fn();
    const gi = b.chats.findIndex((c) => c.chat_type === "group");
    render(Composer, { chat: summary(gi), replyTo: null, confirmSends: true, oncancelreply: () => {}, onsend });
    await fireEvent.input(screen.getByTestId("composer-input"), { target: { value: "hi group" } });
    await fireEvent.click(screen.getByTestId("send-btn"));
    expect(screen.getByTestId("confirm-send")).toBeTruthy();
    expect(onsend).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByTestId("confirm-send-btn"));
    expect(onsend).toHaveBeenCalledTimes(1);
  });

  it("skips confirmation when disabled and passes reply_to", async () => {
    const onsend = vi.fn();
    const gi = b.chats.findIndex((c) => c.chat_type === "group");
    const target = b.chats[gi]!.messages.at(-3)!;
    const cancel = vi.fn();
    render(Composer, { chat: summary(gi), replyTo: target, confirmSends: false, oncancelreply: cancel, onsend });
    expect(screen.getByTestId("reply-chip")).toBeTruthy();
    await fireEvent.input(screen.getByTestId("composer-input"), { target: { value: "re" } });
    await fireEvent.click(screen.getByTestId("send-btn"));
    expect(onsend).toHaveBeenCalledWith(expect.objectContaining({ reply_to: target.id }), target);
    expect(cancel).toHaveBeenCalled();
  });

  it("persists a draft per chat and restores it", async () => {
    const chat = summary(0);
    const first = render(Composer, { chat, replyTo: null, confirmSends: false, oncancelreply: () => {}, onsend: () => {} });
    await fireEvent.input(screen.getByTestId("composer-input"), { target: { value: "unfinished thought" } });
    expect(localStorage.getItem(`wa-desk.draft.${chat.jid}`)).toBe("unfinished thought");
    first.unmount();
    render(Composer, { chat, replyTo: null, confirmSends: false, oncancelreply: () => {}, onsend: () => {} });
    await waitFor(() => expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("unfinished thought"));
  });

  it("clears the draft after sending", async () => {
    const chat = summary(0);
    render(Composer, { chat, replyTo: null, confirmSends: false, oncancelreply: () => {}, onsend: () => {} });
    await fireEvent.input(screen.getByTestId("composer-input"), { target: { value: "bye" } });
    await fireEvent.click(screen.getByTestId("send-btn"));
    expect(localStorage.getItem(`wa-desk.draft.${chat.jid}`)).toBeNull();
  });

  it("disables send when empty", () => {
    render(Composer, { chat: summary(0), replyTo: null, confirmSends: false, oncancelreply: () => {}, onsend: () => {} });
    expect((screen.getByTestId("send-btn") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("RuleEditor", () => {
  it("blocks saving an invalid rule and shows errors", async () => {
    const onsave = vi.fn();
    render(RuleEditor, { rule: newRule(), onsave, oncancel: () => {} });
    await fireEvent.click(screen.getByTestId("save-rule"));
    expect(onsave).not.toHaveBeenCalled();
    expect(screen.getByText("Give the rule a name")).toBeTruthy();
    expect(screen.getByTestId("rule-invalid")).toBeTruthy();
  });

  it("validates regex live", async () => {
    render(RuleEditor, { rule: newRule(), onsave: () => {}, oncancel: () => {} });
    const re = screen.getByTestId("rule-regex");
    await fireEvent.input(re, { target: { value: "(oops" } });
    expect(screen.getByTestId("regex-status").textContent).toMatch(/Invalid/);
    expect(re.getAttribute("aria-invalid")).toBe("true");
    await fireEvent.input(re, { target: { value: "inv\\d+" } });
    expect(screen.getByTestId("regex-status").textContent).toMatch(/Valid pattern/);
  });

  it("saves a valid rule with keywords", async () => {
    const onsave = vi.fn();
    render(RuleEditor, { rule: newRule(), onsave, oncancel: () => {} });
    await fireEvent.input(screen.getByTestId("rule-name"), { target: { value: "My rule" } });
    const kw = screen.getByTestId("rule-keywords").querySelector("input")!;
    await fireEvent.input(kw, { target: { value: "urgent" } });
    await fireEvent.keyDown(kw, { key: "Enter" });
    await fireEvent.click(screen.getByTestId("save-rule"));
    await waitFor(() => expect(onsave).toHaveBeenCalled());
    expect(onsave.mock.calls[0]![0]).toMatchObject({ name: "My rule", trigger: { keywords: ["urgent"] } });
  });

  it("rejects a bad webhook URL", async () => {
    const onsave = vi.fn();
    const r = newRule();
    r.name = "hook";
    r.actions = [{ type: "webhook", url: "notaurl" }];
    render(RuleEditor, { rule: r, onsave, oncancel: () => {} });
    await fireEvent.click(screen.getByTestId("save-rule"));
    expect(onsave).not.toHaveBeenCalled();
    expect(screen.getByText(/Webhook URL must start/)).toBeTruthy();
  });
});

describe("Toggle", () => {
  it("is bindable without onchange and controlled with it", async () => {
    const onchange = vi.fn();
    render(Toggle, { checked: false, label: "X", onchange });
    const sw = screen.getByRole("switch");
    await fireEvent.click(sw);
    expect(onchange).toHaveBeenCalledWith(true);
    expect(sw.getAttribute("aria-checked")).toBe("false");
  });
});
