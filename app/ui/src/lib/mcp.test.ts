import { describe, expect, it } from "vitest";
import { MCP_PORT_MAX, MCP_PORT_MIN, mcpSummary, mcpUrl, validMcpPort } from "./views/settings/mcp";
import type { McpStatus } from "./bindings/McpStatus";

const st = (p: Partial<McpStatus>): McpStatus => ({ state: "running", pid: null, url: mcpUrl(8771), detail: null, restarts: 0, ...p });

describe("mcpUrl", () => {
  it("is the loopback streamable-HTTP endpoint", () => {
    expect(mcpUrl(8771)).toBe("http://127.0.0.1:8771/mcp");
    expect(mcpUrl(9000)).toBe("http://127.0.0.1:9000/mcp");
  });
});

describe("validMcpPort", () => {
  it("accepts whole numbers from 1024 to 65535 only", () => {
    expect(validMcpPort(MCP_PORT_MIN)).toBe(true);
    expect(validMcpPort(MCP_PORT_MAX)).toBe(true);
    expect(validMcpPort(8771)).toBe(true);
    for (const bad of [0, 80, 1023, 65536, 1.5, NaN, -1, "8771", null, undefined]) expect(validMcpPort(bad)).toBe(false);
  });
});

describe("mcpSummary", () => {
  it("is null without a status", () => {
    expect(mcpSummary(null)).toBeNull();
  });
  it("shows the pid and restart count when running", () => {
    expect(mcpSummary(st({ pid: 55 }))?.label).toBe("Running (pid 55)");
    expect(mcpSummary(st({ pid: 55, restarts: 3 }))?.detail).toBe("Restarted 3×");
  });
  it("shows the first-run hint while starting", () => {
    const r = mcpSummary(st({ state: "starting", detail: "Starting (first run installs dependencies)" }));
    expect(r?.label).toContain("first run");
    expect(r?.bad).toBe(false);
  });
  it("points at uv when it is missing", () => {
    const r = mcpSummary(st({ state: "needs_uv", detail: null }));
    expect(r?.bad).toBe(true);
    expect(r?.detail).toContain("https://docs.astral.sh/uv/");
  });
  it("marks crash, give-up, port-busy and error as bad and keeps the detail", () => {
    for (const state of ["crashed", "gave_up", "port_busy", "error"] as const) {
      const r = mcpSummary(st({ state, detail: "boom" }));
      expect(r?.bad).toBe(true);
      expect(r?.detail).toBe("boom");
    }
  });
  it("does not flag off, external, not installed or stopped", () => {
    for (const state of ["disabled", "external", "not_installed", "stopped"] as const) {
      expect(mcpSummary(st({ state }))?.bad).toBe(false);
    }
  });
});
