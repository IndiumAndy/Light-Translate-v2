#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG_PATH="${1:?Usage: notarize-macos-dmg.sh installer.dmg}"
bash "$ROOT_DIR/scripts/check-macos-signing-env.sh"
# A retry must query the saved submission instead of uploading another DMG.
python3 "$ROOT_DIR/scripts/notarize_macos.py" --require-idle
python3 "$ROOT_DIR/scripts/notarize_macos.py" "$DMG_PATH" --staple "$DMG_PATH"
bash "$ROOT_DIR/scripts/verify-macos-release.sh" "$DMG_PATH" "$APPLE_TEAM_ID"
