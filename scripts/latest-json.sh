#!/bin/sh
# Write <dist>/latest.json, the manifest the Tauri updater of every already installed OpsBatch reads from
# https://github.com/<repo>/releases/latest/download/latest.json. Each platform points at the signed asset the Tauri
# updater knows how to install:
#
#   darwin-<arch>   OpsBatch-<version>-macos-<arch>.app.tar.gz   (the .app, replaced in place)
#   windows-x86_64  OpsBatch-<version>-windows-x86_64-setup.exe   (run as an installer)
#   linux-x86_64    OpsBatch-<version>-linux-x86_64.AppImage      (replaces the running AppImage)
#
#   scripts/latest-json.sh <dist> <tag> <owner/repo>      NOTES="release notes" is optional
#
# Fails when an asset or its .sig is missing: a manifest that leaves a platform out would silently stop updates there.
set -e
[ "$#" -eq 3 ] || { echo "usage: latest-json.sh <dist> <tag> <owner/repo>" >&2; exit 2; }
dist="$1"
tag="$2"
repo="$3"
version="${tag#v}"
base="https://github.com/$repo/releases/download/$tag"

platforms='{}'
add() {
  key="$1"
  file="$2"
  if [ ! -f "$dist/$file" ] || [ ! -s "$dist/$file.sig" ]; then
    echo "missing $file or its .sig for $key" >&2
    exit 1
  fi
  platforms=$(printf '%s' "$platforms" | jq --arg key "$key" --arg url "$base/$file" --rawfile sig "$dist/$file.sig" \
    '. + {($key): {signature: ($sig | rtrimstr("\n")), url: $url}}')
}
add darwin-aarch64 "OpsBatch-$version-macos-aarch64.app.tar.gz"
add darwin-x86_64 "OpsBatch-$version-macos-x86_64.app.tar.gz"
add windows-x86_64 "OpsBatch-$version-windows-x86_64-setup.exe"
add linux-x86_64 "OpsBatch-$version-linux-x86_64.AppImage"

jq -n --arg version "$version" \
  --arg notes "${NOTES:-See the assets to download this version.}" \
  --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --argjson platforms "$platforms" \
  '{version: $version, notes: $notes, pub_date: $date, platforms: $platforms}' > "$dist/latest.json"
cat "$dist/latest.json"
