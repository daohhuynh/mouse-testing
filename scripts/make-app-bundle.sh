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
# A human name, because this ends up in /Applications, in Launchpad, in
# Spotlight and in the Input Monitoring list, and "mouse-testing" reads like a
# build artifact in all four.
APP="target/Mouse Testing.app"
ID="dev.mousetesting.suite"

cargo build --release

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>mouse-testing</string>
  <key>CFBundleIdentifier</key><string>$ID</string>
  <key>CFBundleName</key><string>Mouse Testing</string>
  <key>CFBundleDisplayName</key><string>Mouse Testing</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <!-- Shown in the Input Monitoring prompt. -->
  <key>NSInputMonitoringUsageDescription</key>
  <string>Reads mouse reports so it can measure report rate, click timing and sensor behaviour. Nothing is transmitted anywhere.</string>
</dict>
</plist>
PLIST

cp target/release/mouse-testing "$APP/Contents/MacOS/mouse-testing"

# The icon is committed, so a normal build never regenerates it. Run
# scripts/make-icon.sh only when changing the mark itself.
if [ -f assets/AppIcon.icns ]; then
  cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
else
  echo "note: assets/AppIcon.icns is missing, so the app will show the generic icon"
  echo "      run scripts/make-icon.sh to build it"
fi

# Ad-hoc signature with a stable identifier. For an ad-hoc signature the
# designated requirement is ONLY the code hash: not the bundle identifier, not
# the path, not a certificate. So the Input Monitoring grant follows the app
# wherever it is moved, and survives a rebuild that did not change the code,
# because both the compile and the signing are deterministic (measured: an
# unchanged rebuild reproduces the binary hash exactly). A real code change does
# invalidate it. Repair with:
#     tccutil reset ListenEvent dev.mousetesting.suite
codesign --force --sign - --identifier "$ID" "$APP" >/dev/null 2>&1 \
  || echo "note: codesign failed; the bundle still runs but the permission grant will be less stable"

echo "built $APP"
echo
echo "run it with:          open \"$APP\""
echo "install it with:      sh scripts/install.sh"
echo "(use 'open', not the binary directly, so macOS treats it as its own app"
echo " for permission purposes rather than attributing it to your terminal)"
