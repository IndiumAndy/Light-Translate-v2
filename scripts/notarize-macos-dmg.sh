#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG_PATH="${1:?Usage: notarize-macos-dmg.sh installer.dmg}"
bash "$ROOT_DIR/scripts/check-macos-signing-env.sh"
# Preserve the ID and archive checksum so a failed network query can resume
# without uploading/re-signing the same installer again.
python3 "$ROOT_DIR/scripts/notarize_macos.py" "$DMG_PATH" --staple "$DMG_PATH"
bash "$ROOT_DIR/scripts/verify-macos-release.sh" "$DMG_PATH" "$APPLE_TEAM_ID"
