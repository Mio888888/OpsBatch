#!/bin/sh
# Wrap a built spike binary in a minimal macOS .app so computer-use / Launch Services can see it.
# Usage: scripts/bundle-spike.sh spike-terminal   ->  target/spike-apps/spike-terminal.app
set -e
name="$1"
root="$(cd "$(dirname "$0")/.." && pwd)"
app="$root/target/spike-apps/$name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$root/target/debug/$name" "$app/Contents/MacOS/$name"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>$name</string>
<key>CFBundleIdentifier</key><string>com.mio.opsbatch.$name</string>
<key>CFBundleName</key><string>$name</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0.1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
echo "$app"
