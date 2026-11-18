# Bridge additions used by wa-desk (Go, whatsapp-bridge)

All endpoints use the existing auth rules (loopback Host header plus Bearer token) and the existing error style: plain text from `http.Error`, or JSON `{success:false,message}`. Times are RFC3339. JIDs are full strings (`user@server`).

## Receipts → delivery ticks
- New table in `messages.db`:
  ```sql
  message_receipts(message_id TEXT, chat_jid TEXT, status TEXT, updated_at TIMESTAMP, PRIMARY KEY(message_id, chat_jid))
  ```
- `status` is one of `delivered`, `read`, `played`.
- It is written from `events.Receipt` for receipts **from others about our messages**:
  - type `""` (delivery) → `delivered`
  - `read` → `read`
  - `played` → `played`
  - Ignore `read-self`, `retry` and `sender`.
- The status only ever moves forward: `delivered` < `read` < `played`. Write it with `INSERT ... ON CONFLICT DO UPDATE ... WHERE <rank(new) > rank(old)>`.
- In groups, `read` is stored once **any** participant has read it. This is a simplification of WhatsApp, which waits for all of them.
- History messages have no receipts. wa-desk shows a single tick (`sent`) for its own messages that have no row here.

## Chat metadata
- New table:
  ```sql
  chat_meta(jid TEXT PRIMARY KEY, chat_type TEXT NOT NULL, parent_jid TEXT, name TEXT, description TEXT, updated_at TIMESTAMP)
  ```
- `chat_type` is one of `group`, `community`, `channel` or `broadcast`.
- It is filled by `POST /api/chats/refresh` and opportunistically when group info is fetched.
- When `chats.name` is NULL, empty, all digits or `Group <id>`, the refresh also writes the real name into `chats.name`.

## Endpoints
| Method / path | Request | Response |
|---|---|---|
| `POST /api/chats/refresh` | none | `{success, groups, communities, channels, renamed}`. Uses `GetJoinedGroups` (with `IsParent` / `LinkedParentJID`) and `GetSubscribedNewsletters`. Idempotent. At most once per 60 s; within that window it returns the cached result. |
| `GET /api/profile?jid=` | none | `{jid, push_name, verified_name, about, about_set_at, picture_id, devices, business: {description, category, email, website, address} \| null}` from `GetUserInfo` and `GetBusinessProfile` (when it's a business). 404 for an unknown or non-user JID. |
| `GET /api/group?jid=` | none | `{jid, name, topic, created_at, creator, is_parent, linked_parent_jid, announce, locked, participants: [{jid, phone_jid, lid, is_admin, is_super_admin, display_name}], sub_groups: [{jid, name}]}` (`sub_groups` only when `is_parent`, via `GetSubGroups`) |
| `GET /api/newsletter?jid=` | none | `{jid, name, description, subscribers, verified, picture_id}` |
| `GET /api/avatar?jid=&full=0\|1` | none | `{path, picture_id}` for a file cached at `store/avatars/<jid with ':'→'_'>[-full].jpg`, refreshed when the picture ID changes or the file is more than 24 h old; 404 `{success:false,message:"no picture"}` when the chat has none (that answer is cached for 6 h). Downloads with `GetProfilePictureInfo` and then a plain HTTP GET of `info.URL`. |

Every endpoint returns 503 while disconnected, except `/api/avatar`, which still serves cached files.

## Implementation notes (bridge branch `local/wa-desk`, as deployed)
These are the differences from the text above. Clients should code against these.
- **Nullable fields.** Unknown values are JSON `null`, not `""`:
  - profile: `push_name`, `verified_name`, `about`, `about_set_at`, `picture_id`, `business`
  - group: `created_at`, `creator`, `linked_parent_jid`, and per participant `phone_jid`, `lid`, `display_name`
  - newsletter: `picture_id`
  - avatar: `picture_id`
- `about_set_at` is **always `null`**, because whatsmeow's `GetUserInfo` doesn't return the time the status was set.
- `business.description` and `business.website` come only from the business profile options, which whatsmeow parses only partly. They are usually `""`. `business.category` is the category names joined with `", "`. Any `GetBusinessProfile` error gives `business: null`.
- `/api/profile` takes phone JIDs and `@lid` JIDs. A LID is mapped to its phone JID through the LID store before the query, and `jid` in the response echoes the input. A non-user JID returns 404.
- In `/api/group`, `creator` is the owner's phone JID when it is known, otherwise the owner's JID. When `is_parent` is false, the `sub_groups` key is **left out**. When it is true, `sub_groups` is always present, possibly as `[]`. A group you aren't in (or that doesn't exist) returns 404, and a non-`@g.us` JID returns 400. `/api/group` also replaces a placeholder `chats.name`, using an UPDATE only.
- `/api/newsletter` returns 400 for a non-`@newsletter` JID.
- `POST /api/chats/refresh` returns an extra field, `cached` (bool), which is true when the answer is the result cached within the 60 s window. `groups` doesn't count communities. A failed refresh is not cached. The refresh never INSERTs into `chats`; it only UPDATEs rows whose name is a placeholder. It doesn't produce `chat_type = 'broadcast'`.
- `/api/avatar` returns `{path, picture_id, cached}`, where `path` is absolute and `cached` is false only when the file was just downloaded.
  - Next to each image the cache keeps a `<file>.id` sidecar and, for "no picture" answers, a `<file>.none` marker.
  - Community pictures are fetched with `IsCommunity` (taken from `chat_meta`). Channel pictures come from `GetNewsletterInfo`.
  - A community's announcement group has no picture of its own (404 "no picture"); WhatsApp shows the community's picture there, and so does wa-desk, by asking again for the `parent_jid` from `chat_meta`. Community parents have no `messages` rows (`chats.last_message_time` is NULL).
  - An invalid JID returns 400. An upstream failure returns 502, unless a stale cached file exists, in which case that file is served.
  - With no cache entry and no connection, the endpoint returns 503.
