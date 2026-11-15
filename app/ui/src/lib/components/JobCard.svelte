<script lang="ts">
  import { CircleX, FolderOpen, ExternalLink, FileOutput } from "@lucide/svelte";
  import * as api from "../api";
  import type { JobRecord } from "../bindings/JobRecord";
  import ProgressBar from "./ProgressBar.svelte";
  import { formatDuration, timeAgo, pct } from "../utils/format";
  import { baseName } from "../utils/text";
  import { openLocalPath, outputPath, revealPath } from "../platform";
  import { toasts } from "../state/toasts.svelte";

  let { job, compact = false }: { job: JobRecord; compact?: boolean } = $props();
  let cancelling = $state(false);

  const active = $derived(job.status === "running" || job.status === "queued");
  const badge = $derived(
    {
      queued: "badge",
      running: "badge badge-info",
      succeeded: "badge badge-accent",
      failed: "badge badge-danger",
      cancelled: "badge badge-warn",
    }[job.status],
  );
  const duration = $derived(job.finished_at ? job.finished_at - job.created_at : null);

  async function cancel() {
    cancelling = true;
    try {
      await api.cancelJob(job.id);
    } catch (e) {
      toasts.error(e, "Couldn't cancel");
    } finally {
      cancelling = false;
    }
  }

  async function act(fn: (p: string) => Promise<void>, url: string) {
    const p = outputPath(url);
    if (!p) return toasts.push({ kind: "warn", title: "Unknown output location" });
    try {
      await fn(p);
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<div class="card p-3" data-testid="job-card" data-status={job.status}>
  <div class="flex items-center gap-2">
    <p class="min-w-0 flex-1 truncate text-[13px] font-medium">{job.label}</p>
    <span class={badge} data-testid="job-status">{job.status}</span>
    {#if active}
      <button class="btn btn-sm btn-danger" onclick={cancel} disabled={cancelling} data-testid="job-cancel"><CircleX size={13} /> Cancel</button>
    {/if}
  </div>
  {#if active}
    <div class="mt-2"><ProgressBar done={job.done} total={job.total} label="{job.label} progress" /></div>
    <p class="mt-1 flex justify-between text-[11px] text-muted">
      <span class="capitalize">{job.phase}{job.message ? ` · ${job.message}` : ""}</span>
      {#if job.total > 0}<span class="tabular-nums">{Math.round(pct(job.done, job.total))}%</span>{/if}
    </p>
  {:else if !compact}
    <p class="mt-1 text-[11px] text-muted">
      {[timeAgo(job.finished_at ?? job.created_at), duration !== null ? `took ${formatDuration(duration)}` : null, job.message].filter(Boolean).join(" · ")}
    </p>
  {/if}
  {#if job.error}<p class="mt-1.5 rounded-md bg-danger-soft px-2 py-1 font-mono text-[11px] text-danger">{job.error}</p>{/if}
  {#if job.outputs.length && !compact}
    <ul class="mt-2 space-y-1" aria-label="Outputs">
      {#each job.outputs as url (url)}
        {@const p = outputPath(url)}
        <li class="flex items-center gap-2 rounded-md bg-surface-2 px-2 py-1 text-xs" data-testid="job-output">
          <FileOutput size={13} class="shrink-0 text-muted" />
          <span class="min-w-0 flex-1 truncate" title={p ?? url}>{p ? baseName(p) : url}</span>
          <button class="btn btn-sm btn-ghost" onclick={() => act(openLocalPath, url)}><ExternalLink size={12} /> Open</button>
          <button class="btn btn-sm btn-ghost" onclick={() => act(revealPath, url)}><FolderOpen size={12} /> Reveal</button>
        </li>
      {/each}
    </ul>
  {/if}
</div>
