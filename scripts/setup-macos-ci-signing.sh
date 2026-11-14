#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash "$ROOT_DIR/scripts/check-macos-signing-env.sh" --ci
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_OS:-}" == macOS ]] || {
  echo 'This keychain setup is only for ephemeral macOS GitHub Actions runners.' >&2
  exit 1
}

KEYCHAIN_PATH="${RUNNER_TEMP:?}/lighttranslate-signing.keychain-db"
CERT_PATH="$RUNNER_TEMP/lighttranslate-certificate.p12"
KEYCHAIN_PASSWORD="$(openssl rand -hex 32)"
trap 'rm -f "$CERT_PATH"' EXIT
printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$CERT_PATH"
chmod 600 "$CERT_PATH"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security import "$CERT_PATH" -k "$KEYCHAIN_PATH" -P "$APPLE_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
  -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH" >/dev/null
# The runner is disposable; do not change the developer's local keychain list.
security list-keychains -d user -s "$KEYCHAIN_PATH" "$HOME/Library/Keychains/login.keychain-db"
security find-identity -v -p codesigning "$KEYCHAIN_PATH" |
  grep -Fq "\"$APPLE_SIGNING_IDENTITY\""
