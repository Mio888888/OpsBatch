#!/bin/sh
# Build OpsBatch-<version>-linux-<arch>.AppImage (the file the in-app updater looks for) and opsbatch_<version>_<deb-arch>.deb.
#
# Needs `appimagetool` on PATH for the AppImage (https://github.com/AppImage/appimagetool/releases) and `dpkg-deb`
# for the .deb; either step is skipped with a warning when its tool is missing. Build on the oldest distribution you
# support (the CI job uses ubuntu-22.04) so the binary's glibc requirement stays low.
set -e
. "$(dirname "$0")/../common.sh"

cd "$ROOT"
# shellcheck disable=SC2086
cargo build --release -p opsbatch $FEATURE_ARGS
BIN="$ROOT/target/release/opsbatch"

APPDIR="$(mktemp -d)/OpsBatch.AppDir"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/512x512/apps"
cp "$BIN" "$APPDIR/usr/bin/opsbatch"
cp "$ROOT/packaging/linux/opsbatch.desktop" "$APPDIR/opsbatch.desktop"
cp "$ROOT/packaging/linux/opsbatch.desktop" "$APPDIR/usr/share/applications/opsbatch.desktop"
cp "$ROOT/packaging/icons/icon.png" "$APPDIR/opsbatch.png"
cp "$ROOT/packaging/icons/icon.png" "$APPDIR/usr/share/icons/hicolor/512x512/apps/opsbatch.png"
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/opsbatch" "$@"
RUN
chmod +x "$APPDIR/AppRun"

if command -v appimagetool >/dev/null 2>&1; then
  ARCH="$ARCH" appimagetool "$APPDIR" "$DIST/OpsBatch-$VERSION-linux-$ARCH.AppImage"
  echo "$DIST/OpsBatch-$VERSION-linux-$ARCH.AppImage"
else
  echo "warning: appimagetool not found - AppImage skipped" >&2
fi

if command -v dpkg-deb >/dev/null 2>&1; then
  case "$ARCH" in aarch64) DEB_ARCH=arm64 ;; *) DEB_ARCH=amd64 ;; esac
  PKG="$(mktemp -d)/pkg"
  mkdir -p "$PKG/DEBIAN" "$PKG/usr/bin" "$PKG/usr/share/applications" "$PKG/usr/share/icons/hicolor/512x512/apps"
  cp "$BIN" "$PKG/usr/bin/opsbatch"
  cp "$ROOT/packaging/linux/opsbatch.desktop" "$PKG/usr/share/applications/opsbatch.desktop"
  cp "$ROOT/packaging/icons/icon.png" "$PKG/usr/share/icons/hicolor/512x512/apps/opsbatch.png"
  cat > "$PKG/DEBIAN/control" <<CTL
Package: opsbatch
Version: $VERSION
Section: admin
Priority: optional
Architecture: $DEB_ARCH
Maintainer: mio
Depends: libc6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libvulkan1, libfontconfig1, libasound2 | libasound2t64
Description: OpsBatch - batch operations workbench
 SSH terminals, RDP/VNC, batch commands, script libraries, file transfer and workflows in one desktop app.
CTL
  dpkg-deb --build --root-owner-group "$PKG" "$DIST/opsbatch_${VERSION}_${DEB_ARCH}.deb" >/dev/null
  echo "$DIST/opsbatch_${VERSION}_${DEB_ARCH}.deb"
else
  echo "warning: dpkg-deb not found - .deb skipped" >&2
fi
