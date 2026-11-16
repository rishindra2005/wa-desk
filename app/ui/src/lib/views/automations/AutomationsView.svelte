<script lang="ts">
  import { untrack } from "svelte";
  import { Plus, Bot, TriangleAlert, Pencil, Zap } from "@lucide/svelte";
  import * as api from "../../api";
  import type { Rule } from "../../bindings/Rule";
  import type { AutomationEvent } from "../../bindings/AutomationEvent";
  import RuleEditor from "./RuleEditor.svelte";
  import Toggle from "../../components/Toggle.svelte";
  import Modal from "../../components/Modal.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import { app } from "../../state/app.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { newRule } from "../../utils/rules";
  import { timeAgo, dateTime } from "../../utils/format";

  let rules = $state<Rule[]>([]);
  let log = $state<AutomationEvent[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let editing = $state<Rule | null>(null);
  let confirmEnable = $state(false);
  let confirmDelete = $state<Rule | null>(null);

  const enabled = $derived(app.settings?.automation_enabled ?? false);

  async function load() {
    try {
      [rules, log] = await Promise.all([api.listRules(), api.automationLog(100)]);
      error = null;
    } catch (e) {
      error = e;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    untrack(() => void load());
    return api.subscribe("automation:fired", (e) => {
      log = [e, ...log].slice(0, 200);
      const r = rules.find((x) => x.id === e.rule_id);
      if (r) {
        r.fired_count++;
        r.last_fired_at = e.at;
      }
    });
  });

  async function setEnabled(on: boolean) {
    confirmEnable = false;
    try {
      app.settings = await api.setAutomationEnabled(on);
      toasts.success(on ? "Automations enabled" : "Automations disabled");
    } catch (e) {
      toasts.error(e);
    }
  }

  async function save(r: Rule) {
    try {
      const saved = await api.saveRule(r);
      const i = rules.findIndex((x) => x.id === saved.id);
      if (i >= 0) rules[i] = saved;
      else rules = [...rules, saved];
      editing = null;
      toasts.success("Rule saved");
    } catch (e) {
      toasts.error(e, "Couldn't save rule");
    }
  }

  async function toggleRule(r: Rule, on: boolean) {
    r.enabled = on;
    try {
      const saved = await api.saveRule({ ...$state.snapshot(r), enabled: on } as Rule);
      const i = rules.findIndex((x) => x.id === saved.id);
      if (i >= 0) rules[i] = saved;
    } catch (e) {
      r.enabled = !on;
      toasts.error(e);
    }
  }

  async function remove(r: Rule) {
    confirmDelete = null;
    if (r.id === null) return;
    try {
      await api.deleteRule(r.id);
      rules = rules.filter((x) => x.id !== r.id);
      editing = null;
      toasts.success("Rule deleted");
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-5xl space-y-5 p-6">
    <section class="card flex items-start gap-4 p-5 {enabled ? '' : 'bg-surface-2/40'}" aria-labelledby="auto-master">
      <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-full {enabled ? 'bg-accent-soft text-accent' : 'bg-surface-3 text-muted'}"><Bot size={20} /></div>
      <div class="min-w-0 flex-1 space-y-1">
        <h2 id="auto-master" class="text-[15px] font-semibold">Automations are {enabled ? "on" : "off"}</h2>
        <p class="text-[13px] text-muted">Rules react to <em>new</em> incoming messages only — enabling sets a watermark so old messages never fire. Rules in <strong>dry-run</strong> only log what they would do; turn dry-run off per rule to act for real.</p>
        {#if enabled}
          <p class="flex items-center gap-1.5 text-xs text-warn"><TriangleAlert size={13} /> Live rules can send messages on your behalf (capped at {app.settings?.automation_max_per_hour ?? 0}/hour).</p>
        {/if}
      </div>
      <Toggle checked={enabled} label="Enable automations" hideLabel onchange={(v) => (v ? (confirmEnable = true) : void setEnabled(false))} />
    </section>

    <section aria-labelledby="rules-h" class="space-y-2">
      <div class="flex items-center justify-between">
        <h2 id="rules-h" class="section-title">Rules</h2>
        <button class="btn btn-sm btn-primary" onclick={() => (editing = newRule())} data-testid="new-rule"><Plus size={14} /> New rule</button>
      </div>
      {#if loading}
        <div class="card"><Skeleton rows={3} /></div>
      {:else if error}
        <div class="card"><ErrorPanel {error} reset={load} /></div>
      {:else if rules.length === 0}
        <div class="card"><EmptyState icon={Bot} title="No rules yet" message="Create a rule to auto-reply, download or forward." /></div>
      {:else}
        <ul class="card divide-y divide-border">
          {#each rules as r (r.id)}
            <li class="flex items-center gap-3 px-4 py-3" data-testid="rule-row">
              <Toggle checked={r.enabled} label="Enable {r.name}" hideLabel onchange={(v) => toggleRule(r, v)} />
              <div class="min-w-0 flex-1">
                <p class="flex items-center gap-2 truncate text-[13px] font-medium">{r.name}{#if r.dry_run}<span class="badge badge-warn">dry-run</span>{/if}</p>
                <p class="text-xs text-muted">
                  Fired {r.fired_count}× · last {timeAgo(r.last_fired_at)} · {r.actions.map((a) => a.type.replace("_", " ")).join(", ")}
                </p>
              </div>
              <button class="btn btn-sm" onclick={() => (editing = r)}><Pencil size={13} /> Edit</button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section aria-labelledby="act-log" class="space-y-2">
      <h2 id="act-log" class="section-title">Activity</h2>
      {#if log.length === 0}
        <div class="card"><EmptyState icon={Zap} title="Nothing has fired yet" /></div>
      {:else}
        <ul class="card divide-y divide-border" data-testid="automation-log">
          {#each log as e, i (i)}
            <li class="flex items-start gap-3 px-4 py-2.5 text-[13px]">
              <Zap size={14} class="mt-0.5 shrink-0 {e.dry_run ? 'text-warn' : 'text-accent'}" />
              <div class="min-w-0 flex-1">
                <p><span class="font-medium">{e.rule_name}</span> {#if e.dry_run}<span class="badge badge-warn">dry-run</span>{/if}</p>
                <p class="text-xs text-muted">{e.outcomes.join(" · ")}</p>
              </div>
              <button class="shrink-0 text-xs text-muted hover:text-fg hover:underline" title={dateTime(e.at)} onclick={() => app.jumpTo(e.chat_jid, e.message_id)}>{timeAgo(e.at)}</button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
</div>

{#if editing}
  {@const r = editing}
  <Modal title={r.id === null ? "New rule" : `Edit “${r.name}”`} onclose={() => (editing = null)} width="52rem">
    <RuleEditor rule={r} onsave={save} oncancel={() => (editing = null)} ondelete={r.id === null ? undefined : () => (confirmDelete = r)} />
  </Modal>
{/if}

{#if confirmEnable}
  <Modal title="Enable automations?" onclose={() => (confirmEnable = false)}>
    <div class="space-y-2 text-[13px]">
      <p>Enabled rules will run on every <strong>new</strong> incoming message from now on.</p>
      <p class="text-muted">Rules not in dry-run can reply, forward and call webhooks automatically. Test your rules first.</p>
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (confirmEnable = false)}>Cancel</button>
      <button class="btn btn-primary" data-autofocus onclick={() => setEnabled(true)} data-testid="confirm-enable">Enable</button>
    {/snippet}
  </Modal>
{/if}

{#if confirmDelete}
  {@const r = confirmDelete}
  <Modal title="Delete rule?" onclose={() => (confirmDelete = null)}>
    <p class="text-[13px]">“{r.name}” will be removed. This can't be undone.</p>
    {#snippet footer()}
      <button class="btn" onclick={() => (confirmDelete = null)}>Cancel</button>
      <button class="btn btn-danger" onclick={() => remove(r)}>Delete</button>
    {/snippet}
  </Modal>
{/if}
