#!/bin/sh
# Write dist/SHA256SUMS ("<hash>  <file>") for every release asset. The in-app updater downloads this file next to
# the installer and refuses an installer whose hash is missing or different.
set -e
dir="${1:-$(cd "$(dirname "$0")/.." && pwd)/dist}"
cd "$dir"
rm -f SHA256SUMS
files=$(ls | grep -E '\.(dmg|exe|AppImage|deb|rpm)$' || true)
[ -n "$files" ] || { echo "no release assets in $dir" >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum $files > SHA256SUMS
else
  shasum -a 256 $files > SHA256SUMS
fi
cat SHA256SUMS
