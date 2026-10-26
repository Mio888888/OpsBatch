#!/bin/sh
# Shared by the packaging scripts: workspace version, architecture name, output dir, cargo feature flags.
set -e
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="${OPSBATCH_VERSION:-$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)}"
case "${OPSBATCH_ARCH:-$(uname -m)}" in
  arm64|aarch64) ARCH=aarch64 ;;
  x86_64|amd64) ARCH=x86_64 ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
DIST="${OPSBATCH_DIST:-$ROOT/dist}"
mkdir -p "$DIST"
# Set OPSBATCH_FEATURES=real-vault for release builds once the encrypted vault ships (TASKS M1-05).
FEATURES="${OPSBATCH_FEATURES:-}"
if [ -z "$FEATURES" ]; then
  echo "warning: building WITHOUT the real-vault feature - secrets are not persisted and the app uses the isolated -dev data directory" >&2
  FEATURE_ARGS=""
else
  FEATURE_ARGS="--features $FEATURES"
fi
# Installer names are part of the update protocol (crates/ops-core/src/update.rs::installer_asset_name).
