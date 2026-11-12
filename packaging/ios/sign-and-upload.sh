#!/usr/bin/env bash
# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# CI only: sign the iPad app a build job made (never built or run here) and upload it to
# TestFlight. Usage: packaging/ios/sign-and-upload.sh <MailCraft.app>
#
# Inputs (env, from the repository's secrets):
#   IOS_DIST_CERT_P12          base64 of the "Apple Distribution" certificate with its key (.p12)
#   IOS_DIST_CERT_PASSWORD     its password
#   IOS_PROVISIONING_PROFILE   base64 of the App Store provisioning profile for the app's id
#   ASC_KEY_ID, ASC_ISSUER_ID  an App Store Connect API key's id and issuer
#   ASC_KEY_P8                 base64 of that key (.p8)
# Without them it says what is missing and stops without failing (a fork, or before they're set).
set -euo pipefail

APP="${1:?usage: sign-and-upload.sh <MailCraft.app>}"
missing=()
for v in IOS_DIST_CERT_P12 IOS_DIST_CERT_PASSWORD IOS_PROVISIONING_PROFILE ASC_KEY_ID ASC_ISSUER_ID ASC_KEY_P8; do
  [[ -n "${!v:-}" ]] || missing+=("$v")
done
if (( ${#missing[@]} )); then
  echo "::warning::not signed or uploaded: these secrets aren't set: ${missing[*]}"
  exit 0
fi

WORK="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/mailcraft-ios-sign"
rm -rf "$WORK" && mkdir -p "$WORK"
cleanup() { security delete-keychain "$WORK/sign.keychain-db" >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT

# The certificate, in a keychain of its own for this run.
KC_PASS="$(openssl rand -hex 24)"
KEYCHAIN="$WORK/sign.keychain-db"
security create-keychain -p "$KC_PASS" "$KEYCHAIN"
security set-keychain-settings -lut 3600 "$KEYCHAIN"
security unlock-keychain -p "$KC_PASS" "$KEYCHAIN"
printf '%s' "$IOS_DIST_CERT_P12" | base64 --decode > "$WORK/cert.p12"
security import "$WORK/cert.p12" -P "$IOS_DIST_CERT_PASSWORD" -T /usr/bin/codesign -t cert -f pkcs12 -k "$KEYCHAIN"
rm -f "$WORK/cert.p12"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KC_PASS" "$KEYCHAIN" >/dev/null
# shellcheck disable=SC2046
security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
IDENTITY="$(security find-identity -v -p codesigning "$KEYCHAIN" | awk '/Apple Distribution/ { print $2; exit }')"
[[ -n "$IDENTITY" ]] || { echo "error: no 'Apple Distribution' identity in IOS_DIST_CERT_P12" >&2; exit 1; }

# The provisioning profile, inside the app, and the entitlements it grants.
printf '%s' "$IOS_PROVISIONING_PROFILE" | base64 --decode > "$WORK/profile.mobileprovision"
cp "$WORK/profile.mobileprovision" "$APP/embedded.mobileprovision"
security cms -D -i "$WORK/profile.mobileprovision" > "$WORK/profile.plist"
/usr/libexec/PlistBuddy -x -c 'Print :Entitlements' "$WORK/profile.plist" > "$WORK/entitlements.plist"
PROFILE_APP_ID="$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:application-identifier' "$WORK/profile.plist")"
BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Info.plist")"
[[ "$PROFILE_APP_ID" == *".$BUNDLE_ID" ]] || { echo "error: the profile is for $PROFILE_APP_ID, the app is $BUNDLE_ID" >&2; exit 1; }

codesign --force --sign "$IDENTITY" --keychain "$KEYCHAIN" --entitlements "$WORK/entitlements.plist" --generate-entitlement-der "$APP"
codesign --verify --strict --verbose=2 "$APP"

# The .ipa: the app in a Payload folder, zipped.
mkdir -p "$WORK/Payload"
cp -R "$APP" "$WORK/Payload/"
(cd "$WORK" && ditto -c -k --sequesterRsrc --keepParent Payload MailCraft.ipa)

# Up to App Store Connect, for TestFlight, with the API key (kept where altool looks for it).
mkdir -p "$HOME/.appstoreconnect/private_keys"
printf '%s' "$ASC_KEY_P8" | base64 --decode > "$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8"
xcrun altool --upload-app --type ios --file "$WORK/MailCraft.ipa" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
rm -f "$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8"
echo "uploaded MailCraft $(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Info.plist") ($(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP/Info.plist")) to TestFlight"
