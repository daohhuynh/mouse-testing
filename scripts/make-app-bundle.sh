#!/bin/sh
# Builds mouse-testing.app.
#
# This exists because of how macOS attributes permissions. TCC grants attach to
# the "responsible process", which for a binary started from a terminal is the
# terminal or editor, not the binary. Running `cargo run` therefore asks you to
# grant Input Monitoring to your terminal, which is both confusing and wider
# than it needs to be.
#
# A bundle launched with `open` is started by launchd, so it becomes its own
# responsible process and gets its own entry in System Settings.
set -eu

cd "$(dirname "$0")/.."
APP="target/mouse-testing.app"
ID="dev.mousetesting.suite"

cargo build --release

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>mouse-testing</string>
  <key>CFBundleIdentifier</key><string>$ID</string>
  <key>CFBundleName</key><string>mouse testing suite</string>
  <key>CFBundleDisplayName</key><string>mouse testing suite</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <!-- Shown in the Input Monitoring prompt. -->
  <key>NSInputMonitoringUsageDescription</key>
  <string>Reads mouse reports so it can measure report rate, click timing and sensor behaviour. Nothing is transmitted anywhere.</string>
</dict>
</plist>
PLIST

cp target/release/mouse-testing "$APP/Contents/MacOS/mouse-testing"

# Ad-hoc signature with a stable identifier. Not a Developer ID signature, so
# macOS still identifies the app partly by its code hash: rebuilding changes
# that hash and can make you re-grant Input Monitoring. That is a property of
# unsigned software, not a bug here.
codesign --force --sign - --identifier "$ID" "$APP" >/dev/null 2>&1 \
  || echo "note: codesign failed; the bundle still runs but the permission grant will be less stable"

echo "built $APP"
echo
echo "run it with:   open $APP"
echo "(use 'open', not the binary directly, so macOS treats it as its own app"
echo " for permission purposes rather than attributing it to your terminal)"