- In `message_receipts`, `chat_jid` uses the same phone-JID form as `messages.chat_jid`, because LID DMs are resolved. Receipts for `status@broadcast` are not stored.
- Bad or missing `jid` returns 400 as JSON `{success:false,message}`. Wrong methods return 405 as plain text.

## Calls (wa-desk additions, v3)
- `POST /api/call/reject` `{call_id, caller_jid}` → `{success, message}`: `RejectCall(ctx, callerJID, callID)` (caller JID normalized to non-AD; `@s.whatsapp.net` or `@lid`). The row is matched by `call_id` + `result='in_progress'` + (`from_jid` or `chat_jid` = caller). Status codes: 400 bad JSON / missing or non-user `caller_jid`; 409 no ringing row (unknown, already answered/rejected/ended); 503 disconnected; 502 whatsmeow error (row untouched). On success the row becomes `result='rejected'`, `ended_at=now`, `duration_sec=0`; the later `CallTerminate` keeps `rejected`. (`whatsapp-bridge/calls_local.go`)
- **1:1 call media type:** in `CallOffer`, the offer node (`evt.Data`) is searched recursively; a `<video>` element at any depth, or a `media`/`type`/`call-type` attribute equal to `video` on any node, gives `call_type='video'`, otherwise `voice`. Keep group `CallOfferNotice.Media`.
- Calls stay in the existing `calls` table. wa-desk polls it through its watcher, so no webhook is needed.

