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
| `export_media_files` | `{ req: MediaExportReq }` | `MediaExportSummary`: combined download of up to 500 media/document files. `req.mode = "zip"`: `req.dest` is a `.zip` path from a save dialog (`.zip` is appended when missing); entries are streamed into it, via a `.partial` sibling renamed on success. `"folder"`: `req.dest` is an existing folder from a folder dialog; files keep their original names and are never overwritten. In both modes name collisions become `name (1).ext`, `name (2).ext` (case-insensitive). Sources resolve only through `bridge_media_path` + `resolve_within` (never `.bridge-token`, `*.db*` or anything outside the media roots) and missing media is downloaded first. Destinations inside the bridge store/phone-media/outbox folders are `forbidden`. A per-file problem never fails the call: it is listed in `failures` (`saved + failed` = unique items). Allowed in read-only mode (a local export); media not on disk is then reported as failed. Emits `media-export:progress` |
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
| `stage_pasted_file` | `{ name: string \| null, mime: string \| null, dataBase64: string }` | `string`: absolute path of a private copy in the outbox (`outbox/<uuid>/<name>`, dir 0700, file 0600). For clipboard or dropped items that have no path (screenshots, images copied in a browser). Up to 50 MiB; a missing name becomes `pasted-<time>.<ext>` from the MIME type; secret/database names are `forbidden`; blocked in read-only mode. The UI then sends the path like any chosen file |
| `copy_image` | `{ chatJid, messageId }` | `void`: puts the image (decoded pixels, not a path) on the system clipboard. Goes through `resolve_within` on the media roots; `invalid` for non-images, corrupt or oversized (over 64 MiB or 100 MP) files. On Linux it uses GTK's clipboard, which works on both Wayland and X11 |
| `set_zoom` | `{ percent: number }` | `number`: the zoom now active. Clamped to 50 to 200, saved as `Settings.zoom_percent` and applied to the window (webview zoom). `set_settings` never changes it |
| `react` | `{ req: ReactReq }` | `void` |
| `mark_read` | `{ chatJid }` | `void`: the UI calls it when a chat is opened, and again while it is open in a focused window (new messages, window focus). It always withdraws the chat's desktop notifications first, even when the chat is already read, in read-only mode or with the bridge offline |
| `connection_status` | none | `ConnectionState` |
| `bridge_supervisor` | none | `SupervisorStatus`: what the bundled-bridge supervisor is doing (`starting`, `external` (a bridge not started by wa-desk: one answers on the URL, the systemd user unit `whatsapp-bridge.service` is enabled/active in `auto` mode, or the store lock `runtime/.wa-desk-bridge.lock` is held elsewhere; `detail` says which), `disabled`, `not_installed`, `running` + `pid`, `crashed`, `gave_up`, `error`, `stopped`), the last error in `detail`, and `restarts` since start |
| `mcp_status` | none | `McpStatus` `{state, pid, url, detail, restarts}`: what the MCP server supervisor is doing. `state` is `disabled` (setting off), `starting` (`detail` says "first run installs dependencies" while the Python environment is built), `running` + `pid`, `crashed`, `gave_up`, `needs_uv` (`uv` not found; `detail` says to install it from https://docs.astral.sh/uv/), `not_installed` (no server source in this build), `port_busy` (another program holds the port), `external` (an MCP server not started by wa-desk already answers there), `error`, `stopped`. `url` is always `http://127.0.0.1:<mcp_port>/mcp` |
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
| `set_settings` | `{ settings: Settings }` | `Settings`. `Settings.bridge_mode` (`auto` \| `external` \| `bundled`, default `auto`) decides who runs the bridge and is read at app start: `auto` uses a bridge that already answers on the bridge URL and otherwise spawns the bundled `wa-bridge` sidecar; `external` never spawns; `bundled` always spawns and reports an `error` state if the port is taken. `Settings.mcp_enabled` (default `true`) and `Settings.mcp_port` (default `8771`, 1024 to 65535 and not the bridge port, else `invalid`) control the bundled MCP server and, unlike `bridge_mode`, apply immediately: `set_settings` starts, stops or restarts the server before it returns (it does not wait for the server to answer; poll `mcp_status`) |

A message with several attachments is sent by the UI as one `send_message` per file, strictly in order: the typed text is the caption of the first file only (and only the first quotes a reply). A failed file is marked failed with a Retry button and does not stop the others.

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
| `media-export:progress` | `MediaExportProgress` | While `export_media_files` runs: `done`/`total` count two passes (resolve/download, then write); the final event has `done == total`. Match on `request_id` |
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
