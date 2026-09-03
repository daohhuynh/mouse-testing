#!/bin/sh
# Builds the app and installs it into /Applications, so it can be launched from
# Launchpad, Spotlight or the Finder like anything else.
#
# No sudo, deliberately. /Applications is root:admin, group-writable, and has no
# sticky bit, so an admin account can simply copy into it. Installing with sudo
# would leave a root-owned bundle, and then every future reinstall would need
# sudo as well, permanently.
#
# No Gatekeeper prompt either: quarantine is attached by whatever DOWNLOADED a
# file, and nothing downloaded this one. A locally built binary carries only
# com.apple.provenance, which Gatekeeper does not act on. (`spctl -a` still
# reports "rejected" for an ad-hoc signature, but enforcement is gated on the
# quarantine bit, which is absent.) If you ever zip this app and send it to
# another Mac, that copy WILL be quarantined, and the fix there is
# `xattr -dr com.apple.quarantine "/Applications/Mouse Testing.app"`.
set -eu

cd "$(dirname "$0")/.."
NAME="Mouse Testing"
SRC="target/$NAME.app"
DEST="/Applications/$NAME.app"

sh scripts/make-app-bundle.sh

if [ ! -d "$SRC" ]; then
  echo "expected $SRC to exist after the build" >&2
  exit 1
fi

REPLACING=no
if [ -d "$DEST" ]; then
  REPLACING=yes
  # A running app cannot be replaced cleanly, and the copy would half-succeed.
  if pgrep -f "$DEST/Contents/MacOS/" >/dev/null 2>&1; then
    echo "\"$NAME\" is running. Quit it first, then run this again." >&2
    exit 1
  fi
fi

if [ ! -w /Applications ]; then
  echo "/Applications is not writable by this account." >&2
  echo "Either use an administrator account, or keep the app where it is and" >&2
  echo "run it with:  open \"$SRC\"" >&2
  exit 1
fi

# Remove first rather than copying over the top: merging into an existing bundle
# fails partway if that bundle came from a .pkg and is root-owned.
# ditto rather than cp, because it preserves extended attributes and the code
# signature exactly, which a plain recursive copy does not promise to.
rm -rf "$DEST"
ditto "$SRC" "$DEST"

# Register the bundle so Spotlight and Launchpad see it without waiting for a
# background scan.
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
[ -x "$LSREGISTER" ] && "$LSREGISTER" -f "$DEST" >/dev/null 2>&1 || true

if [ "$REPLACING" = yes ]; then
  # Only needed when REPLACING an app: macOS caches the icon against the bundle,
  # and a fresh install has nothing cached yet. Restarting the Dock is routine
  # and it comes straight back.
  touch "$DEST"
  killall Dock >/dev/null 2>&1 || true
  echo
  echo "replaced the existing copy and restarted the Dock so the icon refreshes"
fi

echo
echo "installed  $DEST"
echo
echo "Launch it from Launchpad, Spotlight, or the Applications folder."
echo
echo "One permission is needed, and only for the device-level measurements:"
echo
echo "  1. Open the app once."
echo "  2. System Settings > Privacy & Security > Input Monitoring"
echo "  3. Switch on \"$NAME\". If it is not listed, press + and pick"
echo "     $DEST"
echo "  4. Quit and reopen the app. macOS does not apply the grant to a"
echo "     process that is already running."
echo
echo "Grant it to THIS copy. The permission itself follows the app rather than"
echo "the folder, but macOS registers both copies under one identifier, so"
echo "having a second one under target/ makes the Settings row ambiguous about"
echo "which it is describing."
echo
echo "Rebuilding does NOT cost you the grant unless the code actually changed:"
echo "the build and the signature are both deterministic, so an unchanged"
echo "rebuild produces the same hash and the grant still matches. A real code"
echo "change does invalidate it, and the repair is:"
echo "  tccutil reset ListenEvent dev.mousetesting.suite"
echo
echo "Everything else works with no permission at all, and the app tells you"
echo "which measurements are blocked rather than reporting them as zero."
