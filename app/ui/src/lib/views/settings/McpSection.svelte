<script lang="ts">
  import { Copy } from "@lucide/svelte";
  import * as api from "../../api";
  import type { McpStatus } from "../../bindings/McpStatus";
  import type { Settings } from "../../bindings/Settings";
  import Toggle from "../../components/Toggle.svelte";
  import { app } from "../../state/app.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { copyText } from "../../utils/misc";
  import { MCP_PORT_MAX, MCP_PORT_MIN, mcpSummary, mcpUrl, validMcpPort } from "./mcp";

  interface Props {
    /** The form's draft: the MCP fields follow what was applied. */
    draft: Settings;
  }
  let { draft = $bindable() }: Props = $props();

  let status = $state<McpStatus | null>(null);
  let busy = $state(false);
  let portText = $state(String(draft.mcp_port));

  const portOk = $derived(validMcpPort(Number(portText)) && portText.trim() !== "");
  const url = $derived(mcpUrl(app.settings?.mcp_port ?? draft.mcp_port));
  const view = $derived(mcpSummary(status));

  async function refresh() {
    try {
      status = await api.mcpStatus();
    } catch {
      /* the line just stays as it was */
    }
  }

  $effect(() => {
    let alive = true;
    const load = () => api.mcpStatus().then((s) => { if (alive) status = s; }).catch(() => {});
    void load();
    const t = setInterval(load, 2000);
    return () => { alive = false; clearInterval(t); };
  });

  /** Save only the MCP fields (on top of what is saved) so the server follows at once. */
  async function apply(patch: Partial<Pick<Settings, "mcp_enabled" | "mcp_port">>) {
    if (!app.settings || busy) return;
    busy = true;
    try {
      const saved = await api.setSettings({ ...($state.snapshot(app.settings) as Settings), ...patch });
      app.settings = saved;
      draft.mcp_enabled = saved.mcp_enabled;
      draft.mcp_port = saved.mcp_port;
      portText = String(saved.mcp_port);
      await refresh();
    } catch (e) {
      portText = String(app.settings.mcp_port);
      toasts.error(e, "Couldn't change the MCP server");
    } finally {
      busy = false;
    }
  }

  function applyPort() {
    if (!portOk || Number(portText) === app.settings?.mcp_port) return;
    void apply({ mcp_port: Number(portText) });
  }

  async function copy() {
    if (await copyText(url)) toasts.success("MCP URL copied");
    else toasts.error(new Error("The clipboard is not available"), "Couldn't copy");
  }
</script>

<section class="card space-y-3 p-5" aria-labelledby="s-mcp" data-testid="mcp-section">
  <h2 id="s-mcp" class="text-[15px] font-semibold">MCP server</h2>
  <p class="max-w-prose text-xs text-muted" data-testid="mcp-help">
    It lets AI assistants that support the Model Context Protocol read and send your WhatsApp messages. It only listens on this computer; turn it off if you don't use it.
  </p>
  <Toggle
    checked={draft.mcp_enabled}
    label="MCP server"
    description="Changes apply immediately."
    disabled={busy}
    onchange={(v) => void apply({ mcp_enabled: v })}
  />
  <div class="w-60">
    <label class="label" for="s-mcp-port">Port</label>
    <input
      id="s-mcp-port"
      type="number"
      min={MCP_PORT_MIN}
      max={MCP_PORT_MAX}
      class="input"
      value={portText}
      oninput={(e) => (portText = e.currentTarget.value)}
      disabled={busy}
      aria-invalid={portOk ? undefined : "true"}
      onchange={applyPort}
      onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); applyPort(); } }}
      data-testid="mcp-port"
    />
    {#if !portOk}<p class="mt-1 text-xs text-danger" role="alert">Enter a port from {MCP_PORT_MIN} to {MCP_PORT_MAX}</p>{/if}
  </div>
  <div>
    <span class="label">URL for your MCP client</span>
    <div class="flex items-center gap-2">
      <code class="min-w-0 flex-1 truncate rounded-md bg-surface-2 px-2 py-1 font-mono text-xs" data-testid="mcp-url">{url}</code>
      <button type="button" class="btn btn-sm" onclick={copy} data-testid="mcp-copy"><Copy size={13} /> Copy</button>
    </div>
  </div>
  {#if view}
    <p class="text-[13px] {view.bad ? 'text-danger' : 'text-muted'}" data-testid="mcp-status" data-state={status?.state}>
      {view.label}{#if view.detail}<span class="block font-mono text-[11px]">{view.detail}</span>{/if}
    </p>
  {/if}
</section>
