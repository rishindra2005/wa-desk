# IPC contract: Tauri commands, events and media URLs

Types are defined in `crates/wa-core/src/types.rs` and exported to `app/ui/src/lib/bindings/`. This file is the single source of truth for the names below. Every command is implemented in `app/src-tauri`, and the UI calls them through `app/ui/src/lib/api.ts`.

Calling conventions:
- JS arguments are **camelCase**; Tauri maps them to the snake_case Rust parameter names.
- Every command rejects with an `AppError` object `{kind, message, detail?}`.

## Commands

| Command | Args (JS) | Returns |
|---|---|---|
| `overview` | none | `Overview` |
| `list_chats` | `{ query: ChatQuery }` | `Page<ChatSummary>`. Community parents hold no messages of their own: they are listed only with `kind: "community"`, sorted by (and showing the time of) the newest activity in their groups |
| `get_chat` | `{ jid: string }` | `ChatSummary` |
| `get_messages` | `{ query: MessagesQuery }` | `Page<Message>`: newest page first, items in **ascending** time order, and `next_cursor` pages **older**. With `after`, returns the next newer page (ascending); `next_cursor` then pages **newer** and is `null` at the newest message |
| `get_message_context` | `{ chatJid, messageId, radius: number }` | `Message[]` (ascending, includes the target) |
| `search` | `{ query: SearchQuery }` | `SearchResults` |
| `list_media` | `{ query: MediaQuery }` | `Page<MediaItem>` (newest first) |
| `ensure_media` | `{ chatJid, messageId }` | `MediaRef`: downloads on demand (asks the phone if needed), `source != "missing"` on success |
| `preview_doc` | `{ chatJid, messageId }` | `PreviewRef`: office docs are converted to PDF (cached) |
| `open_media_external` | `{ chatJid, messageId }` | `void`: opens with the system default app |
| `save_media_as` | `{ chatJid, messageId, dest: string }` | `string`: the saved path (`dest` comes from the save dialog) |
| `open_path` | `{ path: string }` | `void`: opens an exported or converted file; only files inside the exports or cache folders (`forbidden` otherwise) |
| `reveal_path` | `{ path: string }` | `void`: shows that file in the file manager (same scope) |
| `get_profile` | `{ jid: string }` | `Profile` (contact, group/community, or channel). Live from the bridge with a 10 min cache; falls back to local data when the bridge is down |
| `chat_stats` | `{ chatJid }` | `ChatStats` |
| `list_links` | `{ chatJid: string \| null, cursor: string \| null, limit: number }` | `Page<LinkItem>` (newest first) |
| `set_starred` | `{ chatJid, messageIds: string[], starred: boolean }` | `void` |
| `list_starred` | `{ chatJid: string \| null, cursor: string \| null, limit: number }` | `Page<SearchHit>` (newest first, `highlights` empty) |
| `export_chat` | `{ req: ExportReq }` | `string`: path of the export (folder for HTML with media, else a file) in the exports folder |
| `forward_messages` | `{ req: ForwardReq }` | `SendResult[]` (one per message × target; never more than 5 targets) |
| `copy_messages_text` | `{ chatJid, messageIds: string[] }` | `string`: WhatsApp-style "[time] Name: text" lines for the clipboard |
| `refresh_chat_names` | none | `number`: how many chat names were fixed from bridge metadata (groups, communities, channels) |
| `list_calls` | `{ query: CallQuery }` | `Page<CallRecord>` (newest first). Not part of `get_messages`: the UI merges `list_calls` for the open chat into the thread by time |
| `reject_call` | `{ callId, callerJid }` | `void`: declines a ringing call (bridge `RejectCall`); `invalid` if it is not ringing |
| `archive_count` | none | `number`: archived chats (for the "Archived" row) |
| `set_archived` | `{ jid, archived: boolean }` | `void`: syncs to the phone through the bridge |
| `set_pinned` | `{ jid, pinned: boolean }` | `void`: at most 3 pinned chats, like WhatsApp (`invalid` beyond that) |
| `set_muted` | `{ jid, until: number \| null }` | `void`: epoch seconds; `-1` = always; `null` = unmute |
| `resync_chat_state` | none | `number`: chats whose archive/pin/mute state changed after a full app-state resync |
| `send_message` | `{ req: SendReq }` | `SendResult` |
| `react` | `{ req: ReactReq }` | `void` |
| `mark_read` | `{ chatJid }` | `void`: the UI calls it when a chat is opened, and again while it is open in a focused window (new messages, window focus). It always withdraws the chat's desktop notifications first, even when the chat is already read, in read-only mode or with the bridge offline |
| `connection_status` | none | `ConnectionState` |
| `reconnect` | none | `ConnectionState` |
| `start_job` | `{ spec: JobSpec }` | `JobRecord` |
| `cancel_job` | `{ id: number }` | `void` |
| `list_jobs` | `{ limit: number }` | `JobRecord[]` (newest first) |
| `list_rules` | none | `Rule[]` |
| `save_rule` | `{ rule: Rule }` | `Rule`: validated; `Invalid` on a bad regex, time or URL |
| `delete_rule` | `{ id: number }` | `void` |
| `test_rule` | `{ rule: Rule, sample: Message }` | `RuleEval` |
| `automation_log` | `{ limit: number }` | `AutomationEvent[]` |
| `set_automation_enabled` | `{ enabled: boolean }` | `Settings`: enabling sets the watermark, so old messages never fire |
| `query_logs` | `{ filter: LogFilter }` | `LogLine[]` (newest first) |
| `list_issues` | none | `Issue[]` (open first, then by `last_seen` desc) |
| `resolve_issue` | `{ id: number }` | `void` |
| `clear_resolved_issues` | none | `number` |
| `export_issue_bundle` | `{ ids: number[] \| null }` | `string`: path of the zip in the exports folder |
| `get_settings` | none | `Settings` |
| `set_settings` | `{ settings: Settings }` | `Settings` |

