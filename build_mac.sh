#!/bin/bash
# Builds "File Tracker.app" into the dist folder.
# Run from Terminal in this folder:   bash build_mac.sh
set -e
cd "$(dirname "$0")"

if [ "$1" != "--skip-build" ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    [ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust isn't installed yet. Install it with this command, then run this script again:"
    echo
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo
    exit 1
  fi
  echo "Building File Tracker (the first build takes a few minutes)…"
  cargo build --release
fi

APP="dist/File Tracker.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/file_tracker "$APP/Contents/MacOS/File Tracker"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>File Tracker</string>
  <key>CFBundleDisplayName</key><string>File Tracker</string>
  <key>CFBundleIdentifier</key><string>com.filetracker.app</string>
  <key>CFBundleVersion</key><string>1.0.0</string>
  <key>CFBundleShortVersionString</key><string>1.0.0</string>
  <key>CFBundleExecutable</key><string>File Tracker</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Ad-hoc signature so macOS will run it (you'll still be asked to confirm the first time)
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
echo
echo "Done: $(pwd)/$APP"
echo "Drag it into your Applications folder. The first time, right-click it and choose Open."
