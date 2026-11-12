#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Light Translate.app"
DEST_DIR="${LT_INSTALL_DIR:-$HOME/Applications}"
TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/Library/Caches/LightTranslateV2/target}"

# Pick a stable Apple Development identity from the login keychain so macOS TCC
# binds Accessibility & Screen Recording permissions to the certificate subject
# instead of an ephemeral ad-hoc CDHash that breaks on every rebuild.
SIGNING_ID="${APPLE_SIGNING_IDENTITY:-}"
if [[ -z "$SIGNING_ID" ]]; then
  SIGNING_ID="$(security find-identity -v -p codesigning | awk -F'"' '/Apple Development: Yisu Wang/ {print $2; exit}')"
fi
if [[ -z "$SIGNING_ID" ]]; then
  SIGNING_ID="$(security find-identity -v -p codesigning | awk -F'"' '/Apple Development:/ {print $2; exit}')"
fi

cd "$ROOT_DIR"
CARGO_TARGET_DIR="$TARGET_DIR" pnpm tauri build --bundles app

BUILT_APP="$TARGET_DIR/release/bundle/macos/$APP_NAME"
DEST_APP="$DEST_DIR/$APP_NAME"

pkill -x light-translate-v2 || true
sleep 0.4

mkdir -p "$DEST_DIR"
rm -rf "$DEST_APP"
ditto "$BUILT_APP" "$DEST_APP"
xattr -cr "$DEST_APP"

if [[ -n "$SIGNING_ID" ]]; then
  echo "Signing with stable identity: $SIGNING_ID"
  codesign --force --deep --options runtime --sign "$SIGNING_ID" "$DEST_APP"
else
  echo "Warning: No Apple Development certificate found; falling back to ad-hoc signing (TCC permissions will reset on rebuild)."
  codesign --force --deep --sign - "$DEST_APP"
fi

codesign --verify --deep --strict "$DEST_APP"
open "$DEST_APP"
echo "Installed and launched: $DEST_APP"
