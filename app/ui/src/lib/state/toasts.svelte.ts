// Toast notifications.

import { friendly, type ViewId } from "../errors";
import { toAppError } from "../api";

export type ToastKind = "success" | "error" | "info" | "warn";

export interface Toast {
  id: number;
  kind: ToastKind;
  title: string;
  message?: string;
  action?: { label: string; run: () => void };
  timeout: number;
}

let seq = 0;

class ToastStore {
  items = $state<Toast[]>([]);
  /** Called when a toast action wants to navigate; wired by the app shell. */
  navigate: (view: ViewId) => void = () => {};

  push(t: Omit<Toast, "id" | "timeout"> & { timeout?: number }): number {
    const id = ++seq;
    const toast: Toast = { timeout: t.kind === "error" ? 7000 : 3500, ...t, id };
    this.items = [...this.items.slice(-4), toast];
    if (toast.timeout > 0) setTimeout(() => this.dismiss(id), toast.timeout);
    return id;
  }

  dismiss(id: number): void {
    this.items = this.items.filter((t) => t.id !== id);
  }

  success(title: string, message?: string): number {
    return this.push(message === undefined ? { kind: "success", title } : { kind: "success", title, message });
  }

  info(title: string, message?: string): number {
    return this.push(message === undefined ? { kind: "info", title } : { kind: "info", title, message });
  }

  /** Show any thrown value as a friendly error toast. Cancelled errors are ignored. */
  error(e: unknown, context?: string): number | null {
    const err = toAppError(e);
    if (err.kind === "cancelled") return null;
    const f = friendly(err);
    const message = [context, f.message].filter(Boolean).join(": ");
    const action = f.action;
    return this.push({
      kind: "error",
      title: f.title,
      ...(message ? { message } : {}),
      ...(action ? { action: { label: action.label, run: () => this.navigate(action.view) } } : {}),
    });
  }
}

export const toasts = new ToastStore();
