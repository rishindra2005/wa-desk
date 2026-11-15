<script lang="ts">
  import { Copy, Pause, Play, RefreshCw } from "@lucide/svelte";
  import * as api from "../../api";
  import type { LogLevel } from "../../bindings/LogLevel";
  import type { LogLine } from "../../bindings/LogLine";
  import VirtualList from "../../components/VirtualList.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import ErrorPanel from "../../components/ErrorPanel.svelte";
  import { copyText, debounce } from "../../utils/misc";
  import { toasts } from "../../state/toasts.svelte";

  let level = $state<LogLevel>("info");
  let contains = $state("");
  let target = $state("");
  let tail = $state(true);
  let lines = $state<LogLine[]>([]);
  let error = $state<unknown>(null);
  let loaded = $state(false);

  const LEVELS: LogLevel[] = ["trace", "debug", "info", "warn", "error"];
  const COLORS: Record<LogLevel, string> = {
    trace: "text-faint",
    debug: "text-muted",
    info: "text-info",
    warn: "text-warn",
    error: "text-danger",
  };

  async function load() {
    try {
      lines = await api.queryLogs({ level, contains: contains.trim() || null, target: target.trim() || null, since: null, limit: 1000 });
      error = null;
    } catch (e) {
      error = e;
    } finally {
      loaded = true;
    }
  }
  const reload = debounce(() => void load(), 200);

  $effect(() => {
    void level;
    void contains;
    void target;
    reload();
    return () => reload.cancel();
  });

  $effect(() => {
    if (!tail) return;
    const t = setInterval(() => void load(), 2000);
    return () => clearInterval(t);
  });

  function fmtTs(ts: number) {
    const d = new Date(ts * 1000);
    return d.toTimeString().slice(0, 8);
  }
  function lineText(l: LogLine) {
    return `${new Date(l.ts * 1000).toISOString()} ${l.level.toUpperCase()} ${l.target}: ${l.message}${l.fields ? ` ${l.fields}` : ""}`;
  }
  async function copy(l: LogLine) {
    if (await copyText(lineText(l))) toasts.success("Copied log line");
  }
</script>

<div class="card flex h-[60vh] min-h-80 flex-col overflow-hidden" data-testid="logs">
  <div class="flex flex-wrap items-center gap-2 border-b border-border p-2">
    <select class="input w-28" aria-label="Minimum level" bind:value={level} data-testid="log-level">
      {#each LEVELS as l (l)}<option value={l}>{l}+</option>{/each}
    </select>
    <input class="input w-56" placeholder="Filter text" aria-label="Filter log text" bind:value={contains} data-testid="log-filter" />
    <input class="input w-44" placeholder="Target (e.g. wa_bridge)" aria-label="Log target" bind:value={target} />
    <div class="flex-1"></div>
    <span class="text-xs text-muted">{lines.length} lines</span>
    <button class="btn btn-sm" aria-pressed={tail} onclick={() => (tail = !tail)} data-testid="log-tail">
      {#if tail}<Pause size={13} /> Live{:else}<Play size={13} /> Paused{/if}
    </button>
    <button class="icon-btn h-7 w-7" aria-label="Refresh logs" onclick={load}><RefreshCw size={14} /></button>
  </div>
  <div class="min-h-0 flex-1 font-mono text-[12px]">
    {#if error}
      <ErrorPanel {error} reset={load} />
    {:else if loaded && lines.length === 0}
      <EmptyState title="No log lines match" />
    {:else}
      <VirtualList items={lines} getKey={(l, i) => `${l.ts}:${i}:${l.message}`} estimate={24} class="h-full" ariaLabel="Log lines">
        {#snippet row(l: LogLine)}
          <div class="group flex items-start gap-2 px-3 py-[3px] hover:bg-surface-2" data-testid="log-line">
            <span class="shrink-0 text-faint">{fmtTs(l.ts)}</span>
            <span class="w-11 shrink-0 font-semibold uppercase {COLORS[l.level]}">{l.level}</span>
            <span class="w-36 shrink-0 truncate text-muted" title={l.target}>{l.target}</span>
            <span class="min-w-0 flex-1 break-words">{l.message}{#if l.fields}<span class="ml-2 text-faint">{l.fields}</span>{/if}</span>
            <button class="opacity-0 group-hover:opacity-100 focus:opacity-100" aria-label="Copy line" onclick={() => copy(l)}><Copy size={12} /></button>
          </div>
        {/snippet}
      </VirtualList>
    {/if}
  </div>
</div>
