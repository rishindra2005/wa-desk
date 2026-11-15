<script lang="ts">
  import { CircleCheck, PackageOpen, FolderOpen, ExternalLink, Trash2, ChevronRight, ShieldCheck } from "@lucide/svelte";
  import * as api from "../../api";
  import type { Issue } from "../../bindings/Issue";
  import EmptyState from "../../components/EmptyState.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import Modal from "../../components/Modal.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import { app } from "../../state/app.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { dateTime, timeAgo } from "../../utils/format";
  import { openLocalPath, revealPath } from "../../platform";

  let issues = $state<Issue[]>([]);
  let tab = $state<"open" | "resolved">("open");
  let error = $state<unknown>(null);
  let expanded = $state<number | null>(null);
  let exporting = $state(false);
  let exported = $state<string | null>(null);

  const shown = $derived(issues.filter((i) => i.status === tab));
  const openCount = $derived(issues.filter((i) => i.status === "open").length);
  const resolvedCount = $derived(issues.length - openCount);

  async function load() {
    try {
      issues = await api.listIssues();
      error = null;
      app.openIssues = issues.filter((i) => i.status === "open").length;
    } catch (e) {
      error = e;
    }
  }

  $effect(() => {
    void load();
    return api.subscribe("issue:new", () => void load());
  });

  async function resolve(i: Issue) {
    try {
      await api.resolveIssue(i.id);
      await load();
      toasts.success("Issue resolved");
    } catch (e) {
      toasts.error(e);
    }
  }

  async function clearResolved() {
    try {
      const n = await api.clearResolvedIssues();
      await load();
      toasts.success(`Cleared ${n} resolved issue${n === 1 ? "" : "s"}`);
    } catch (e) {
      toasts.error(e);
    }
  }

  async function exportReport() {
    exporting = true;
    try {
      exported = await api.exportIssueBundle(null);
    } catch (e) {
      toasts.error(e, "Export failed");
    } finally {
      exporting = false;
    }
  }
</script>

<div class="space-y-3" data-testid="issues">
  <div class="flex flex-wrap items-center gap-2">
    <div class="tabbar" role="tablist" aria-label="Issue status">
      <button role="tab" aria-selected={tab === "open"} onclick={() => (tab = "open")}>Open <span class="badge ml-1">{openCount}</span></button>
      <button role="tab" aria-selected={tab === "resolved"} onclick={() => (tab = "resolved")}>Resolved <span class="badge ml-1">{resolvedCount}</span></button>
    </div>
    <div class="flex-1"></div>
    {#if tab === "resolved" && resolvedCount > 0}
      <button class="btn btn-sm" onclick={clearResolved} data-testid="clear-resolved"><Trash2 size={13} /> Clear resolved</button>
    {/if}
    <button class="btn btn-sm" onclick={exportReport} disabled={exporting} data-testid="export-report">
      {#if exporting}<Spinner size={12} />{:else}<PackageOpen size={13} />{/if} Export report
    </button>
  </div>

  {#if error}
    <div class="card"><ErrorPanel {error} reset={load} /></div>
  {:else if shown.length === 0}
    <div class="card"><EmptyState icon={ShieldCheck} title={tab === "open" ? "No open issues" : "Nothing resolved yet"} message={tab === "open" ? "Everything is running smoothly." : undefined} /></div>
  {:else}
    <ul class="card divide-y divide-border">
      {#each shown as i (i.id)}
        <li data-testid="issue-row">
          <div class="flex items-center gap-3 px-4 py-3">
            <button class="icon-btn h-6 w-6" aria-label="Toggle details" aria-expanded={expanded === i.id} onclick={() => (expanded = expanded === i.id ? null : i.id)}>
              <ChevronRight size={14} class="transition-transform {expanded === i.id ? 'rotate-90' : ''}" />
            </button>
            <div class="min-w-0 flex-1">
              <p class="truncate text-[13px] font-medium">{i.title}</p>
              <p class="text-xs text-muted">
                <span class="badge {i.kind === 'bridge_down' || i.kind === 'internal' ? 'badge-danger' : 'badge-warn'}">{i.kind}</span>
                {i.source} · {i.count}× · last {timeAgo(i.last_seen)}
              </p>
            </div>
            {#if i.status === "open"}
              <button class="btn btn-sm" onclick={() => resolve(i)} data-testid="resolve-issue"><CircleCheck size={13} /> Resolve</button>
            {/if}
          </div>
          {#if expanded === i.id}
            <div class="space-y-1 px-4 pb-3 pl-13 text-xs text-muted">
              <p>First seen {dateTime(i.first_seen)} · last seen {dateTime(i.last_seen)}</p>
              <p>Fingerprint <code class="font-mono">{i.fingerprint}</code></p>
              {#if i.last_detail}<pre class="whitespace-pre-wrap rounded-md bg-surface-2 p-2 font-mono text-[11px] text-fg">{i.last_detail}</pre>{/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

{#if exported}
  {@const path = exported}
  <Modal title="Report exported" onclose={() => (exported = null)}>
    <p class="text-[13px]">The report bundle (logs, issues and system info — no message content or tokens) was saved to:</p>
    <p class="mt-2 break-all rounded-md bg-surface-2 p-2 font-mono text-xs" data-testid="export-path">{path}</p>
    {#snippet footer()}
      <button class="btn" onclick={() => revealPath(path).catch((e) => toasts.error(e))}><FolderOpen size={14} /> Show in folder</button>
      <button class="btn" onclick={() => openLocalPath(path).catch((e) => toasts.error(e))}><ExternalLink size={14} /> Open</button>
      <button class="btn btn-primary" onclick={() => (exported = null)}>Done</button>
    {/snippet}
  </Modal>
{/if}
