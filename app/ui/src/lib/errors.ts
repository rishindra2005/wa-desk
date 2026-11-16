// Map AppError kinds to friendly, actionable copy for toasts and inline errors.

import type { AppError } from "./bindings/AppError";
import type { ErrorKind } from "./bindings/ErrorKind";
import { toAppError } from "./api";

export type ViewId = "chats" | "calls" | "search" | "media" | "docs" | "tools" | "automations" | "system" | "settings";

export interface FriendlyError {
  title: string;
  message: string;
  /** Suggested navigation, e.g. to the System view for bridge problems. */
  action?: { label: string; view: ViewId };
}

const TITLES: Record<ErrorKind, string> = {
  not_found: "Not found",
  bridge_down: "WhatsApp bridge is offline",
  auth: "WhatsApp needs to be re-linked",
  forbidden: "Not allowed",
  busy: "Busy — try again shortly",
  timeout: "That took too long",
  invalid: "Check your input",
  external: "An external tool failed",
  cancelled: "Cancelled",
  internal: "Something went wrong",
};

/** Turn any thrown value into toast-ready copy. */
export function friendly(e: unknown): FriendlyError {
  const err: AppError = toAppError(e);
  const title = TITLES[err.kind];
  const message = err.message && err.message !== err.kind ? err.message : "";
  switch (err.kind) {
    case "bridge_down":
      return { title, message: message || "Messages can't be sent or downloaded right now.", action: { label: "Open System", view: "system" } };
    case "auth":
      return { title, message: message || "Scan the pairing QR in System.", action: { label: "Open System", view: "system" } };
    case "external":
      return { title, message, action: { label: "View logs", view: "system" } };
    case "internal":
      return { title, message, action: { label: "View issues", view: "system" } };
    default:
      return { title, message };
  }
}
