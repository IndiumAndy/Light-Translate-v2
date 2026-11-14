#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG_PATH="${1:?Usage: notarize-macos-dmg.sh installer.dmg}"
bash "$ROOT_DIR/scripts/check-macos-signing-env.sh"
RESULT_PATH="$(mktemp)"
trap 'rm -f "$RESULT_PATH"' EXIT

# Tauri notarizes/staples the .app; also notarize and staple the outer DMG so
# Gatekeeper can validate the exact file downloaded by the user, including offline.
if ! xcrun notarytool submit "$DMG_PATH" --apple-id "$APPLE_ID" \
  --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" \
  --wait --timeout "${LT_NOTARY_TIMEOUT:-2h}" --output-format json > "$RESULT_PATH"; then
  cat "$RESULT_PATH" >&2
  exit 1
fi
python3 - "$RESULT_PATH" <<'PY'
import json
import sys
with open(sys.argv[1]) as stream:
    result = json.load(stream)
if result.get('status') != 'Accepted':
    sys.exit(f"DMG notarization was not accepted: {result.get('status')} ({result.get('id')})")
print(f"DMG notarization accepted: {result['id']}")
PY
xcrun stapler staple "$DMG_PATH"
bash "$ROOT_DIR/scripts/verify-macos-release.sh" "$DMG_PATH" "$APPLE_TEAM_ID"