The UI picks local files (attachments, convert inputs) and save destinations with `@tauri-apps/plugin-dialog` and passes absolute paths in. The backend validates every path.

## Events (`listen(name, cb)`)

| Event | Payload | When it fires |
|---|---|---|
| `db:new-messages` | `NewMessagesEvent` | New rows arrived (about 1 s poll) |
| `db:changed` | `DbChangedEvent` | Edits, revokes, renames: refresh the affected chats |
| `bridge:state` | `ConnectionState` | On change, polled every 3 s |
| `job:progress` | `JobProgress` | Throttled to ≤ 5/s per job |
| `job:finished` | `JobRecord` | A job reached a terminal status |
| `automation:fired` | `AutomationEvent` | A rule matched (dry-run or real) |
| `call:update` | `CallRecord` | An incoming call started ringing, or its state changed (answered elsewhere, missed, ended) |
| `issue:new` | `Issue` | An issue was recorded for the first time, or reopened |
| `notification:open` | `NotificationOpenEvent` | The user clicked a desktop notification of `chat_jid`. The backend has already raised the window and withdrawn that chat's other notifications; the UI opens the chat |

## Media URLs (custom protocol `wa-media`)

The backend returns complete URLs. The UI uses them as-is in `src`/`href`/`fetch`.

- `wa-media://localhost/m/<urlenc chat_jid>/<urlenc message_id>`: the original file. Supports HTTP `Range` (video/audio seeking) and sets `Content-Type`.
- `wa-media://localhost/t/<chat>/<id>`: a 256 px thumbnail, generated on demand (images, video frames, PDF first page). Returns 404 if none is available.
- `wa-media://localhost/p/<chat>/<id>`: the preview PDF for office documents, cached after `preview_doc`.
- `wa-media://localhost/a/<urlenc jid>`: profile picture (contact, group, community or channel), cached; 404 when there is none (show initials). Add `?full=1` for the full-size picture. A community group without a picture of its own (always the case for the announcement group) gets its community's picture. A 404 for "no picture" is cacheable (`max-age=1800`); a 404 because the bridge is offline, slow or failing is `no-store`, and the UI retries a failed picture after 5 minutes.
- `wa-media://localhost/x/<urlenc absolute path>`: an exported or converted output file. Only files inside the exports or cache folders are served.

Paths are always resolved, canonicalised and scope-checked in Rust. `.bridge-token` and `*.db*` files are never served.

## Desktop notifications

Message and call notifications are shown by the backend (never by the UI) through the freedesktop notification service on Linux (`org.freedesktop.Notifications` over D-Bus), so that each one has an id that can be replaced and closed:
- **One per chat.** A newer message replaces the chat's notification (`replaces_id`), whose title then carries the count: "Alice (3 messages)". A poll with new messages in more than 3 chats shows one "N new messages in M chats" summary instead, which accumulates.
- **Calls.** A ringing call's notification is replaced by the missed-call one, and closed when the call is answered elsewhere, rejected or ends.
- **Withdrawn** when the chat is opened or marked read here (`mark_read`), when its read marker moves so that it is no longer unread (read on the phone; seen by the watcher), or when one of its notifications is clicked (`notification:open`). The summary closes once all of its chats are read.
- Without a D-Bus session (or on other platforms) notifications go through `tauri-plugin-notification` and cannot be replaced or withdrawn.
