#!/usr/bin/env bash
# Print the release notes for a tag: the fixed intro, then every change since the previous tag.
# Usage: scripts/release-notes.sh <tag> [repo-url]
set -euo pipefail
cd "$(dirname "$0")/.."
tag="${1:?usage: release-notes.sh <tag> [repo-url]}"
repo="${2:-https://github.com/rishindra2005/wa-desk}"

prev="$(git describe --tags --abbrev=0 --match 'v*' "${tag}^" 2>/dev/null || true)"
range="${prev:+${prev}..}${tag}"

cat .github/release-intro.md
echo
if [[ -n "$prev" ]]; then
  echo "## Changes since ${prev}"
else
  echo "## Changes"
fi
echo
# One line per commit, oldest first; merges and version-bump commits are left out.
changes="$(git log --no-merges --reverse --format='%s' "$range" | grep -vE '^release: v[0-9]' || true)"
if [[ -n "$changes" ]]; then
  sed 's/^/- /' <<<"$changes"
else
  echo "- Maintenance release."
fi
if [[ -n "$prev" ]]; then
  echo
  echo "**Full changelog:** ${repo}/compare/${prev}...${tag}"
fi
