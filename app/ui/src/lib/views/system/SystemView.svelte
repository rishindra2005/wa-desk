<script lang="ts">
  import BridgeCard from "./BridgeCard.svelte";
  import OverviewCard from "./OverviewCard.svelte";
  import McpLine from "./McpLine.svelte";
  import JobsPanel from "./JobsPanel.svelte";
  import LogsViewer from "./LogsViewer.svelte";
  import IssuesPanel from "./IssuesPanel.svelte";
  import { app } from "../../state/app.svelte";

  import { RefreshCw } from "@lucide/svelte";
  import Spinner from "../../components/Spinner.svelte";

  let tab = $state<"jobs" | "logs" | "issues">("jobs");
  let refreshing = $state(false);
  let fixed = $state<number | null>(null);
  async function refreshNames() {
    refreshing = true;
    fixed = await app.refreshNames();
    refreshing = false;
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-6xl space-y-5 p-6">
    <div class="grid gap-5 lg:grid-cols-2">
      <BridgeCard />
      <OverviewCard />
    </div>
    <McpLine />
    <section class="card flex items-center gap-4 p-4" aria-labelledby="names-title">
      <div class="min-w-0 flex-1">
        <h2 id="names-title" class="text-[15px] font-semibold">Chat names</h2>
        <p class="text-xs text-muted">Replace placeholder group, community and channel names with the real ones from WhatsApp.</p>
        {#if fixed !== null}<p class="mt-1 text-xs text-accent-text" data-testid="refresh-result" role="status">{fixed === 0 ? "Everything was already up to date." : `Fixed ${fixed} name${fixed === 1 ? "" : "s"}.`}</p>{/if}
      </div>
      <button class="btn" onclick={refreshNames} disabled={refreshing} data-testid="system-refresh-names">
        {#if refreshing}<Spinner size={14} label="Refreshing" />{:else}<RefreshCw size={14} />{/if} Refresh names
      </button>
    </section>
    <div class="tabbar" role="tablist" aria-label="System sections">
      <button role="tab" aria-selected={tab === "jobs"} onclick={() => (tab = "jobs")}>Jobs {#if app.runningJobs.length}<span class="badge badge-info ml-1">{app.runningJobs.length}</span>{/if}</button>
      <button role="tab" aria-selected={tab === "logs"} onclick={() => (tab = "logs")}>Logs</button>
      <button role="tab" aria-selected={tab === "issues"} onclick={() => (tab = "issues")}>Issues {#if app.openIssues}<span class="badge badge-danger ml-1">{app.openIssues}</span>{/if}</button>
    </div>
    <div role="tabpanel">
      {#if tab === "jobs"}<JobsPanel />{:else if tab === "logs"}<LogsViewer />{:else}<IssuesPanel />{/if}
    </div>
  </div>
</div>
