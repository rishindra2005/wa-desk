// Typed wrappers for every Tauri command and event in docs/CONTRACT.md.
// Every command rejects with an `ApiError` (an `Error` that is also an `AppError`).

import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppError } from "./bindings/AppError";
import type { AutomationEvent } from "./bindings/AutomationEvent";
import type { CallQuery } from "./bindings/CallQuery";
import type { CallRecord } from "./bindings/CallRecord";
import type { ChatQuery } from "./bindings/ChatQuery";
import type { ChatStats } from "./bindings/ChatStats";
import type { ExportReq } from "./bindings/ExportReq";
import type { ForwardReq } from "./bindings/ForwardReq";
import type { LinkItem } from "./bindings/LinkItem";
import type { Profile } from "./bindings/Profile";
import type { SearchHit } from "./bindings/SearchHit";
import type { ChatSummary } from "./bindings/ChatSummary";
import type { ConnectionState } from "./bindings/ConnectionState";
import type { DbChangedEvent } from "./bindings/DbChangedEvent";
import type { NotificationOpenEvent } from "./bindings/NotificationOpenEvent";
import type { ErrorKind } from "./bindings/ErrorKind";
import type { Issue } from "./bindings/Issue";
import type { JobProgress } from "./bindings/JobProgress";
import type { JobRecord } from "./bindings/JobRecord";
import type { JobSpec } from "./bindings/JobSpec";
import type { LogFilter } from "./bindings/LogFilter";
import type { LogLine } from "./bindings/LogLine";
import type { MediaItem } from "./bindings/MediaItem";
import type { MediaExportProgress } from "./bindings/MediaExportProgress";
import type { MediaExportReq } from "./bindings/MediaExportReq";
import type { MediaExportSummary } from "./bindings/MediaExportSummary";
import type { MediaQuery } from "./bindings/MediaQuery";
import type { MediaRef } from "./bindings/MediaRef";
import type { Message } from "./bindings/Message";
import type { MessagesQuery } from "./bindings/MessagesQuery";
import type { NewMessagesEvent } from "./bindings/NewMessagesEvent";
import type { Overview } from "./bindings/Overview";
import type { Page } from "./bindings/Page";
import type { PreviewRef } from "./bindings/PreviewRef";
import type { ReactReq } from "./bindings/ReactReq";
import type { Rule } from "./bindings/Rule";
import type { RuleEval } from "./bindings/RuleEval";
import type { SearchQuery } from "./bindings/SearchQuery";
import type { SearchResults } from "./bindings/SearchResults";
import type { SendReq } from "./bindings/SendReq";
import type { SendResult } from "./bindings/SendResult";
import type { Settings } from "./bindings/Settings";

export const ERROR_KINDS: readonly ErrorKind[] = [
  "not_found", "bridge_down", "auth", "forbidden", "busy", "timeout", "invalid", "external", "cancelled", "internal",
];

/** An `Error` carrying the contract's `AppError` fields. */
export class ApiError extends Error implements AppError {
  readonly kind: ErrorKind;
  readonly detail?: string;
  constructor(err: AppError) {
    super(err.message);
    this.name = "ApiError";
    this.kind = err.kind;
    if (err.detail !== undefined) this.detail = err.detail;
  }
  toJSON(): AppError {
    return this.detail === undefined
      ? { kind: this.kind, message: this.message }
      : { kind: this.kind, message: this.message, detail: this.detail };
  }
}

function isErrorKind(v: unknown): v is ErrorKind {
  return typeof v === "string" && (ERROR_KINDS as readonly string[]).includes(v);
}

/** Normalise anything thrown (AppError object, JSON string, Error, string…) to an `ApiError`. */
export function toAppError(e: unknown): ApiError {
  if (e instanceof ApiError) return e;
  if (e && typeof e === "object") {
    const o = e as Record<string, unknown>;
    if (isErrorKind(o.kind)) {
      const message = typeof o.message === "string" && o.message ? o.message : o.kind;
      const detail = typeof o.detail === "string" ? o.detail : undefined;
      return new ApiError(detail === undefined ? { kind: o.kind, message } : { kind: o.kind, message, detail });
    }
    if (e instanceof Error) {
      const detail = e.stack ?? undefined;
      return new ApiError(detail ? { kind: "internal", message: e.message || "Unexpected error", detail } : { kind: "internal", message: e.message || "Unexpected error" });
    }
    if (typeof o.message === "string") return new ApiError({ kind: "internal", message: o.message });
  }
  if (typeof e === "string") {
    const s = e.trim();
    if (s.startsWith("{")) {
      try {
        const parsed: unknown = JSON.parse(s);
        if (parsed && typeof parsed === "object" && isErrorKind((parsed as Record<string, unknown>).kind)) {
          return toAppError(parsed);
        }
      } catch {
        /* fall through: treat as plain text */
      }
    }
    return new ApiError({ kind: "internal", message: s || "Unexpected error" });
  }
  return new ApiError({ kind: "internal", message: "Unexpected error", detail: String(e) });
}

