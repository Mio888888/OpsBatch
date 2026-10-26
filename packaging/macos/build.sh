#!/bin/sh
# Build OpsBatch.app and OpsBatch-<version>-macos-<arch>.dmg into dist/.
#
#   packaging/macos/build.sh
#
# Optional environment:
#   OPSBATCH_TARGET      cargo target triple (e.g. x86_64-apple-darwin) for cross builds; ARCH follows it
#   OPSBATCH_FEATURES    cargo features (real-vault for production)
#   MACOS_SIGN_IDENTITY  "Developer ID Application: Name (TEAMID)" - without it the app is ad-hoc signed
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD   notarize + staple the dmg (needs MACOS_SIGN_IDENTITY)
set -e
. "$(dirname "$0")/../common.sh"

if [ -n "$OPSBATCH_TARGET" ]; then
  case "$OPSBATCH_TARGET" in aarch64-*) ARCH=aarch64 ;; x86_64-*) ARCH=x86_64 ;; esac
  TARGET_ARGS="--target $OPSBATCH_TARGET"
  BIN="$ROOT/target/$OPSBATCH_TARGET/release/opsbatch"
else
  TARGET_ARGS=""
  BIN="$ROOT/target/release/opsbatch"
fi

cd "$ROOT"
# shellcheck disable=SC2086
cargo build --release -p opsbatch $FEATURE_ARGS $TARGET_ARGS

APP="$DIST/OpsBatch.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/opsbatch"
cp "$ROOT/packaging/icons/icon.icns" "$APP/Contents/Resources/icon.icns"
sed "s/@VERSION@/$VERSION/g" "$ROOT/packaging/macos/Info.plist.in" > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

if [ -n "$MACOS_SIGN_IDENTITY" ]; then
  codesign --force --options runtime --timestamp --entitlements "$ROOT/packaging/macos/entitlements.plist" --sign "$MACOS_SIGN_IDENTITY" "$APP"
else
  echo "warning: MACOS_SIGN_IDENTITY not set - ad-hoc signing (Gatekeeper will warn on other machines)" >&2
  codesign --force --sign - "$APP"
fi
codesign --verify --deep --strict "$APP"

DMG="$DIST/OpsBatch-$VERSION-macos-$ARCH.dmg"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "OpsBatch $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"

if [ -n "$MACOS_SIGN_IDENTITY" ]; then
  codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$DMG"
  if [ -n "$APPLE_ID" ] && [ -n "$APPLE_TEAM_ID" ] && [ -n "$APPLE_APP_PASSWORD" ]; then
    xcrun notarytool submit "$DMG" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$DMG"
  else
    echo "warning: APPLE_ID / APPLE_TEAM_ID / APPLE_APP_PASSWORD not set - dmg is signed but not notarized" >&2
  fi
fi
echo "$DMG"
