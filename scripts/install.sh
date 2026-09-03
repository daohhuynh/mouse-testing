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

# --reset-permission clears the Input Monitoring grant even when the hash did
# not change. The automatic path below only fires when this install replaces a
# DIFFERENT build, which is the moment a grant dies. It cannot help someone who
# is already stuck: their installed copy is the build being refused, so a
# reinstall is byte-identical and there is nothing for the comparison to notice.
FORCE_RESET=no
if [ "${1:-}" = "--reset-permission" ]; then
  FORCE_RESET=yes
  shift
fi

cd "$(dirname "$0")/.."
NAME="Mouse Testing"
ID="dev.mousetesting.suite"
SRC="target/$NAME.app"
DEST="/Applications/$NAME.app"

sh scripts/make-app-bundle.sh

if [ ! -d "$SRC" ]; then
  echo "expected $SRC to exist after the build" >&2
  exit 1
fi

# Is anything running out of this bundle? Asked with lsof against the actual
# executable rather than by matching argv: a process started with a relative
# path carries a relative argv, so a pattern match on the absolute path silently
# finds nothing, and a match on the tail alone cannot tell this bundle's copy
# from the other one.
running_from() {
  [ -x "$1/Contents/MacOS/mouse-testing" ] || return 1
  [ -n "$(lsof -t "$1/Contents/MacOS/mouse-testing" 2>/dev/null)" ]
}

# The ad-hoc signature's designated requirement is ONLY the code hash, so this
# is the whole identity the Input Monitoring grant is pinned to.
bundle_cdhash() {
  codesign -dvvv "$1" 2>&1 | sed -n 's/^CDHash=//p'
}

REPLACING=no
OLD_HASH=""
if [ -d "$DEST" ]; then
  REPLACING=yes
  OLD_HASH=$(bundle_cdhash "$DEST")
  # A running app cannot be replaced cleanly, and the copy would half-succeed.
  if running_from "$DEST"; then
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

NEW_HASH=$(bundle_cdhash "$DEST")

# Register the bundle so Spotlight and Launchpad see it without waiting for a
# background scan. This runs BEFORE the tccutil call below, deliberately:
# tccutil takes a bundle identifier rather than a path and resolves it through
# LaunchServices, and the bundle was just deleted and recreated underneath it.
# Registering first costs nothing and removes the question.
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
[ -x "$LSREGISTER" ] && "$LSREGISTER" -f "$DEST" >/dev/null 2>&1 || true

# A changed code hash silently voids the Input Monitoring grant, and this is the
# only moment anything knows it happened. The row in System Settings keeps its
# switch in the ON position, because only the stored requirement stopped
# matching and nothing revoked the authorisation, so the app is refused while
# looking granted. That is unfalsifiable from the app's side: it sees a denial
# identical to never having been granted at all.
#
# The stale row is worse than no row, so it is cleared here rather than
# explained later. Nothing of value is destroyed: a grant whose hash no longer
# matches is already dead, and removing it lets the app prompt again and makes
# Settings honest. A grant that still matches is left alone, which is the whole
# benefit of the build being deterministic.
GRANT_VOIDED=no
if [ "$FORCE_RESET" = yes ]; then
  GRANT_VOIDED=forced
elif [ "$REPLACING" = yes ]; then
  if [ -z "$OLD_HASH" ] || [ -z "$NEW_HASH" ]; then
    # Refuse to guess. An unsigned or unreadable bundle on either side means the
    # comparison proves nothing, and silence here would be read as "checked, fine".
    GRANT_VOIDED=unreadable
  elif [ "$OLD_HASH" != "$NEW_HASH" ]; then
    GRANT_VOIDED=yes
  fi
fi

TCC_ERR=""
if [ "$GRANT_VOIDED" = yes ] || [ "$GRANT_VOIDED" = forced ]; then
  TCC_ERR=$(tccutil reset ListenEvent "$ID" 2>&1) || GRANT_VOIDED=failed
fi

# Leave exactly one bundle carrying this identifier. macOS registers every copy
# under the same one, so a second one makes the Input Monitoring row ambiguous
# about which app it is describing, and a STALE one is worse: the permission is
# keyed on the code hash, so an older copy's row can never match the installed
# app, and the symptom is a toggle that is switched on and does nothing.
#
# Never while something is running out of it, though. An app whose bundle is
# deleted underneath it keeps running but loses its icon, falling back to the
# generic placeholder in the Dock and in Command-Tab while the Finder still
# shows the real one. That looks exactly like a broken install and is not
# obviously self-inflicted.
if running_from "$SRC"; then
  echo
  echo "note: something is still running from $SRC, so it was left in place."
  echo "      quit it and re-run this to tidy up. Two bundles with one"
  echo "      identifier make the Input Monitoring row ambiguous."
else
  rm -rf "$SRC"
fi

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

if [ "$GRANT_VOIDED" = yes ]; then
  echo
  echo "The code changed, so the code hash changed, and an ad-hoc signature has"
  echo "nothing else to be identified by. Any Input Monitoring grant you had was"
  echo "for the previous build and would have kept its switch ON while being"
  echo "refused, so it has been cleared. Grant it again below."
elif [ "$GRANT_VOIDED" = forced ]; then
  echo
  echo "Cleared the Input Monitoring grant because you asked for it."
  echo "Grant it again below."
elif [ "$GRANT_VOIDED" = failed ]; then
  echo
  echo "This build has a different code hash from the one it replaced, so an"
  echo "existing Input Monitoring grant no longer matches it. Clearing that"
  echo "grant failed, and its switch will look ON and do nothing until it goes."
  echo "tccutil said:"
  echo "    $TCC_ERR"
  echo "Run this yourself, then grant it again:"
  echo "    tccutil reset ListenEvent $ID"
elif [ "$GRANT_VOIDED" = unreadable ]; then
  echo
  echo "Could not read the code hash of one of the bundles, so whether this"
  echo "install invalidated an existing Input Monitoring grant is unknown. If"
  echo "the switch is on and the app still reports no access, run:"
  echo "    tccutil reset ListenEvent $ID"
fi
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
echo "change does invalidate it, and the switch stays ON while it does, so this"
echo "script compares the two code hashes and clears the dead grant when it"
echo "replaces a different build. It cannot notice anything when the installed"
echo "copy is already this build, so if you are stuck with a switch that is on"
echo "and refused, ask for it directly:"
echo "  sh scripts/install.sh --reset-permission"
echo "or clear it by hand with:"
echo "  tccutil reset ListenEvent $ID"
echo
echo "Everything else works with no permission at all, and the app tells you"
echo "which measurements are blocked rather than reporting them as zero."
