# Contributing to wa-desk

wa-desk is a Rust + Tauri v2 desktop app on top of a live WhatsApp bridge. It covers:
- browsing, search and media/docs viewing;
- sending, reacting and attaching;
- archived, pinned and muted chats, and the call log;
- file conversion;
- automations;
- a system dashboard with logs and issue reports.

## Never break the live system
- The Go bridge (`whatsapp-bridge.service`, `127.0.0.1:8080`) owns `messages.db` and `whatsapp.db`.
- **Open them read-only.** Use `file:<path>?mode=ro`, `busy_timeout` ≤ 2000 ms, and paged reads (≤ 500 rows).
  - No transaction may be held across an `.await`.
  - No single query may run longer than about 300 ms; use a progress-handler deadline.
  - The bridge's go-sqlite3 gives up after 5 s on a lock, and when that happens a WhatsApp message is lost.
- The only write ever allowed to `messages.db` is mirroring `phone_media_links`, in short transactions.
- Never read, log or serve `.bridge-token` or any `*.db*` file to the UI. All bridge HTTP happens in Rust.

## Project layout

| Path | What it holds |
|---|---|
| `crates/wa-core` | Shared contract: types (exported to TypeScript), traits, errors, paths, config |
| `crates/wa-testkit` | Test fixtures that follow the real data layout and seed edge cases |
| `crates/wa-db` | Read-only access to the bridge databases, the app's FTS index, the change watcher |
| `crates/wa-bridge` | Bridge HTTP client and outbox |
| `crates/wa-jobs` | Backfill, phone media sync, document linker, converters |
| `crates/wa-automation` | Automation rules (off and dry-run by default) |
| `crates/wa-obs` | Logging, redaction, issue reports |
| `app/src-tauri` | The Tauri app: commands, `wa-media://` protocol, tray |
| `app/ui` | Svelte 5 UI, with a mock backend for development and tests |

Changes to `crates/wa-core` change the contract for everything else. Update `docs/CONTRACT.md` in the same change.

New dependencies must use versions from `[workspace.dependencies]`. If you add a workspace dependency, choose a version released at least 2 weeks ago.

## Contract
- **Types:** `crates/wa-core/src/types.rs`. They are exported to TypeScript in `app/ui/src/lib/bindings/*.ts` by `cargo test -p wa-core`.
- **Traits:** `crates/wa-core/src/traits.rs`.
- **Errors:** `crates/wa-core/src/error.rs`. Return `wa_core::Result<T>` everywhere and map errors to the right `ErrorKind`.
- **Times:** Unix seconds (UTC). Parse stored text with `wa_core::time::parse_ts`, never compare raw timestamp strings.
- **Bridge store paths:** use `wa_core::paths::bridge_media_path`.
- **Anything served, linked or handed to a tool:** pass it through `wa_core::paths::resolve_within`.
- **Configuration:** `wa_core::config::Paths`. Tests use `wa_testkit::Fixture`.
- Bridge endpoints the app relies on are documented in `docs/BRIDGE_API.md`.

## Quality bar
All of this must be green before a change is merged:
```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace        # or cargo test
# UI: cd app/ui && npm run check && npm test && npx playwright test
```
`scripts/check.sh` runs the whole gate.

Other expectations:
- No `unwrap()`/`expect()` in non-test code except on provably infallible values, with a comment explaining why.
- Use `tracing` for structured logs with fields, not `println!`. Never log tokens, message text at info level, or full phone numbers.
- Doc comments on public items. Keep functions small.
- Tests cover the edge cases of the change.
- Clear, focused commit messages.

## The bundled bridge
The Go bridge lives in the `bridge/` git submodule ([wa-bridge](https://github.com/rishindra2005/wa-bridge)). Do not edit it from this repo; change it in its own repository and bump the submodule.
- Clone with `git clone --recurse-submodules`, or run `git submodule update --init` in an existing checkout.
- Build the sidecar for the host with `scripts/build-bridge.sh`. It needs Go and a C compiler (go-sqlite3 uses CGO) and writes `app/src-tauri/binaries/wa-bridge-<target triple>[.exe]`, which is git-ignored. Pass a Rust target triple to build for another target (cross-compiling needs the matching C compiler in `CC`).
- Only release builds bundle it: `tauri.release.conf.json` adds `bundle.externalBin`. `tauri.conf.json` does not, so dev builds and tests never need Go. Run `scripts/build-bridge.sh` before `cargo tauri build --config tauri.release.conf.json`.
- The supervisor is `app/src-tauri/src/bridge_supervisor.rs`. Its tests use fake binaries and temp dirs, never a real bridge or store. In `cargo tauri dev` there is no sidecar unless you set `WA_DESK_BRIDGE_BIN`; never point it at your real store.

## Running things
- Rust: `cargo test --workspace`, `cargo nextest run --workspace`.
- UI dev against mocks: `cd app/ui && npm run dev` (mock IPC is on when `window.__TAURI_INTERNALS__` is absent).
- Whole app: `cargo tauri dev` from `app/src-tauri`. `WA_DESK_DATA_ROOT=/tmp/wa-copy` points it at a copy of the data, and `WA_DESK_READ_ONLY=1` blocks every write.
- Package: `cd app/src-tauri && cargo tauri build -b deb`.
