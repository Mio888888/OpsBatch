#!/bin/sh
# Sign release assets with the updater key, writing <file>.sig next to each. The signature is a minisign signature in
# the format of the Tauri updater: the original OpsBatch app (Tauri) and the in-app updater of this build
# (crates/ops-core/src/update.rs) both verify it against the same public key.
#
#   TAURI_SIGNING_PRIVATE_KEY=<key text> [TAURI_SIGNING_PRIVATE_KEY_PASSWORD=...] scripts/sign-updates.sh <file>...
#
# Without TAURI_SIGNING_PRIVATE_KEY nothing is signed (builds from forks and pull requests have no secrets).
set -e
if [ -z "$TAURI_SIGNING_PRIVATE_KEY" ]; then
  echo "TAURI_SIGNING_PRIVATE_KEY is not set - not signing" >&2
  exit 0
fi
[ "$#" -gt 0 ] || { echo "usage: sign-updates.sh <file>..." >&2; exit 2; }
for file in "$@"; do
  [ -f "$file" ] || { echo "no such file: $file" >&2; exit 1; }
  rm -f "$file.sig"
  npx --yes @tauri-apps/cli@2 signer sign "$file" </dev/null
  [ -s "$file.sig" ] || { echo "signing $file left no signature" >&2; exit 1; }
done
