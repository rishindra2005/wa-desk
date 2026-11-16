// Global application state (runes). One instance, created at startup.

import * as api from "../api";
import type { ConnectionState } from "../bindings/ConnectionState";
import type { JobRecord } from "../bindings/JobRecord";
import type { Overview } from "../bindings/Overview";
import type { Settings } from "../bindings/Settings";
import type { Theme } from "../bindings/Theme";
import type { ViewId } from "../errors";
import { toasts } from "./toasts.svelte";
import { calls } from "./calls.svelte";
import { debounce } from "../utils/misc";
import { notify } from "../platform";
import { resetAvatarCache } from "../avatar";

export const VIEWS: { id: ViewId; label: string }[] = [
  { id: "chats", label: "Chats" },
  { id: "calls", label: "Calls" },
  { id: "search", label: "Search" },
  { id: "media", label: "Media" },
  { id: "docs", label: "Docs" },
  { id: "tools", label: "Tools" },
  { id: "automations", label: "Automations" },
  { id: "system", label: "System" },
  { id: "settings", label: "Settings" },
];

export interface JumpTarget {
  chatJid: string;
  messageId: string;
  /** Changes on every request so repeated jumps to the same message re-trigger. */
  nonce: number;
}

const THEME_KEY = "wa-desk.theme";

class AppState {
  view = $state<ViewId>("chats");
  openChatJid = $state<string | null>(null);
  jump = $state<JumpTarget | null>(null);
  settings = $state<Settings | null>(null);
  conn = $state<ConnectionState | null>(null);
  overview = $state<Overview | null>(null);
  openIssues = $state(0);
  unreadChats = $state(0);
  jobs = $state<JobRecord[]>([]);
  paletteOpen = $state(false);
  /** Search text handed to the Search view (from the top bar). */
  searchText = $state("");
  searchNonce = $state(0);
  systemDark = $state(false);
  /** Media item handed to the Tools converter (from Docs / Media). */
  toolsInput = $state<{ chatJid: string; messageId: string; label: string } | null>(null);
  /** Profile / info drawer target (contact, group, community or channel JID). */
  profileJid = $state<string | null>(null);
  profileBack = $state<string[]>([]);
  /** Chat scoped Search view (from "Search in chat"). */
  searchChat = $state<string | null>(null);
  exportDialog = $state<{ chatJid: string; chatName: string; messageIds: string[] } | null>(null);
  forwardDialog = $state<{ chatJid: string; messageIds: string[] } | null>(null);
  /** Theme preview before settings load (persisted locally to avoid a flash). */
  localTheme = $state<Theme>("system");

  readonly theme = $derived<Theme>(this.settings?.theme ?? this.localTheme);
  readonly dark = $derived(this.theme === "dark" || (this.theme === "system" && this.systemDark));
  readonly runningJobs = $derived(this.jobs.filter((j) => j.status === "running" || j.status === "queued"));

  private started = false;
  private disposers: (() => void)[] = [];

  go(view: ViewId): void {
    this.view = view;
  }

  openChat(jid: string): void {
    this.openChatJid = jid;
    this.view = "chats";
  }

  jumpTo(chatJid: string, messageId: string): void {
    this.openChatJid = chatJid;
    this.view = "chats";
    this.jump = { chatJid, messageId, nonce: Date.now() + Math.random() };
  }

  /** Open the info drawer for a JID; `push` keeps the previous one for Back. */
  openProfile(jid: string, push = false): void {
    if (push && this.profileJid && this.profileJid !== jid) this.profileBack = [...this.profileBack, this.profileJid];
    else if (!push) this.profileBack = [];
    this.profileJid = jid;
  }

  closeProfile(): void {
    this.profileJid = null;
    this.profileBack = [];
  }

  profileGoBack(): void {
    const prev = this.profileBack[this.profileBack.length - 1];
    if (!prev) return this.closeProfile();
    this.profileBack = this.profileBack.slice(0, -1);
    this.profileJid = prev;
  }

  searchInChat(jid: string): void {
    this.searchChat = jid;
    this.openSearch();
  }

  openExport(chatJid: string, chatName: string, messageIds: string[] = []): void {
    this.exportDialog = { chatJid, chatName, messageIds };
  }

  openForward(chatJid: string, messageIds: string[]): void {
    this.forwardDialog = { chatJid, messageIds };
  }