## Call history backfill (wa-desk additions, v5)
Live events only see calls that ring while the bridge is online, never calls placed from the phone. The bridge now also fills `calls` from WhatsApp's stored call history (`whatsapp-bridge/calllog_local.go`):
- **App state** `call_log` mutations (`CallLogAction.CallLogRecord`): live incremental patches (`events.AppState`), and a read-only replay of every collection on `POST /api/calls/resync` (whatsmeow's own app-state versions are not touched).
- **History sync** `HistorySync.callLogRecords`, plus `CallLogMessage`s and missed-call stubs (`CALL_MISSED_*`, `SILENCED_UNKNOWN_CALLER_*`) in synced conversations; live `CallLogMessage`s too.
- `POST /api/calls/resync[?debug=1]` → `{success, cached, records, skipped: {reason: count}, inserted, updated, unchanged, replaced_synthetic, collections: [{name, version, mutations, call_log, call_log_removed, delete_individual_call_log, error?}]}`. At most once per 5 min (cached result otherwise); 503 while disconnected. A successful `POST /api/chats/resync-state` also starts it in the background. `debug=1` bypasses the cache and adds `debug: [{index, result, incoming, creator, group, participants, start, duration, call_type, stored}]` per `call_log` mutation.
- Row mapping: same columns and result values. `connected`/`accepted elsewhere` → `ended` (`duration_sec` = talk time); `rejected` → `rejected`; `missed`/`cancelled`/`unavailable`/`failed`/`abandoned`/silenced → `missed` (`duration_sec` 0); ongoing/upcoming records are skipped. A multi-party call whose overall result is `invalid` (or, outgoing, not connected) but has a participant with `connected` and a non-zero duration counts as `ended`. `is_group` = group JID present or more than one other participant (ad-hoc group calls use the first participant as `chat_jid`). WhatsApp puts our own JID in `callCreatorJID` even on incoming records and lists only the other parties as participants; incoming records without participants name no peer and are skipped (counted in `skipped.no_chat`). `is_from_me` = not incoming (outgoing calls have `from_jid` = our phone JID and `chat_jid` = the callee). `ended_at` = start + duration. `reason` = `<source>:<outcome>` with source `app_state`, `history_sync`, `message` or `stub` (live rows keep their terminate reason). JIDs are phone form; LIDs are mapped through the LID store.
- Merge rules: rows are matched by `call_id` across chats. A live row still `in_progress` takes the final result; a finished live row only gets empty columns filled (and LID `chat_jid`/`from_jid` replaced by the phone JID). Message and stub rows have no call ID, so they use `call_id = 'msg:<message id>'` and are skipped (or later deleted) when a real call of the same chat lies within 120 s.
- Backfilled rows get new rowids, so wa-desk's watcher reports them as new calls (`call:update`, and missed-call notifications when the window is unfocused) the first time they are inserted.

## Chat state: archive, pin, mute (wa-desk additions, v4)
- New table:
  ```sql
  chat_state(jid TEXT PRIMARY KEY, archived INTEGER NOT NULL DEFAULT 0, pinned INTEGER NOT NULL DEFAULT 0, muted_until INTEGER, updated_at TIMESTAMP)
  ```
  - `muted_until` is in epoch seconds; `-1` means forever; NULL means not muted.
  - Keys are phone-form JIDs (the same form `chats` uses). LIDs are mapped through the LID store.
- Filled from the app-state events `events.Archive`, `events.Pin` and `events.Mute`, live and during full syncs. Each is an upsert of that one field.
- `POST /api/chats/resync-state` → `{success, changed}`
  - Runs `FetchAppState(ctx, appstate.WAPatchRegularLow, true, false)` and then `WAPatchRegularHigh`, so archive, pin and mute are replayed.
  - Rate-limited to once per 5 minutes; otherwise it returns the cached result.
  - `changed` = rows whose values differ afterwards.
- `POST /api/chat/archive {jid, archived}`, `POST /api/chat/pin {jid, pinned}` and `POST /api/chat/mute {jid, until}` (`until`: epoch seconds, `-1` forever, `null` unmute) → `{success, message}`.
  - Use `appstate.BuildArchive` (with the last message's time and key from `messages`), `BuildPin` and `BuildMute`, then `SendAppState`.
  - Update `chat_state` on success.
  - 503 when disconnected; 400 bad JID; 502 on a whatsmeow error.
- **Implementation notes (as deployed, `whatsapp-bridge/chatstate_local.go`).** Clients should code against these:
  - **Full syncs don't emit events.** whatsmeow drops `Archive`/`Pin`/`Mute` events on a full sync (`EmitAppStateEventsOnFullSync` is false, and the bridge keeps it that way so other app-state events aren't replayed). whatsmeow still writes every mutation, full sync included, into its own `whatsmeow_chat_settings` table in `whatsapp.db`. So `chat_state` is **reconciled from that table**:
    - on every connect;
    - after `events.AppStateSyncComplete` for `regular_low` or `regular_high`;
    - at the end of `resync-state`.

    Live events still upsert their one field.
  - When a chat has both a `@lid` row and a phone row in whatsmeow's table, the LID row wins. A LID with no phone mapping is stored under its `@lid` JID.
  - `resync-state` returns `{success, changed, cached}`. `cached` is true inside the 5-minute window. `changed` compares a snapshot of `chat_state` taken before the sync with the table afterwards, where a missing row counts as not archived, not pinned and not muted. Status codes: 503 when disconnected, 502 when `FetchAppState` fails, 500 on a local DB error. A failed resync is not cached.
  - `muted_until`: whatsmeow's `MuteEndTimestamp` is in **milliseconds** and is stored ÷1000. A muted action whose end is missing, ≤0 or past whatsmeow's `MutedForever` (9999-12-31) is stored as `-1`. An expired mute keeps its past value, so clients should treat `muted_until <= now` (other than `-1`) as not muted.
  - **Write endpoints:**
    - **`jid`:** a phone JID, a `@lid` JID or a `@g.us` JID. Anything else returns 400. A LID input is stored under its phone JID.
    - **Patch index:** the patch goes out with the DM's **LID** when one is known, because WhatsApp now indexes DMs by LID. Otherwise it uses the JID as given.
    - **Required fields:** `archived` / `pinned` must be booleans. `until` is required: an integer that is either `-1` or in the future (seconds), or `null` to unmute. A missing or invalid value returns 400.
    - **Archive range:** archive uses the newest `messages` row of the chat (`id`, `is_from_me`, plus `sender` as the participant for others' group messages). A chat with no messages sends no range.
    - **Archive unpins:** archiving also sets `pinned = 0`, as `BuildArchive` does.
