#!/usr/bin/env bash
set -euo pipefail

DMG_PATH="${1:?Usage: verify-macos-release.sh installer.dmg APPLE_TEAM_ID}"
TEAM_ID="${2:?Expected Apple developer team ID is required}"
[[ -f "$DMG_PATH" ]] || { echo "Missing installer: $DMG_PATH" >&2; exit 1; }
[[ "$TEAM_ID" =~ ^[A-Z0-9]{10}$ ]] || { echo 'Invalid Apple team ID' >&2; exit 1; }

verify_identity() {
  local path="$1" details
  codesign --verify --deep --strict --all-architectures "$path"
  details="$(codesign --display --verbose=4 "$path" 2>&1)"
  grep -Fqx "TeamIdentifier=$TEAM_ID" <<< "$details"
  grep -q '^Authority=Developer ID Application: ' <<< "$details"
  # Require Apple's Developer ID certificate chain, rather than trusting its name.
  codesign --verify --strict --all-architectures \
    -R='anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists' \
    "$path"
}

verify_identity "$DMG_PATH"
xcrun stapler validate "$DMG_PATH"
spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG_PATH"

MOUNT_DIR="$(mktemp -d)"
MOUNTED=false
cleanup() {
  if [[ "$MOUNTED" == true ]]; then
    hdiutil detach "$MOUNT_DIR" >/dev/null || true
  fi
  rmdir "$MOUNT_DIR" 2>/dev/null || true
}
trap cleanup EXIT
hdiutil attach -readonly -nobrowse -mountpoint "$MOUNT_DIR" "$DMG_PATH" >/dev/null
MOUNTED=true
[[ -L "$MOUNT_DIR/Applications" && "$(readlink "$MOUNT_DIR/Applications")" == /Applications ]] || {
  echo 'Installer is missing the Applications folder link.' >&2
  exit 1
}
APP_PATH="$MOUNT_DIR/Light Translate.app"
[[ -d "$APP_PATH" ]] || { echo 'Installer does not contain Light Translate.app' >&2; exit 1; }
verify_identity "$APP_PATH"

APP_ID="$(python3 - "$APP_PATH/Contents/Info.plist" <<'PY'
import plistlib
import sys
with open(sys.argv[1], 'rb') as stream:
    print(plistlib.load(stream)['CFBundleIdentifier'])
PY
)"
[[ "$APP_ID" == com.atat.lighttranslate.v2 ]] || { echo "Unexpected bundle ID: $APP_ID" >&2; exit 1; }
REQUIREMENT="$(codesign --display --requirements - "$APP_PATH" 2>&1)"
DESIGNATED="$(grep '^designated =>' <<< "$REQUIREMENT")"
if grep -qi cdhash <<< "$DESIGNATED"; then
  echo 'Unstable designated requirement: TCC permissions would be tied to this build.' >&2
  exit 1
fi
grep -Fq 'identifier "com.atat.lighttranslate.v2"' <<< "$DESIGNATED"

APP_EXECUTABLE="$APP_PATH/Contents/MacOS/light-translate-v2"
lipo -verify_arch arm64 x86_64 "$APP_EXECUTABLE"
for arch in arm64 x86_64; do
  DETAILS="$(codesign --display --verbose=4 --arch "$arch" "$APP_PATH" 2>&1)"
  grep -Eq '^CodeDirectory .*flags=.*\(.*runtime.*\)' <<< "$DETAILS"
  grep -q '^Timestamp=' <<< "$DETAILS"
done
xcrun stapler validate "$APP_PATH"
spctl --assess --type execute --verbose=4 "$APP_PATH"
printf 'Verified Developer ID, team %s, stable app identity, hardened runtime, both architectures, notarization and Gatekeeper.\n' "$TEAM_ID"
