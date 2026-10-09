#!/bin/bash
set -euo pipefail
umask 077

: "${KURU_SIGNING_BINARY:?missing private signing executable}"
: "${MACOS_SIGNING_P12_BASE64:?missing signing certificate}"
: "${MACOS_SIGNING_P12_PASSWORD:?missing certificate password}"
: "${MACOS_SIGNING_IDENTITY:?missing Developer ID Application identity}"
: "${APPLE_NOTARY_KEY_P8:?missing notarization Team API key}"
: "${APPLE_NOTARY_KEY_ID:?missing notarization key ID}"
: "${APPLE_NOTARY_ISSUER_ID:?missing notarization issuer ID}"

signing_tmp=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/kuru-signing.XXXXXX")
keychain="$signing_tmp/signing.keychain-db"
cleanup() {
  status=$?
  trap - EXIT
  if [ -f "$keychain" ]; then
    security delete-keychain "$keychain" || status=1
  fi
  rm -rf -- "$signing_tmp"
  exit "$status"
}
trap cleanup EXIT

keychain_password=$(openssl rand -hex 32)
printf '%s' "$MACOS_SIGNING_P12_BASE64" | /usr/bin/base64 -D > "$signing_tmp/identity.p12"
printf '%s' "$APPLE_NOTARY_KEY_P8" > "$signing_tmp/AuthKey.p8"
security create-keychain -p "$keychain_password" "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$signing_tmp/identity.p12" -k "$keychain" \
  -P "$MACOS_SIGNING_P12_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$keychain"
codesign --force --options runtime --timestamp --keychain "$keychain" \
  --sign "$MACOS_SIGNING_IDENTITY" "$KURU_SIGNING_BINARY"
codesign --verify --strict --verbose=2 "$KURU_SIGNING_BINARY"
ditto -c -k --keepParent "$KURU_SIGNING_BINARY" "$signing_tmp/notarization.zip"
xcrun notarytool submit "$signing_tmp/notarization.zip" \
  --key "$signing_tmp/AuthKey.p8" --key-id "$APPLE_NOTARY_KEY_ID" \
  --issuer "$APPLE_NOTARY_ISSUER_ID" --wait --timeout 20m \
  --output-format json > "$signing_tmp/result.json"
jq -e '.status == "Accepted" and (.id | type == "string" and length > 0)' \
  "$signing_tmp/result.json" >/dev/null
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  printf 'Apple notarization accepted: %s\n' "$(jq -r '.id' "$signing_tmp/result.json")" >> "$GITHUB_STEP_SUMMARY"
fi
