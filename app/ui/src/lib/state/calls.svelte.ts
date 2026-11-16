// Call state shared by the shell: the missed-call badge and incoming-call banners.

import * as api from "../api";
import type { CallRecord } from "../bindings/CallRecord";
import { friendly } from "../errors";
import { isMissedForMe, isRinging, missedSince } from "../utils/calls";
import { toAppError } from "../api";
import { toasts } from "./toasts.svelte";

const SEEN_KEY = "wa-desk.callsSeen";
/** A ringing banner never outlives this (the bridge may not report an end). */
export const RING_TTL_MS = 90_000;
/** First run: count missed calls from the last week. */
const FIRST_RUN_WINDOW = 7 * 86_400;

export interface RingingCall {
  call: CallRecord;
  busy: boolean;
  /** Why declining is unavailable (read-only, offline…); disables the button. */
  error: string | null;
}

class CallsState {
  /** Missed calls since the last visit to the Calls view. */
  missedCount = $state(0);
  ringing = $state<RingingCall[]>([]);
  /** Unix seconds of the last visit to the Calls view. */
  lastSeen = $state(0);
  /** True while the Calls view is open (new missed calls are then already "seen"). */
  viewing = false;

  private started = false;
  private timers = new Map<string, ReturnType<typeof setTimeout>>();
  private disposers: (() => void)[] = [];

  constructor() {
    this.lastSeen = this.loadSeen();
  }

  private loadSeen(): number {
    try {
      const v = Number(localStorage.getItem(SEEN_KEY));
      if (Number.isFinite(v) && v > 0) return v;
    } catch {
      /* storage unavailable */
    }
    return Math.floor(Date.now() / 1000) - FIRST_RUN_WINDOW;
  }

  /** Subscribe to `call:update` and load the badge. Idempotent. */
  start(): void {
    if (this.started) return;
    this.started = true;
    this.disposers.push(api.subscribe("call:update", (c) => this.onUpdate(c)));
    void this.refresh();
  }

  stop(): void {
    for (const d of this.disposers) d();
    this.disposers = [];
    for (const t of this.timers.values()) clearTimeout(t);
    this.timers.clear();
    this.started = false;
  }

  /** Recount missed calls and pick up calls that are ringing right now. */
  async refresh(): Promise<void> {
    try {
      const page = await api.listCalls({ chat_jid: null, missed_only: true, media: null, cursor: null, limit: 200 });
      this.missedCount = this.viewing ? 0 : missedSince(page.items, this.lastSeen);
    } catch {
      /* the Calls view shows errors */
    }
    try {
      const recent = await api.listCalls({ chat_jid: null, missed_only: false, media: null, cursor: null, limit: 10 });
      const fresh = Date.now() / 1000 - 120;
      for (const c of recent.items) if (isRinging(c) && c.ts > fresh) this.addRinging(c);
    } catch {
      /* ignore */
    }
  }

  /** Mark everything up to now as seen (Calls view opened or focused). */
  markSeen(): void {
    this.lastSeen = Math.floor(Date.now() / 1000);
    this.missedCount = 0;
    try {
      localStorage.setItem(SEEN_KEY, String(this.lastSeen));
    } catch {
      /* ignore */
    }
  }

  private onUpdate(c: CallRecord): void {
    if (isRinging(c)) this.addRinging(c);
    else this.removeRinging(c.call_id);
    if (isMissedForMe(c)) {
      if (this.viewing) this.markSeen();
      else void this.refresh();
    }
  }

  private addRinging(c: CallRecord): void {
    const i = this.ringing.findIndex((r) => r.call.call_id === c.call_id);
    if (i >= 0) {
      const cur = this.ringing[i] as RingingCall;
      this.ringing[i] = { ...cur, call: c };
      return;
    }
    this.ringing = [...this.ringing, { call: c, busy: false, error: null }];
    clearTimeout(this.timers.get(c.call_id));
    this.timers.set(c.call_id, setTimeout(() => this.removeRinging(c.call_id), RING_TTL_MS));
  }

  private removeRinging(callId: string): void {
    clearTimeout(this.timers.get(callId));
    this.timers.delete(callId);
    if (this.ringing.some((r) => r.call.call_id === callId)) this.ringing = this.ringing.filter((r) => r.call.call_id !== callId);
  }

  /** Hide a banner without declining (the call keeps ringing on the phone). */
  dismiss(callId: string): void {
    this.removeRinging(callId);
  }

  /** Decline a ringing call through the bridge. */
  async decline(callId: string): Promise<void> {
    const i = this.ringing.findIndex((r) => r.call.call_id === callId);
    const cur = this.ringing[i];
    if (!cur || cur.busy) return;
    this.ringing[i] = { ...cur, busy: true, error: null };
    try {
      await api.rejectCall(cur.call.call_id, cur.call.caller_jid);
      this.removeRinging(callId);
    } catch (e) {
      const err = toAppError(e);
      if (err.kind === "invalid") {
        // Answered, ended or declined elsewhere in the meantime.
        this.removeRinging(callId);
        toasts.info("That call is no longer ringing");
        return;
      }
      const f = friendly(err);
      const j = this.ringing.findIndex((r) => r.call.call_id === callId);
      const now = this.ringing[j];
      if (now) this.ringing[j] = { ...now, busy: false, error: err.kind === "forbidden" ? "Declining is unavailable in read-only mode" : f.message || f.title };
    }
  }
}

export const calls = new CallsState();
