#!/usr/bin/env bash
# Build the AppImage without FUSE.
#
# linuxdeploy aborts ("filesystem error: status: Permission denied") when any
# PATH entry cannot be stat'ed (e.g. /root/.cargo/bin inherited from a sudo'd
# shell), so unreadable PATH entries are dropped first. NO_STRIP avoids
# linuxdeploy's bundled strip failing on newer ELF sections;
# APPIMAGE_EXTRACT_AND_RUN lets linuxdeploy/appimagetool run without FUSE.
set -euo pipefail
cd "$(dirname "$0")/../app/src-tauri"

clean=""
IFS=':' read -r -a parts <<< "${PATH}"
for d in "${parts[@]}"; do
  [[ -n "$d" && -d "$d" && -x "$d" ]] || continue
  clean="${clean:+$clean:}$d"
done

export PATH="$clean" NO_STRIP=true APPIMAGE_EXTRACT_AND_RUN=1
exec cargo tauri build -b appimage "$@"
