import type { McpStatus } from "../../bindings/McpStatus";

/** Lowest port wa-desk lets the MCP server use (no privileged ports). */
export const MCP_PORT_MIN = 1024;
/** Highest TCP port. */
export const MCP_PORT_MAX = 65535;

/** URL an MCP client connects to; the server only listens on this machine. */
export function mcpUrl(port: number): string {
  return `http://127.0.0.1:${port}/mcp`;
}

/** True for a whole number in the allowed port range. */
export function validMcpPort(port: unknown): port is number {
  return typeof port === "number" && Number.isInteger(port) && port >= MCP_PORT_MIN && port <= MCP_PORT_MAX;
}

/** One line for the Settings section and the System view. `bad` marks problems. */
export function mcpSummary(s: McpStatus | null | undefined): { label: string; detail: string | null; bad: boolean } | null {
  if (!s) return null;
  switch (s.state) {
    case "disabled": return { label: "Off", detail: null, bad: false };
    case "starting": return { label: s.detail ?? "Starting", detail: null, bad: false };
    case "running": return { label: s.pid ? `Running (pid ${s.pid})` : "Running", detail: s.restarts > 0 ? `Restarted ${s.restarts}×` : null, bad: false };
    case "crashed": return { label: "Stopped unexpectedly; restarting", detail: s.detail, bad: true };
    case "gave_up": return { label: "Keeps crashing; not restarting", detail: s.detail, bad: true };
    case "needs_uv": return { label: "Needs uv", detail: s.detail ?? "Install uv from https://docs.astral.sh/uv/ to run the MCP server.", bad: true };
    case "not_installed": return { label: "Not included in this install", detail: s.detail, bad: false };
    case "port_busy": return { label: "Port in use by another program", detail: s.detail, bad: true };
    case "external": return { label: "Using an MCP server that is already running", detail: s.detail, bad: false };
    case "error": return { label: "Cannot start", detail: s.detail, bad: true };
    case "stopped": return { label: "Stopped", detail: null, bad: false };
    default: return null;
  }
}
