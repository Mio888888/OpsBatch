#!/bin/sh
# Runs the encrypted vault against the real system keychain under a throw-away keychain service and data directory,
# then removes both. The master-key item of the default service (com.opsbatch.app) is shared with the Tauri build and is
# never touched. Works on macOS (Keychain), Linux (Secret Service) and Windows (Credential Manager, from Git Bash).
#   scripts/verify-real-vault.sh               the keychain round trip: setup, restart, stale key, restart
#   scripts/verify-real-vault.sh --no-keychain a machine without a usable keychain: the startup password is asked, twice
set -u
cd "$(dirname "$0")/.."
SVC="com.opsbatch.app.verify-$$-$(date +%s)"
DIR="$(mktemp -d "${TMPDIR:-/tmp}/opsbatch-vault-check.XXXXXX")"
export OPSBATCH_KEYCHAIN_SERVICE="$SVC" OPSBATCH_VAULT_CHECK_DIR="$DIR"
cargo build -q -p ops-core --features real-vault --example vault_check || exit 2
EXE=""
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) EXE=".exe" ;; esac
BIN="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/debug/examples/vault_check$EXE"
cleanup() {
  "$BIN" cleanup >/dev/null 2>&1
  # safety net: whatever the example could not delete, only under the throw-away service
  if [ "$(uname)" = "Darwin" ]; then
    while security delete-generic-password -s "$SVC" >/dev/null 2>&1; do :; done
  fi
  rm -rf "$DIR"
}
trap cleanup EXIT INT TERM
status=0
if [ "${1:-}" = "--no-keychain" ]; then
  PHASES="nokeychain nokeychain-restart"
else
  PHASES="setup restart stale restart"
fi
for phase in $PHASES; do
  echo "== $phase"
  "$BIN" "$phase" || status=1
done
echo "== cleanup"
echo "service=$SVC dir=$DIR (removed on exit)"
exit $status
