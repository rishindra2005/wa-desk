import type { BridgeState } from "../../bindings/BridgeState";

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
