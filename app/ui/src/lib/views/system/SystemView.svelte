<script lang="ts">
  import BridgeCard from "./BridgeCard.svelte";
  import OverviewCard from "./OverviewCard.svelte";
  import JobsPanel from "./JobsPanel.svelte";
  import LogsViewer from "./LogsViewer.svelte";
  import IssuesPanel from "./IssuesPanel.svelte";
  import { app } from "../../state/app.svelte";

  let tab = $state<"jobs" | "logs" | "issues">("jobs");
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-6xl space-y-5 p-6">
    <div class="grid gap-5 lg:grid-cols-2">
      <BridgeCard />
      <OverviewCard />
    </div>
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
