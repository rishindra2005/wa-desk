<script lang="ts">
  import { Plus, Trash2, FlaskConical, CircleCheck, CircleX } from "@lucide/svelte";
  import * as api from "../../api";
  import type { Rule } from "../../bindings/Rule";
  import type { Action } from "../../bindings/Action";
  import type { MediaKind } from "../../bindings/MediaKind";
  import type { Message } from "../../bindings/Message";
  import type { RuleEval } from "../../bindings/RuleEval";
  import ChatPicker from "../../components/ChatPicker.svelte";
  import ChipsInput from "../../components/ChipsInput.svelte";
  import Toggle from "../../components/Toggle.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import { validateRule, hasErrors, regexError, TEMPLATE_VARS } from "../../utils/rules";
  import { timeOfDay, shortDate, mediaEmoji } from "../../utils/format";
  import { toasts } from "../../state/toasts.svelte";

  interface Props {
    rule: Rule;
    onsave: (r: Rule) => Promise<void> | void;
    oncancel: () => void;
    ondelete?: () => void;
  }
  let { rule: initial, onsave, oncancel, ondelete }: Props = $props();

  // svelte-ignore state_referenced_locally
  let rule = $state<Rule>(structuredClone($state.snapshot(initial)) as Rule);
  let submitted = $state(false);
  let saving = $state(false);
  let regexText = $state(rule.trigger.regex ?? "");
  let hoursOn = $state(!!(rule.trigger.active_from || rule.trigger.active_to));

  const errors = $derived(validateRule(rule));
  const liveRegexError = $derived(regexError(regexText || null));
  const KINDS: MediaKind[] = ["image", "video", "audio", "document", "sticker"];

  $effect(() => {
    rule.trigger.regex = regexText.trim() ? regexText : null;
  });
  $effect(() => {
    if (!hoursOn) {
      rule.trigger.active_from = null;
      rule.trigger.active_to = null;
    } else if (!rule.trigger.active_from && !rule.trigger.active_to) {
      rule.trigger.active_from = "22:00";
      rule.trigger.active_to = "07:00";
    }
  });

  const ACTION_TYPES: { type: Action["type"]; label: string }[] = [
    { type: "reply", label: "Reply" },
    { type: "download_media", label: "Download media" },
    { type: "notify", label: "Notify" },
    { type: "forward", label: "Forward" },
    { type: "tag", label: "Tag" },
    { type: "webhook", label: "Webhook" },
  ];

  function blank(type: Action["type"]): Action {
    switch (type) {
      case "reply": return { type, text: "" };
      case "download_media": return { type };
      case "notify": return { type, title: null };
      case "forward": return { type, to_jid: "" };
      case "tag": return { type, label: "" };
      case "webhook": return { type, url: "https://" };
    }
  }

  function toggleKind(k: MediaKind) {
    const ks = rule.trigger.media_kinds;
    rule.trigger.media_kinds = ks.includes(k) ? ks.filter((x) => x !== k) : [...ks, k];
  }

  async function save() {
    submitted = true;
    if (hasErrors(errors)) return;
    saving = true;
    try {
      await onsave($state.snapshot(rule) as Rule);
    } finally {
      saving = false;
    }
  }

  // ---- test against a recent message
  let testChat = $state<string[]>([]);
  let samples = $state<Message[]>([]);
  let sampleId = $state<string>("");
  let evalResult = $state<RuleEval | null>(null);
  let testing = $state(false);

  $effect(() => {
    const jid = testChat[0];
    samples = [];
    sampleId = "";
    evalResult = null;
    if (!jid) return;
    api.getMessages({ chat_jid: jid, before: null, limit: 30 }).then(
      (p) => {
        samples = p.items.filter((m) => !m.deleted).reverse();
        sampleId = samples[0]?.id ?? "";
      },
      (e) => toasts.error(e),
    );
  });

  async function runTest() {
    const sample = samples.find((m) => m.id === sampleId);
    if (!sample) return;
    testing = true;
    try {
      evalResult = await api.testRule($state.snapshot(rule) as Rule, sample);
    } catch (e) {
      toasts.error(e, "Test failed");
    } finally {
      testing = false;
    }
  }

  function err(k: keyof typeof errors) {
    return submitted ? errors[k] : undefined;
  }
</script>