  /** Fix placeholder chat names from bridge metadata and report how many changed. */
  async refreshNames(): Promise<number | null> {
    try {
      const n = await api.refreshChatNames();
      resetAvatarCache();
      toasts.success(n === 0 ? "Names are up to date" : `Fixed ${n} chat name${n === 1 ? "" : "s"}`);
      return n;
    } catch (e) {
      toasts.error(e, "Couldn't refresh names");
      return null;
    }
  }

  openSearch(text?: string): void {
    if (text !== undefined) this.searchText = text;
    this.searchNonce++;
    this.view = "search";
  }

  async setTheme(theme: Theme): Promise<void> {
    this.localTheme = theme;
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      /* storage unavailable */
    }
    if (this.settings) {
      const prev = this.settings;
      this.settings = { ...prev, theme };
      try {
        this.settings = await api.setSettings($state.snapshot(this.settings));
      } catch (e) {
        this.settings = prev;
        toasts.error(e, "Couldn't save theme");
      }
    }
  }

  async refreshOverview(): Promise<void> {
    try {
      this.overview = await api.overview();
    } catch {
      /* shown in System view */
    }
  }

  async refreshIssues(): Promise<void> {
    try {
      const issues = await api.listIssues();
      this.openIssues = issues.filter((i) => i.status === "open").length;
    } catch {
      /* ignore */
    }
  }

  async refreshUnread(): Promise<void> {
    try {
      const page = await api.listChats({ filter: null, kind: "all", unread_only: true, cursor: null, limit: 500 });
      this.unreadChats = page.items.length;
    } catch {
      /* ignore */
    }
  }

  async refreshJobs(): Promise<void> {
    try {
      this.jobs = await api.listJobs(50);
    } catch {
      /* ignore */
    }
  }

  upsertJob(job: JobRecord): void {
    const i = this.jobs.findIndex((j) => j.id === job.id);
    if (i >= 0) this.jobs[i] = job;
    else this.jobs = [job, ...this.jobs];
  }

  private debouncedUnread = debounce(() => void this.refreshUnread(), 400);

  /** Load initial state and subscribe to backend events. Idempotent. */
  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    try {
      const t = localStorage.getItem(THEME_KEY);
      if (t === "light" || t === "dark" || t === "system") this.localTheme = t;
    } catch {
      /* ignore */
    }
    if (typeof window !== "undefined" && window.matchMedia) {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      this.systemDark = mq.matches;
      const onChange = (e: MediaQueryListEvent) => (this.systemDark = e.matches);
      mq.addEventListener?.("change", onChange);
      this.disposers.push(() => mq.removeEventListener?.("change", onChange));
    }
    this.disposers.push(
      api.subscribe("bridge:state", (s) => (this.conn = s)),
      api.subscribe("job:progress", (p) => {
        const j = this.jobs.find((x) => x.id === p.id);
        if (j) {
          j.done = p.done;
          j.total = p.total;
          j.phase = p.phase;
          j.message = p.message;
          if (j.status === "queued") j.status = "running";
        }
      }),
      api.subscribe("job:finished", (job) => {
        this.upsertJob(job);
        if (job.status === "succeeded") toasts.success(`${job.label} finished`, job.message ?? undefined);
        else if (job.status === "failed") toasts.push({ kind: "error", title: `${job.label} failed`, message: job.error ?? "", action: { label: "Open System", run: () => this.go("system") } });
        void this.refreshOverview();
      }),
      api.subscribe("issue:new", (issue) => {
        void this.refreshIssues();
        if (this.settings?.desktop_notifications && typeof document !== "undefined" && document.hidden) {
          void notify(`wa-desk: ${issue.title}`, issue.source);
        }
      }),
      api.subscribe("automation:fired", (e) => {
        if (this.settings?.desktop_notifications && typeof document !== "undefined" && document.hidden) {
          void notify(`Rule fired: ${e.rule_name}`, e.outcomes.join(" · "));
        }
      }),
      api.subscribe("db:new-messages", () => this.debouncedUnread()),
      api.subscribe("db:changed", () => this.debouncedUnread()),
    );
    const [settings, conn] = await Promise.allSettled([api.getSettings(), api.connectionStatus()]);
    if (settings.status === "fulfilled") {
      this.settings = settings.value;
      this.localTheme = settings.value.theme;
    } else toasts.error(settings.reason, "Couldn't load settings");
    if (conn.status === "fulfilled") this.conn = conn.value;
    void this.refreshOverview();
    void this.refreshIssues();
    void this.refreshUnread();
    void this.refreshJobs();
    calls.start();
  }

  stop(): void {
    for (const d of this.disposers) d();
    this.disposers = [];
    this.started = false;
    calls.stop();
  }
}

export const app = new AppState();
