#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG_PATH="${1:?Usage: notarize-macos-dmg.sh installer.dmg}"
bash "$ROOT_DIR/scripts/check-macos-signing-env.sh"
# Only the app submission this attempt already saw Accepted may be ignored.
# Another run's newer upload still blocks this DMG.
APP_ARCHIVE="$(dirname "$DMG_PATH")/../macos/Light Translate.zip"
if [[ -f "$APP_ARCHIVE" ]]; then
  OWN_ID="$(python3 "$ROOT_DIR/scripts/notarize_macos.py" --print-matching-id "$APP_ARCHIVE")"
  if [[ -n "$OWN_ID" ]]; then
    export LT_NOTARY_OWN_SUBMISSIONS="$OWN_ID"
  fi
fi
python3 "$ROOT_DIR/scripts/notarize_macos.py" --require-idle
python3 "$ROOT_DIR/scripts/notarize_macos.py" "$DMG_PATH" --staple "$DMG_PATH"
bash "$ROOT_DIR/scripts/verify-macos-release.sh" "$DMG_PATH" "$APPLE_TEAM_ID"