<form class="space-y-6" onsubmit={(e) => (e.preventDefault(), save())} data-testid="rule-editor" novalidate>
  <div class="grid gap-4 sm:grid-cols-[1fr_auto_auto] sm:items-end">
    <div>
      <label class="label" for="r-name">Rule name</label>
      <input id="r-name" class="input" bind:value={rule.name} aria-invalid={err("name") ? "true" : undefined} placeholder="e.g. Night auto-reply" data-testid="rule-name" />
      {#if err("name")}<p class="mt-1 text-xs text-danger" role="alert">{err("name")}</p>{/if}
    </div>
    <Toggle bind:checked={rule.enabled} label="Enabled" />
    <Toggle bind:checked={rule.dry_run} label="Dry run" />
  </div>

  <section class="space-y-3" aria-labelledby="trig-h">
    <h3 id="trig-h" class="section-title">When a message arrives…</h3>
    <div class="grid gap-3 sm:grid-cols-2">
      <ChatPicker bind:selected={rule.trigger.chat_jids} label="In chats" placeholder="Any direct chat" />
      <ChipsInput bind:values={rule.trigger.senders} label="From senders" placeholder="Phone or JID, Enter to add" />
      <ChipsInput bind:values={rule.trigger.keywords} label="Containing any keyword" placeholder="Keyword, Enter to add" testid="rule-keywords" />
      <div>
        <label class="label" for="r-regex">Matching regex</label>
        <input id="r-regex" class="input font-mono" bind:value={regexText} placeholder="optional, e.g. inv[-_ ]?\d+" aria-invalid={liveRegexError ? "true" : undefined} aria-describedby="r-regex-msg" data-testid="rule-regex" />
        <p id="r-regex-msg" class="mt-1 text-xs {liveRegexError ? 'text-danger' : 'text-faint'}" data-testid="regex-status">
          {#if liveRegexError}Invalid: {liveRegexError}{:else if regexText}Valid pattern (case-insensitive){:else}Leave empty to skip{/if}
        </p>
      </div>
    </div>
    <div class="flex flex-wrap items-center gap-1.5">
      <span class="label mb-0 mr-1">Media kinds</span>
      {#each KINDS as k (k)}
        <button type="button" class="chip capitalize" aria-pressed={rule.trigger.media_kinds.includes(k)} onclick={() => toggleKind(k)}>{mediaEmoji(k)} {k}</button>
      {/each}
    </div>
    <div class="grid gap-4 sm:grid-cols-2">
      <Toggle bind:checked={rule.trigger.include_groups} label="Include groups" description="Groups must also be listed in “In chats”." />
      <Toggle bind:checked={rule.trigger.mention_only} label="Only when I'm @mentioned" description="Applies in groups." />
    </div>
    <div class="flex flex-wrap items-end gap-3">
      <Toggle bind:checked={hoursOn} label="Active hours only" />
      {#if hoursOn}
        <label class="text-xs">From <input type="time" class="input mt-1 w-28" bind:value={rule.trigger.active_from} /></label>
        <label class="text-xs">To <input type="time" class="input mt-1 w-28" bind:value={rule.trigger.active_to} /></label>
      {/if}
      {#if err("hours")}<p class="text-xs text-danger" role="alert">{err("hours")}</p>{/if}
    </div>
  </section>

  <section class="space-y-3" aria-labelledby="act-h">
    <h3 id="act-h" class="section-title">…then do</h3>
    {#each rule.actions as a, i (i)}
      <div class="card flex items-start gap-3 p-3" data-testid="rule-action">
        <select
          class="input w-40"
          aria-label="Action type"
          value={a.type}
          onchange={(e) => (rule.actions[i] = blank((e.currentTarget as HTMLSelectElement).value as Action["type"]))}
        >
          {#each ACTION_TYPES as t (t.type)}<option value={t.type}>{t.label}</option>{/each}
        </select>
        <div class="min-w-0 flex-1">
          {#if a.type === "reply"}
            <textarea class="input" rows="2" bind:value={a.text} placeholder="Reply text" aria-label="Reply text" data-testid="reply-text"></textarea>
            <p class="mt-1 flex flex-wrap gap-1 text-[11px] text-faint">
              Variables:
              {#each TEMPLATE_VARS as v (v)}
                <button type="button" class="kbd hover:text-fg" onclick={() => a.type === "reply" && (a.text += v)}>{v}</button>
              {/each}
            </p>
          {:else if a.type === "download_media"}
            <p class="pt-1.5 text-xs text-muted">Download the message's media to the archive.</p>
          {:else if a.type === "notify"}
            <input class="input" placeholder="Title (optional, supports variables)" aria-label="Notification title" value={a.title ?? ""} oninput={(e) => a.type === "notify" && (a.title = e.currentTarget.value || null)} />
          {:else if a.type === "forward"}
            <ChatPicker
              selected={a.to_jid ? [a.to_jid] : []}
              multiple={false}
              label="Forward to"
              placeholder="Pick a chat"
              onchange={(j) => a.type === "forward" && (a.to_jid = j[0] ?? "")}
            />
          {:else if a.type === "tag"}
            <input class="input" placeholder="Label, e.g. finance" aria-label="Tag label" bind:value={a.label} />
          {:else if a.type === "webhook"}
            <input class="input font-mono" placeholder="https://…" aria-label="Webhook URL" bind:value={a.url} data-testid="webhook-url" />
          {/if}
          {#if submitted && errors[`action.${i}`]}<p class="mt-1 text-xs text-danger" role="alert">{errors[`action.${i}`]}</p>{/if}
        </div>
        <button type="button" class="icon-btn" aria-label="Remove action" onclick={() => (rule.actions = rule.actions.filter((_, j) => j !== i))}><Trash2 size={15} /></button>
      </div>
    {/each}
    {#if err("actions")}<p class="text-xs text-danger" role="alert">{err("actions")}</p>{/if}
    <button type="button" class="btn btn-sm" onclick={() => (rule.actions = [...rule.actions, blank("reply")])}><Plus size={13} /> Add action</button>
    <div class="w-56">
      <label class="label" for="r-cool">Cooldown per chat (seconds)</label>
      <input id="r-cool" type="number" min="0" class="input" bind:value={rule.cooldown_secs} aria-invalid={err("cooldown") ? "true" : undefined} />
      {#if err("cooldown")}<p class="mt-1 text-xs text-danger">{err("cooldown")}</p>{/if}
    </div>
  </section>

  <section class="card space-y-3 bg-surface-2/40 p-4" aria-labelledby="test-h">
    <h3 id="test-h" class="flex items-center gap-2 text-[13px] font-semibold"><FlaskConical size={15} /> Test this rule</h3>
    <div class="grid gap-3 sm:grid-cols-2">
      <ChatPicker bind:selected={testChat} multiple={false} label="Chat" placeholder="Pick a chat" testid="test-chat" />
      <div>
        <label class="label" for="r-sample">Recent message</label>
        <select id="r-sample" class="input" bind:value={sampleId} disabled={!samples.length} data-testid="test-sample">
          {#each samples as m (m.id)}
            <option value={m.id}>{shortDate(m.ts)} {timeOfDay(m.ts)} · {m.is_from_me ? "You" : m.sender_name}: {(m.content || m.media?.filename || m.media?.kind || "").slice(0, 60)}</option>
          {/each}
        </select>
      </div>
    </div>
    <button type="button" class="btn btn-sm" disabled={!sampleId || testing} onclick={runTest} data-testid="run-test">{#if testing}<Spinner size={12} />{/if} Run test</button>
    {#if evalResult}
      <div class="rounded-md border p-3 {evalResult.matched ? 'border-accent bg-accent-soft' : 'border-border bg-surface'}" data-testid="test-result" aria-live="polite">
        <p class="flex items-center gap-1.5 text-[13px] font-semibold {evalResult.matched ? 'text-accent-text' : 'text-muted'}">
          {#if evalResult.matched}<CircleCheck size={15} /> Would fire{:else}<CircleX size={15} /> Would not fire{/if}
        </p>
        <ul class="mt-2 space-y-0.5 text-xs">{#each evalResult.reasons as r, i (i)}<li>{r}</li>{/each}</ul>
        {#if evalResult.planned.length}
          <p class="mt-2 text-xs font-semibold">Planned actions</p>
          <ul class="mt-1 space-y-0.5 text-xs">{#each evalResult.planned as p, i (i)}<li>• {p}</li>{/each}</ul>
        {/if}
      </div>
    {/if}
  </section>

  <div class="flex items-center gap-2 border-t border-border pt-4">
    {#if ondelete}<button type="button" class="btn btn-danger" onclick={ondelete}><Trash2 size={14} /> Delete</button>{/if}
    <div class="flex-1"></div>
    {#if submitted && hasErrors(errors)}<p class="text-xs text-danger" data-testid="rule-invalid">Fix the highlighted fields</p>{/if}
    <button type="button" class="btn" onclick={oncancel}>Cancel</button>
    <button type="submit" class="btn btn-primary" disabled={saving} data-testid="save-rule">{#if saving}<Spinner size={13} />{/if} Save rule</button>
  </div>
</form>