/** Invoke a command, normalising rejections. */
export async function call<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toAppError(e);
  }
}

// ---------------------------------------------------------------- commands

export const overview = () => call<Overview>("overview");
export const listChats = (query: ChatQuery) => call<Page<ChatSummary>>("list_chats", { query });
export const archiveCount = () => call<number>("archive_count");
export const setArchived = (jid: string, archived: boolean) => call<void>("set_archived", { jid, archived });
export const setPinned = (jid: string, pinned: boolean) => call<void>("set_pinned", { jid, pinned });
export const setMuted = (jid: string, until: number | null) => call<void>("set_muted", { jid, until });
export const resyncChatState = () => call<number>("resync_chat_state");
export const getChat = (jid: string) => call<ChatSummary>("get_chat", { jid });
export const listCalls = (query: CallQuery) => call<Page<CallRecord>>("list_calls", { query });
export const rejectCall = (callId: string, callerJid: string) => call<void>("reject_call", { callId, callerJid });
export const getMessages = (query: MessagesQuery) => call<Page<Message>>("get_messages", { query });
export const getMessageContext = (chatJid: string, messageId: string, radius: number) =>
  call<Message[]>("get_message_context", { chatJid, messageId, radius });
export const search = (query: SearchQuery) => call<SearchResults>("search", { query });
export const listMedia = (query: MediaQuery) => call<Page<MediaItem>>("list_media", { query });
export const ensureMedia = (chatJid: string, messageId: string) => call<MediaRef>("ensure_media", { chatJid, messageId });
export const previewDoc = (chatJid: string, messageId: string) => call<PreviewRef>("preview_doc", { chatJid, messageId });
export const openMediaExternal = (chatJid: string, messageId: string) =>
  call<void>("open_media_external", { chatJid, messageId });
export const saveMediaAs = (chatJid: string, messageId: string, dest: string) =>
  call<string>("save_media_as", { chatJid, messageId, dest });
export const exportMediaFiles = (req: MediaExportReq) => call<MediaExportSummary>("export_media_files", { req });
export const openPath = (path: string) => call<void>("open_path", { path });
export const revealPath = (path: string) => call<void>("reveal_path", { path });
export const getProfile = (jid: string) => call<Profile>("get_profile", { jid });
export const chatStats = (chatJid: string) => call<ChatStats>("chat_stats", { chatJid });
export const listLinks = (chatJid: string | null, cursor: string | null, limit: number) =>
  call<Page<LinkItem>>("list_links", { chatJid, cursor, limit });
export const setStarred = (chatJid: string, messageIds: string[], starred: boolean) =>
  call<void>("set_starred", { chatJid, messageIds, starred });
export const listStarred = (chatJid: string | null, cursor: string | null, limit: number) =>
  call<Page<SearchHit>>("list_starred", { chatJid, cursor, limit });
export const exportChat = (req: ExportReq) => call<string>("export_chat", { req });
export const forwardMessages = (req: ForwardReq) => call<SendResult[]>("forward_messages", { req });
export const copyMessagesText = (chatJid: string, messageIds: string[]) =>
  call<string>("copy_messages_text", { chatJid, messageIds });
export const refreshChatNames = () => call<number>("refresh_chat_names");
export const sendMessage = (req: SendReq) => call<SendResult>("send_message", { req });
/** Write clipboard/dropped bytes (no path) to a private outbox file; returns its absolute path. */
export const stagePastedFile = (name: string | null, mime: string | null, dataBase64: string) =>
  call<string>("stage_pasted_file", { name, mime, dataBase64 });
