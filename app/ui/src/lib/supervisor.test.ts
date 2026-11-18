import { describe, expect, it } from "vitest";
import { supervisorSummary } from "./views/system/bridge";
import type { SupervisorStatus } from "./bindings/SupervisorStatus";

const st = (p: Partial<SupervisorStatus>): SupervisorStatus => ({ state: "running", pid: null, detail: null, restarts: 0, ...p });

describe("supervisorSummary", () => {
  it("is null without a status", () => {
    expect(supervisorSummary(null)).toBeNull();
  });
  it("shows the pid when running and the restart count", () => {
    expect(supervisorSummary(st({ pid: 321 }))?.label).toContain("pid 321");
    expect(supervisorSummary(st({ pid: 321, restarts: 2 }))?.detail).toBe("Restarted 2×");
  });
  it("marks crash, give-up and error states as bad and keeps the detail", () => {
    for (const state of ["crashed", "gave_up", "error"] as const) {
      const r = supervisorSummary(st({ state, detail: "boom" }));
      expect(r?.bad).toBe(true);
      expect(r?.detail).toBe("boom");
    }
  });
  it("treats external, disabled and not-installed as fine", () => {
    for (const state of ["external", "disabled", "not_installed", "starting", "stopped"] as const) {
      expect(supervisorSummary(st({ state }))?.bad).toBe(false);
    }
    expect(supervisorSummary(st({ state: "not_installed" }))?.label).toMatch(/No bundled bridge/);
  });
});
