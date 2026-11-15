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