/** Put a downloaded image message on the system clipboard as image data. */
export const copyImage = (chatJid: string, messageId: string) => call<void>("copy_image", { chatJid, messageId });
/** Set (clamped, persisted) app zoom in percent; returns the level now active. */
export const setZoom = (percent: number) => call<number>("set_zoom", { percent });
export const react = (req: ReactReq) => call<void>("react", { req });
export const markRead = (chatJid: string) => call<void>("mark_read", { chatJid });
export const connectionStatus = () => call<ConnectionState>("connection_status");
export const reconnect = () => call<ConnectionState>("reconnect");
export const startJob = (spec: JobSpec) => call<JobRecord>("start_job", { spec });
export const cancelJob = (id: number) => call<void>("cancel_job", { id });
export const listJobs = (limit: number) => call<JobRecord[]>("list_jobs", { limit });
export const listRules = () => call<Rule[]>("list_rules");
export const saveRule = (rule: Rule) => call<Rule>("save_rule", { rule });
export const deleteRule = (id: number) => call<void>("delete_rule", { id });
export const testRule = (rule: Rule, sample: Message) => call<RuleEval>("test_rule", { rule, sample });
export const automationLog = (limit: number) => call<AutomationEvent[]>("automation_log", { limit });
export const setAutomationEnabled = (enabled: boolean) => call<Settings>("set_automation_enabled", { enabled });
export const queryLogs = (filter: LogFilter) => call<LogLine[]>("query_logs", { filter });
export const listIssues = () => call<Issue[]>("list_issues");
export const resolveIssue = (id: number) => call<void>("resolve_issue", { id });
export const clearResolvedIssues = () => call<number>("clear_resolved_issues");
export const exportIssueBundle = (ids: number[] | null) => call<string>("export_issue_bundle", { ids });
export const getSettings = () => call<Settings>("get_settings");
export const setSettings = (settings: Settings) => call<Settings>("set_settings", { settings });

/** Every command name in the contract (used by tests to check mock coverage). */
export const COMMANDS = [
  "overview", "list_chats", "get_chat", "get_messages", "get_message_context", "search", "list_media",
  "ensure_media", "preview_doc", "open_media_external", "save_media_as", "export_media_files", "open_path", "reveal_path", "get_profile",
  "chat_stats", "list_links", "set_starred", "list_starred", "export_chat", "forward_messages", "copy_messages_text",
  "refresh_chat_names", "archive_count", "set_archived", "set_pinned", "set_muted", "resync_chat_state", "send_message", "stage_pasted_file", "copy_image", "set_zoom", "react", "mark_read", "list_calls", "reject_call",
  "connection_status", "reconnect", "start_job", "cancel_job", "list_jobs", "list_rules", "save_rule",
  "delete_rule", "test_rule", "automation_log", "set_automation_enabled", "query_logs", "list_issues",
  "resolve_issue", "clear_resolved_issues", "export_issue_bundle", "get_settings", "set_settings",
] as const;
export type CommandName = (typeof COMMANDS)[number];

// ---------------------------------------------------------------- events

export interface EventMap {
  "db:new-messages": NewMessagesEvent;
  "db:changed": DbChangedEvent;
  "bridge:state": ConnectionState;
  "job:progress": JobProgress;
  "job:finished": JobRecord;
  "automation:fired": AutomationEvent;
  "issue:new": Issue;
  "call:update": CallRecord;
  "media-export:progress": MediaExportProgress;
  "notification:open": NotificationOpenEvent;
}
export type EventName = keyof EventMap;
export const EVENTS: readonly EventName[] = [
  "db:new-messages", "db:changed", "bridge:state", "job:progress", "job:finished", "automation:fired", "issue:new", "call:update", "media-export:progress", "notification:open",
];

/** Subscribe to a backend event with a typed payload. */
export function onEvent<K extends EventName>(name: K, cb: (payload: EventMap[K]) => void): Promise<UnlistenFn> {
  return listen<EventMap[K]>(name, (e) => cb(e.payload));
}

export const onNewMessages = (cb: (p: NewMessagesEvent) => void) => onEvent("db:new-messages", cb);
export const onDbChanged = (cb: (p: DbChangedEvent) => void) => onEvent("db:changed", cb);
export const onBridgeState = (cb: (p: ConnectionState) => void) => onEvent("bridge:state", cb);
export const onJobProgress = (cb: (p: JobProgress) => void) => onEvent("job:progress", cb);
export const onJobFinished = (cb: (p: JobRecord) => void) => onEvent("job:finished", cb);
export const onAutomationFired = (cb: (p: AutomationEvent) => void) => onEvent("automation:fired", cb);
export const onIssueNew = (cb: (p: Issue) => void) => onEvent("issue:new", cb);
export const onCallUpdate = (cb: (p: CallRecord) => void) => onEvent("call:update", cb);
export const onNotificationOpen = (cb: (p: NotificationOpenEvent) => void) => onEvent("notification:open", cb);

/**
 * Subscribe for the lifetime of the calling component (use inside `$effect`
 * or component init). Returns a synchronous disposer that also handles
 * unsubscribing before `listen` has resolved.
 */
export function subscribe<K extends EventName>(name: K, cb: (payload: EventMap[K]) => void): () => void {
  let disposed = false;
  let un: UnlistenFn | null = null;
  onEvent(name, (p) => {
    if (!disposed) cb(p);
  })
    .then((u) => {
      if (disposed) void u();
      else un = u;
    })
    .catch(() => {
      /* event plugin unavailable (tests) */
    });
  return () => {
    disposed = true;
    if (un) void un();
  };
}
