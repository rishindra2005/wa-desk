import type { BridgeState } from "../../bindings/BridgeState";
import type { SupervisorStatus } from "../../bindings/SupervisorStatus";

/** Label and colour for a bridge state (used by the pill and the System card). */
export function bridgeTone(state: BridgeState | undefined | null): { label: string; color: string } {
  switch (state) {
    case "connected": return { label: "Connected", color: "var(--accent)" };
    case "connecting": return { label: "Connecting", color: "var(--info)" };
    case "pairing": return { label: "Pairing", color: "var(--info)" };
    case "disconnected": return { label: "Disconnected", color: "var(--warn)" };
    case "logged_out": return { label: "Logged out", color: "var(--danger)" };
    case "outdated": return { label: "Outdated", color: "var(--warn)" };
    case "error": return { label: "Error", color: "var(--danger)" };
    case "unreachable": return { label: "Offline", color: "var(--danger)" };
    default: return { label: "Unknown", color: "var(--text-faint)" };
  }
}

/** One line for the System card describing who runs the bridge. */
export function supervisorSummary(s: SupervisorStatus | null | undefined): { label: string; detail: string | null; bad: boolean } | null {
  if (!s) return null;
  switch (s.state) {
    case "starting": return { label: "Checking for a bridge…", detail: null, bad: false };
    case "external": return { label: "Using a bridge you run yourself", detail: s.detail, bad: false };
    case "disabled": return { label: "Bundled bridge off (bridge mode: external)", detail: s.detail, bad: false };
    case "not_installed": return { label: "No bundled bridge in this install", detail: "Start the bridge yourself, or install a release build.", bad: false };
    case "running": return { label: s.pid ? `Bundled bridge running (pid ${s.pid})` : "Bundled bridge running", detail: s.restarts > 0 ? `Restarted ${s.restarts}×` : null, bad: false };
    case "crashed": return { label: "Bundled bridge stopped; restarting", detail: s.detail, bad: true };
    case "gave_up": return { label: "Bundled bridge keeps crashing; not restarting", detail: s.detail, bad: true };
    case "error": return { label: "Bundled bridge cannot start", detail: s.detail, bad: true };
    case "stopped": return { label: "Bundled bridge stopped", detail: null, bad: false };
    default: return null;
  }
}
