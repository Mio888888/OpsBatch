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

# Libraries the AppImage leaves to the system: glibc and the loader (the binary needs the build host's glibc or newer
# anyway), the GPU stack (it has to match the driver) and the X11 core, fontconfig and freetype (every desktop has them,
# and a copy mixed with the system's configuration misbehaves). Everything else the binary links to travels along.
is_system_lib() {
  case "$1" in
    linux-vdso.so.*|ld-linux*|libc.so.*|libm.so.*|libdl.so.*|libpthread.so.*|librt.so.*|libutil.so.*|libresolv.so.*) return 0 ;;
    libanl.so.*|libnss_*|libgcc_s.so.*|libstdc++.so.*) return 0 ;;
    libGL.so.*|libGLX.so.*|libGLdispatch.so.*|libEGL.so.*|libOpenGL.so.*|libGLESv2.so.*|libdrm.so.*|libgbm.so.*) return 0 ;;
    libX11.so.*|libxcb.so.*|libXau.so.*|libXdmcp.so.*|libfontconfig.so.*|libfreetype.so.*) return 0 ;;
  esac
  return 1
}

# Copy the shared libraries the binary needs into $1/usr/lib and make the binary look there first. The binary is
# patched with RPATH instead of the AppRun setting LD_LIBRARY_PATH, which every process started from the terminal
# (ssh, the user's own programs) would inherit.
bundle_libs() {
  for tool in ldd patchelf ldconfig; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool is needed to bundle the libraries into the AppImage" >&2; exit 1; }
  done
  mkdir -p "$1/usr/lib"
  libs="$(ldd "$1/usr/bin/opsbatch" | awk '$2 == "=>" && $3 ~ /^\// { print $3 }')"
  # Loaded with dlopen at run time, so ldd does not list them.
  for soname in libvulkan.so.1 libwayland-client.so.0 libwayland-cursor.so.0 libwayland-egl.so.1 libxkbcommon.so.0 libxkbcommon-x11.so.0; do
    libs="$libs
$(ldconfig -p | awk -v want="$soname" '$1 == want { print $NF; exit }')"
  done
  libs="$(echo "$libs" | sort -u)"
  while read -r lib; do
    [ -n "$lib" ] || continue
    is_system_lib "$(basename "$lib")" && continue
    cp -L "$lib" "$1/usr/lib/"
  done <<EOF
$libs
EOF
  patchelf --force-rpath --set-rpath '$ORIGIN/../lib' "$1/usr/bin/opsbatch"
  # Every library the binary links to must now come from the AppDir or be one the system is expected to have.
  problems=""
  resolved="$(ldd "$1/usr/bin/opsbatch")"
  while read -r name arrow path _; do
    case "$name" in linux-vdso.so.*|/*|"") continue ;; esac
    case "$path" in "$1"/*) continue ;; esac
    is_system_lib "$name" && [ "$arrow" = "=>" ] && [ "$path" != "not" ] && continue
    problems="$problems $name"
  done <<EOF
$resolved
EOF
  if [ -n "$problems" ]; then
    echo "error: not bundled in the AppImage, and not provided by every system:$problems" >&2
    exit 1
  fi
  echo "bundled $(ls "$1/usr/lib" | wc -l | tr -d ' ') libraries:" >&2
  ls "$1/usr/lib" >&2
}

if command -v appimagetool >/dev/null 2>&1; then
  bundle_libs "$APPDIR"
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
