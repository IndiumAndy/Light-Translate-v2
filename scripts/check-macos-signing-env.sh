#!/usr/bin/env bash
set -euo pipefail

# Run before an official build. Never silently publish an unsigned fallback.
required=(APPLE_SIGNING_IDENTITY APPLE_TEAM_ID APPLE_ID APPLE_PASSWORD)
if [[ "${1:-}" == "--ci" ]]; then
  required+=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD)
fi
missing=()
for name in "${required[@]}"; do
  [[ -n "${!name:-}" ]] || missing+=("$name")
done
if (( ${#missing[@]} )); then
  printf 'Missing macOS release credentials: %s\n' "${missing[*]}" >&2
  exit 1
fi
if [[ "$APPLE_SIGNING_IDENTITY" != "Developer ID Application: "* ]]; then
  echo 'Public macOS releases require a Developer ID Application identity.' >&2
  exit 1
fi
if [[ ! "$APPLE_TEAM_ID" =~ ^[A-Z0-9]{10}$ ]] ||
   [[ "$APPLE_SIGNING_IDENTITY" != *" ($APPLE_TEAM_ID)" ]]; then
  echo 'APPLE_TEAM_ID must match the team in APPLE_SIGNING_IDENTITY.' >&2
  exit 1
fi
