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
#   IOS_NSE_PROVISIONING_PROFILE  base64 of the one for its notification extension (<app id>.notifications)
#   ASC_KEY_ID, ASC_ISSUER_ID  an App Store Connect API key's id and issuer
#   ASC_KEY_P8                 base64 of that key (.p8)
# And, for an Ad Hoc build as well (a year on the devices the profiles name, installed from a Mac):
#   IOS_ADHOC_PROFILE, IOS_NSE_ADHOC_PROFILE  base64 of the Ad Hoc profiles for the app and its extension
#   ADHOC_PASSPHRASE           what the Ad Hoc .ipa is encrypted with (it names the devices)
#   ADHOC_OUT                  the folder MailCraft-adhoc.ipa.enc goes to
# Without them it says what is missing and stops without failing (a fork, or before they're set).
set -euo pipefail

APP="${1:?usage: sign-and-upload.sh <MailCraft.app>}"
missing=()
for v in IOS_DIST_CERT_P12 IOS_DIST_CERT_PASSWORD IOS_PROVISIONING_PROFILE IOS_NSE_PROVISIONING_PROFILE ASC_KEY_ID ASC_ISSUER_ID ASC_KEY_P8; do
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

# A bundle's provisioning profile, inside it, and the entitlements it grants: `sign <bundle>
# <profile base64> <name>`. Both bundles get one keychain access group, the app's own id, first
# (the profiles allow `<team>.*`): the notification extension reads the note key MailCraft keeps
# there (apps/mailcraft-ios/src/push_keys.rs), and the app's own items stay where they were.
APP_BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Info.plist")"
# (Unsigned, for the Ad Hoc build.)
mkdir -p "$WORK/unsigned"
cp -R "$APP" "$WORK/unsigned/"
sign() {
  local bundle="$1" profile="$2" name="$3"
  printf '%s' "$profile" | base64 --decode > "$WORK/$name.mobileprovision"
  cp "$WORK/$name.mobileprovision" "$bundle/embedded.mobileprovision"
  security cms -D -i "$WORK/$name.mobileprovision" > "$WORK/$name.plist"
  /usr/libexec/PlistBuddy -x -c 'Print :Entitlements' "$WORK/$name.plist" > "$WORK/$name-entitlements.plist"
  local profile_id bundle_id team
  profile_id="$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:application-identifier' "$WORK/$name.plist")"
  bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$bundle/Info.plist")"
  [[ "$profile_id" == *".$bundle_id" ]] || { echo "error: the profile is for $profile_id, the bundle is $bundle_id" >&2; exit 1; }
  team="$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:com.apple.developer.team-identifier' "$WORK/$name.plist")"
  /usr/libexec/PlistBuddy -c 'Delete :keychain-access-groups' "$WORK/$name-entitlements.plist" >/dev/null 2>&1 || true
  /usr/libexec/PlistBuddy -c 'Add :keychain-access-groups array' -c "Add :keychain-access-groups:0 string $team.$APP_BUNDLE_ID" "$WORK/$name-entitlements.plist"
  codesign --force --sign "$IDENTITY" --keychain "$KEYCHAIN" --entitlements "$WORK/$name-entitlements.plist" --generate-entitlement-der "$bundle"
}
# The extension first: the app's signature covers what is inside it.
sign "$APP/PlugIns/MailCraftNotify.appex" "$IOS_NSE_PROVISIONING_PROFILE" notify
sign "$APP" "$IOS_PROVISIONING_PROFILE" app
codesign --verify --strict --deep --verbose=2 "$APP"

# The Ad Hoc build: the same app, signed with the Ad Hoc profiles, encrypted (its profile names
# the owner's devices) for the workflow to keep as an artifact.
if [[ -n "${IOS_ADHOC_PROFILE:-}" && -n "${IOS_NSE_ADHOC_PROFILE:-}" && -n "${ADHOC_PASSPHRASE:-}" && -n "${ADHOC_OUT:-}" ]]; then
  ADHOC="$WORK/unsigned/$(basename "$APP")"
  sign "$ADHOC/PlugIns/MailCraftNotify.appex" "$IOS_NSE_ADHOC_PROFILE" adhoc-notify
  sign "$ADHOC" "$IOS_ADHOC_PROFILE" adhoc-app
  codesign --verify --strict --deep --verbose=2 "$ADHOC"
  rm -rf "$WORK/Payload" && mkdir -p "$WORK/Payload" "$ADHOC_OUT"
  cp -R "$ADHOC" "$WORK/Payload/"
  (cd "$WORK" && ditto -c -k --sequesterRsrc --keepParent Payload MailCraft-adhoc.ipa)
  openssl enc -aes-256-cbc -pbkdf2 -iter 600000 -salt -in "$WORK/MailCraft-adhoc.ipa" -out "$ADHOC_OUT/MailCraft-adhoc.ipa.enc" -pass env:ADHOC_PASSPHRASE
  echo "made the Ad Hoc build"
else
  echo "no Ad Hoc build: its profiles or passphrase aren't set"
fi

# The .ipa: the app in a Payload folder, zipped.
rm -rf "$WORK/Payload" && mkdir -p "$WORK/Payload"
cp -R "$APP" "$WORK/Payload/"
(cd "$WORK" && ditto -c -k --sequesterRsrc --keepParent Payload MailCraft.ipa)

# Up to App Store Connect, for TestFlight, with the API key (kept where altool looks for it).
mkdir -p "$HOME/.appstoreconnect/private_keys"
printf '%s' "$ASC_KEY_P8" | base64 --decode > "$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8"
xcrun altool --upload-app --type ios --file "$WORK/MailCraft.ipa" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
rm -f "$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8"
echo "uploaded MailCraft $(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Info.plist") ($(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP/Info.plist")) to TestFlight"
