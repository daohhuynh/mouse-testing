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

# Refuse to rebuild a bundle something is running out of. Deleting an app's
# bundle while it runs does not stop it, but it does strip its icon: the Dock
# and Command-Tab fall back to the generic placeholder while the Finder still
# shows the real one, which reads as a broken install rather than as a
# self-inflicted wound. This is asked with lsof against the actual executable,
# not by matching argv, because a process started with a relative path carries a
# relative argv and a pattern match on the absolute path finds nothing.
running_from() {
  [ -x "$1/Contents/MacOS/mouse-testing" ] || return 1
  [ -n "$(lsof -t "$1/Contents/MacOS/mouse-testing" 2>/dev/null)" ]
}

for stale in "$APP" target/mouse-testing.app; do
  if running_from "$stale"; then
    echo "\"$stale\" is running. Quit it first, then run this again." >&2
    exit 1
  fi
done

# Sweep away bundles left by an earlier name. One of these outlived a rename
# once and took the Input Monitoring grant with it.
rm -rf target/mouse-testing.app

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
