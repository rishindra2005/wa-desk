# wa-desk: contributor and agent guide

wa-desk is a Rust + Tauri v2 desktop app on top of a live WhatsApp bridge. It covers:
- browsing, search and media/docs viewing;
- sending, reacting and attaching;
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

## Ownership
Edit only the paths you own. Ask the integrator to change anything else.

| Path | Owner |
|---|---|
| `crates/wa-core/**` (shared contract), `crates/wa-testkit/**`, `Cargo.toml`, `app/src-tauri/**`, `scripts/**`, this file | integrator |
| `crates/wa-db/**` | agent A (data) |
| `crates/wa-bridge/**`, `crates/wa-jobs/**`, `crates/wa-automation/**`, `crates/wa-obs/**` | agent B (effects) |
| `app/ui/**` | agent C (UI) |

You may add dependencies to your own crate's `Cargo.toml`, but only with versions from `[workspace.dependencies]`. If you need a new workspace dependency, choose a version released at least 2 weeks ago and say so in your hand-back.

## Contract
- **Types:** `crates/wa-core/src/types.rs`. They are exported to TypeScript in `app/ui/src/lib/bindings/*.ts` by `cargo test -p wa-core`.
- **Traits:** `crates/wa-core/src/traits.rs`.
- **Errors:** `crates/wa-core/src/error.rs`. Return `wa_core::Result<T>` everywhere and map errors to the right `ErrorKind`.
- **Times:** Unix seconds (UTC). Parse stored text with `wa_core::time::parse_ts`, never compare raw timestamp strings.
- **Bridge store paths:** use `wa_core::paths::bridge_media_path`.
- **Anything served, linked or handed to a tool:** pass it through `wa_core::paths::resolve_within`.
- **Configuration:** `wa_core::config::Paths`. Tests use `wa_testkit::Fixture`, which follows the real layout and seeds edge cases.

## Quality bar (must be green before handing back)
```
cargo fmt --all -- --check
cargo clippy -p <your crates> --all-targets -- -D warnings
cargo nextest run -p <your crates>        # or cargo test
# UI: cd app/ui && npm run check && npm test && npx playwright test
```

Other expectations:
- No `unwrap()`/`expect()` in non-test code except on provably infallible values, with a comment explaining why.
- Use `tracing` for structured logs with fields, not `println!`. Never log tokens, message text at info level, or full phone numbers.
- Doc comments on public items. Keep functions small.
- Tests cover the edge cases listed in your task.
- Commit on your branch with clear messages. End each message with the attribution lines the integrator gives you.

## Running things
- Rust: `cargo test --workspace`, `cargo nextest run --workspace`.
- UI dev against mocks: `cd app/ui && npm run dev` (mock IPC is on when `window.__TAURI_INTERNALS__` is absent).
- Whole app: `cargo tauri dev` from `app/src-tauri`. `WA_DESK_DATA_ROOT=/tmp/wa-copy` points it at a copy of the data.
- Full gate: `scripts/check.sh`.
