<script lang="ts">
  import * as api from "../../api";
  import type { McpStatus } from "../../bindings/McpStatus";
  import { mcpSummary } from "../settings/mcp";

  let status = $state<McpStatus | null>(null);
  $effect(() => {
    let alive = true;
    const load = () => api.mcpStatus().then((s) => { if (alive) status = s; }).catch(() => {});
    void load();
    const t = setInterval(load, 3000);
    return () => { alive = false; clearInterval(t); };
  });
  const view = $derived(mcpSummary(status));
</script>

{#if view}
  <section class="card flex items-center gap-3 px-4 py-3 text-[13px]" aria-label="MCP server" data-testid="mcp-system-line" data-state={status?.state}>
    <span class="font-semibold">MCP server</span>
    <span class={view.bad ? "text-danger" : "text-muted"}>{view.label}</span>
    {#if status?.state === "running" || status?.state === "external"}<code class="ml-auto truncate font-mono text-xs text-faint">{status.url}</code>{/if}
    {#if view.detail}<span class="truncate font-mono text-[11px] text-faint" title={view.detail}>{view.detail}</span>{/if}
  </section>
{/if}
