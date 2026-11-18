#!/usr/bin/env bash
# Build the Go WhatsApp bridge (bridge/ submodule) as a Tauri sidecar:
#   app/src-tauri/binaries/wa-bridge-<rust target triple>[.exe]
# Usage: scripts/build-bridge.sh [<rust-target-triple>]   (default: host triple)
# Needs Go and a C compiler (go-sqlite3 requires CGO). Cross-compiling needs the
# matching C cross compiler in CC (for example x86_64-w64-mingw32-gcc for Windows).
set -euo pipefail
cd "$(dirname "$0")/.."

triple="${1:-}"
if [[ -z "$triple" ]]; then
  triple="$(rustc -vV | sed -n 's/^host: //p')"
fi
[[ -n "$triple" ]] || { echo "could not determine the target triple" >&2; exit 1; }

src="bridge/whatsapp-bridge"
[[ -f "$src/go.mod" ]] || { echo "missing $src: run 'git submodule update --init'" >&2; exit 1; }

ext=""
export CGO_ENABLED=1
case "$triple" in
  x86_64-pc-windows-*) export GOOS=windows GOARCH=amd64; ext=".exe" ;;
  aarch64-apple-darwin) export GOOS=darwin GOARCH=arm64 ;;
  x86_64-apple-darwin) export GOOS=darwin GOARCH=amd64 ;;
  aarch64-unknown-linux-gnu) export GOOS=linux GOARCH=arm64 ;;
  *-pc-windows-*) echo "unsupported Windows target: $triple" >&2; exit 1 ;;
esac

out="$PWD/app/src-tauri/binaries/wa-bridge-${triple}${ext}"
mkdir -p "$(dirname "$out")"
echo "building $out"
(cd "$src" && go build -trimpath -ldflags "-s -w" -o "$out" .)
echo "built $out"
