# IPC contract: Tauri commands, events and media URLs

Types are defined in `crates/wa-core/src/types.rs` and exported to `app/ui/src/lib/bindings/`. This file is the single source of truth for the names below. The integrator implements every command in `app/src-tauri`, and the UI calls them through `app/ui/src/lib/api.ts`.

Calling conventions:
- JS arguments are **camelCase**; Tauri maps them to the snake_case Rust parameter names.
- Every command rejects with an `AppError` object `{kind, message, detail?}`.

## Commands

| Command | Args (JS) | Returns |
|---|---|---|
| `overview` | none | `Overview` |
| `list_chats` | `{ query: ChatQuery }` | `Page<ChatSummary>` |
| `get_chat` | `{ jid: string }` | `ChatSummary` |
| `get_messages` | `{ query: MessagesQuery }` | `Page<Message>`: newest page first, items in **ascending** time order, and `next_cursor` pages **older** |
| `get_message_context` | `{ chatJid, messageId, radius: number }` | `Message[]` (ascending, includes the target) |
| `search` | `{ query: SearchQuery }` | `SearchResults` |
| `list_media` | `{ query: MediaQuery }` | `Page<MediaItem>` (newest first) |
| `ensure_media` | `{ chatJid, messageId }` | `MediaRef`: downloads on demand (asks the phone if needed), `source != "missing"` on success |
| `preview_doc` | `{ chatJid, messageId }` | `PreviewRef`: office docs are converted to PDF (cached) |
| `open_media_external` | `{ chatJid, messageId }` | `void`: opens with the system default app |
| `save_media_as` | `{ chatJid, messageId, dest: string }` | `string`: the saved path (`dest` comes from the save dialog) |
| `open_path` | `{ path: string }` | `void`: opens an exported or converted file; only files inside the exports or cache folders (`forbidden` otherwise) |
| `reveal_path` | `{ path: string }` | `void`: shows that file in the file manager (same scope) |
| `send_message` | `{ req: SendReq }` | `SendResult` |
| `react` | `{ req: ReactReq }` | `void` |
| `mark_read` | `{ chatJid }` | `void` |
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
| `issue:new` | `Issue` | An issue was recorded for the first time, or reopened |

## Media URLs (custom protocol `wa-media`)

The backend returns complete URLs. The UI uses them as-is in `src`/`href`/`fetch`.

- `wa-media://localhost/m/<urlenc chat_jid>/<urlenc message_id>`: the original file. Supports HTTP `Range` (video/audio seeking) and sets `Content-Type`.
- `wa-media://localhost/t/<chat>/<id>`: a 256 px thumbnail, generated on demand (images, video frames, PDF first page). Returns 404 if none is available.
- `wa-media://localhost/p/<chat>/<id>`: the preview PDF for office documents, cached after `preview_doc`.
- `wa-media://localhost/x/<urlenc absolute path>`: an exported or converted output file. Only files inside the exports or cache folders are served.

Paths are always resolved, canonicalised and scope-checked in Rust. `.bridge-token` and `*.db*` files are never served.
