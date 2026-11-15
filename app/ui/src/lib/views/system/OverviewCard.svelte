<script lang="ts">
  import { app } from "../../state/app.svelte";
  import ProgressBar from "../../components/ProgressBar.svelte";
  import { formatCount, shortDate, pct } from "../../utils/format";

  $effect(() => {
    void app.refreshOverview();
    const t = setInterval(() => void app.refreshOverview(), 5000);
    return () => clearInterval(t);
  });
  const o = $derived(app.overview);
</script>

<section class="card p-5" aria-labelledby="ov-h" data-testid="overview">
  <h2 id="ov-h" class="text-[15px] font-semibold">Archive</h2>
  {#if o}
    <dl class="mt-3 grid grid-cols-2 gap-x-6 gap-y-3 sm:grid-cols-4">
      <div><dt class="text-xs text-muted">Chats</dt><dd class="text-xl font-semibold tabular-nums">{formatCount(o.chats)}</dd></div>
      <div><dt class="text-xs text-muted">Messages</dt><dd class="text-xl font-semibold tabular-nums">{formatCount(o.messages)}</dd></div>
      <div><dt class="text-xs text-muted">Oldest</dt><dd class="text-[13px] font-medium">{o.oldest_ts ? shortDate(o.oldest_ts, Date.now(), true) : "—"}</dd></div>
      <div><dt class="text-xs text-muted">Newest</dt><dd class="text-[13px] font-medium">{o.newest_ts ? shortDate(o.newest_ts, Date.now(), true) : "—"}</dd></div>
    </dl>
    <div class="mt-4 space-y-3">
      <div>
        <p class="mb-1 flex justify-between text-xs"><span class="text-muted">Media downloaded</span><span class="tabular-nums">{formatCount(o.media_local)} / {formatCount(o.media_total)} ({Math.round(pct(o.media_local, o.media_total))}%)</span></p>
        <ProgressBar done={o.media_local} total={o.media_total} label="Media downloaded" />
      </div>
      <div>
        <p class="mb-1 flex justify-between text-xs">
          <span class="text-muted">Search index</span>
          <span class="tabular-nums">{o.indexed >= o.messages ? "Ready" : `${Math.round(pct(o.indexed, o.messages))}% built`}</span>
        </p>
        <ProgressBar done={o.indexed} total={o.messages} label="Search index" />
      </div>
    </div>
    <p class="mt-3 text-[11px] text-faint">wa-desk {o.app_version}</p>
  {:else}
    <div class="mt-3 space-y-2"><div class="skeleton h-6 w-2/3"></div><div class="skeleton h-3 w-full"></div><div class="skeleton h-3 w-full"></div></div>
  {/if}
</section>
