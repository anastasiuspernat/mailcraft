#!/usr/bin/env bash
# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build MailCraft for iPad (apps/mailcraft-ios) and pack it as MailCraft.app.
#
# Usage: packaging/ios/build.sh [--sim | --device] [--run]
#   --sim (default)  the iPad simulator of this Mac (x86_64 on an Intel Mac, arm64 on Apple silicon)
#   --device         a real iPad (arm64); signing it is not done here yet
#   --run            install it in the booted iPad simulator (booting one if none is) and start it
# Env: MAILCRAFT_IOS_BUNDLE_ID (default crispy.pixels.mailcraft),
#      MAILCRAFT_IOS_SIMULATOR (a simulator's name or id; default the first iPad Pro 13-inch).
# Output: target/ios/<target>/release/MailCraft.app
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KIND=sim
RUN=0
for arg in "$@"; do
  case "$arg" in
    --sim) KIND=sim ;;
    --device) KIND=device ;;
    --run) RUN=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [[ $KIND == device ]]; then
  TARGET=aarch64-apple-ios; PLATFORM=iPhoneOS
elif [[ $(uname -m) == arm64 ]]; then
  TARGET=aarch64-apple-ios-sim; PLATFORM=iPhoneSimulator
else
  TARGET=x86_64-apple-ios; PLATFORM=iPhoneSimulator
fi
[[ $RUN == 1 && $KIND == device ]] && { echo "--run is for the simulator" >&2; exit 2; }

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
BUNDLE_ID="${MAILCRAFT_IOS_BUNDLE_ID:-crispy.pixels.mailcraft}"
export CARGO_TARGET_DIR="$ROOT/target/ios"

echo "==> building mailcraft-ios for $TARGET"
cargo build --release -p mailcraft-ios --target "$TARGET"

APP="$CARGO_TARGET_DIR/$TARGET/release/MailCraft.app"
rm -rf "$APP"
mkdir -p "$APP"
cp "$CARGO_TARGET_DIR/$TARGET/release/mailcraft-ios" "$APP/mailcraft-ios"
BUILD="${MAILCRAFT_IOS_BUILD:-1}"
sed -e "s/@BUNDLE_ID@/$BUNDLE_ID/" -e "s/@VERSION@/$VERSION/" -e "s/@BUILD@/$BUILD/" -e "s/@PLATFORM@/$PLATFORM/" \
  "$ROOT/packaging/ios/Info.plist.in" > "$APP/Info.plist"

# The app icon, from the project's (assets/app-icon/mailcraft-1024.png), without its transparent
# corners (the App Store takes no icon with an alpha channel), compiled into Assets.car.
ICONS="$CARGO_TARGET_DIR/ios-assets"
rm -rf "$ICONS" && mkdir -p "$ICONS/Assets.xcassets/AppIcon.appiconset"
sips -s format jpeg "$ROOT/assets/app-icon/mailcraft-1024.png" --out "$ICONS/flat.jpg" >/dev/null
sips -s format png "$ICONS/flat.jpg" --out "$ICONS/Assets.xcassets/AppIcon.appiconset/icon-1024.png" >/dev/null
cat > "$ICONS/Assets.xcassets/Contents.json" <<'JSON'
{ "info" : { "author" : "xcode", "version" : 1 } }
JSON
cat > "$ICONS/Assets.xcassets/AppIcon.appiconset/Contents.json" <<'JSON'
{ "images" : [ { "filename" : "icon-1024.png", "idiom" : "universal", "platform" : "ios", "size" : "1024x1024" } ],
  "info" : { "author" : "xcode", "version" : 1 } }
JSON
SDK_NAME=$([[ $PLATFORM == iPhoneOS ]] && echo iphoneos || echo iphonesimulator)
xcrun actool "$ICONS/Assets.xcassets" --compile "$APP" --platform "$SDK_NAME" --minimum-deployment-target 16.0 \
  --target-device ipad --app-icon AppIcon --output-partial-info-plist "$ICONS/partial.plist" >/dev/null
# The icon's entries, and what Xcode itself writes into an app's Info.plist (the App Store asks).
/usr/libexec/PlistBuddy -c "Merge $ICONS/partial.plist" "$APP/Info.plist" >/dev/null
SDK_VERSION="$(xcrun --sdk "$SDK_NAME" --show-sdk-version)"
SDK_BUILD="$(xcrun --sdk "$SDK_NAME" --show-sdk-build-version)"
XCODE="$(xcodebuild -version | awk '/Xcode/ {print $2}')"
XCODE_BUILD="$(xcodebuild -version | awk '/Build version/ {print $3}')"
XCODE_NUM="$(printf '%s' "$XCODE" | awk -F. '{printf "%d%d%d", $1, $2, ($3 == "" ? 0 : $3)}')"
for kv in "DTPlatformName:string:$SDK_NAME" "DTPlatformVersion:string:$SDK_VERSION" "DTPlatformBuild:string:$SDK_BUILD" \
          "DTSDKName:string:$SDK_NAME$SDK_VERSION" "DTSDKBuild:string:$SDK_BUILD" "DTXcode:string:$XCODE_NUM" \
          "DTXcodeBuild:string:$XCODE_BUILD" "DTCompiler:string:com.apple.compilers.llvm.clang.1_0" \
          "BuildMachineOSBuild:string:$(sw_vers -buildVersion)"; do
  IFS=: read -r key type value <<<"$kv"
  /usr/libexec/PlistBuddy -c "Add :$key $type $value" "$APP/Info.plist" >/dev/null
done
plutil -lint "$APP/Info.plist" >/dev/null
echo "==> $APP"

if [[ $RUN == 1 ]]; then
  SIM="${MAILCRAFT_IOS_SIMULATOR:-}"
  if [[ -z $SIM ]]; then
    SIM="$(xcrun simctl list devices booted | sed -n 's/.*iPad.*(\([0-9A-F-]\{36\}\)) (Booted).*/\1/p' | head -1)"
  fi
  if [[ -z $SIM ]]; then
    SIM="$(xcrun simctl list devices available | sed -n 's/.*iPad Pro 13-inch.*(\([0-9A-F-]\{36\}\)) (Shutdown).*/\1/p' | head -1)"
    echo "==> booting simulator $SIM"
    xcrun simctl boot "$SIM"
    open -a Simulator
  fi
  xcrun simctl install "$SIM" "$APP"
  xcrun simctl launch "$SIM" "$BUNDLE_ID"
fi
