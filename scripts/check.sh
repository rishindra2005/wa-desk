#!/usr/bin/env bash
# Full quality gate: formatting, lints, Rust tests, TS bindings, UI checks/tests.
set -euo pipefail
cd "$(dirname "$0")/.."
echo "== rustfmt";  cargo fmt --all -- --check
echo "== clippy";   cargo clippy --workspace --all-targets -- -D warnings
echo "== tests";    if command -v cargo-nextest >/dev/null; then cargo nextest run --workspace --no-fail-fast; else cargo test --workspace; fi
echo "== doctests"; cargo test --workspace --doc
echo "== bindings"; cargo test -p wa-core --quiet >/dev/null && git diff --exit-code -- app/ui/src/lib/bindings || { echo "bindings out of date (committed)"; exit 1; }
echo "== ui";       (cd app/ui && npm run check && npm test && npm run build)
if [[ "${E2E:-1}" == "1" ]]; then echo "== e2e"; (cd app/ui && npx playwright test); fi
echo "ALL CHECKS PASSED"
