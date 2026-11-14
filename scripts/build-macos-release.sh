#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
bash scripts/check-macos-signing-env.sh
# Stop before compiling when Apple still has a recent submission in progress.
python3 scripts/notarize_macos.py --require-idle
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/src-tauri/target}"
export CARGO_TARGET_DIR="$TARGET_DIR"
BUNDLE_DIR="$TARGET_DIR/universal-apple-darwin/release/bundle"
APP_PATH="$BUNDLE_DIR/macos/Light Translate.app"
ARCHIVE_PATH="$BUNDLE_DIR/macos/Light Translate.zip"
VERSION="$(python3 -c 'import json; print(json.load(open("src-tauri/tauri.conf.json"))["version"])')"
DMG_PATH="$BUNDLE_DIR/dmg/Light Translate_${VERSION}_universal.dmg"

# Let Tauri sign the app, then manage notarization separately. A transient info
# request must not discard the signed app or restart a completed upload.
env -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
  -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
  pnpm tauri build --target universal-apple-darwin --bundles app
codesign --verify --deep --strict --all-architectures "$APP_PATH"
ditto -c -k --keepParent "$APP_PATH" "$ARCHIVE_PATH"
python3 scripts/notarize_macos.py "$ARCHIVE_PATH" --staple "$APP_PATH"

# Package the already-stapled app without rebuilding or re-signing it. Apple's
# hdiutil preserves its sealed resources and stapled ticket inside the image.
STAGING_DIR="$(mktemp -d)"
trap 'rm -rf "$STAGING_DIR"' EXIT
ditto "$APP_PATH" "$STAGING_DIR/Light Translate.app"
ln -s /Applications "$STAGING_DIR/Applications"
mkdir -p "$BUNDLE_DIR/dmg"
hdiutil create -volname 'Light Translate' -srcfolder "$STAGING_DIR" \
  -fs HFS+ -format UDZO -ov "$DMG_PATH"
codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG_PATH"
codesign --verify --strict "$DMG_PATH"
