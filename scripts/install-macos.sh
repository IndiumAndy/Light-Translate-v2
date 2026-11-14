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
if [[ -z "$SIGNING_ID" || "$SIGNING_ID" == "-" ]]; then
  echo 'A stable Apple signing certificate is required. Install an Apple Development certificate or set APPLE_SIGNING_IDENTITY.' >&2
  exit 1
fi
security find-identity -v -p codesigning | grep -Fq "\"$SIGNING_ID\"" || {
  echo 'The requested signing identity is not available in the keychain.' >&2
  exit 1
}

cd "$ROOT_DIR"
APPLE_SIGNING_IDENTITY="$SIGNING_ID" CARGO_TARGET_DIR="$TARGET_DIR" pnpm tauri build --bundles app

BUILT_APP="$TARGET_DIR/release/bundle/macos/$APP_NAME"
DEST_APP="$DEST_DIR/$APP_NAME"

# Verify the built bundle before replacing an installed application. Tauri signs
# nested code and the final bundle; do not re-sign it after copying.
codesign --verify --deep --strict "$BUILT_APP"
REQUIREMENT="$(codesign --display --requirements - "$BUILT_APP" 2>&1)"
if grep '^designated =>' <<< "$REQUIREMENT" | grep -qi cdhash; then
  echo 'Refusing to install a build with an unstable signing requirement.' >&2
  exit 1
fi

pkill -x light-translate-v2 || true
sleep 0.4

mkdir -p "$DEST_DIR"
rm -rf "$DEST_APP"
ditto "$BUILT_APP" "$DEST_APP"

codesign --verify --deep --strict "$DEST_APP"
open "$DEST_APP"
echo "Installed and launched: $DEST_APP"
