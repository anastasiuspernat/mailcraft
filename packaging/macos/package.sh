#!/usr/bin/env bash
# Build, sign and (optionally) notarize the macOS release artifacts:
#
#   $DIST/mailcraft-<version>-macos-<arch>.dmg          MailCraft.app on a drag-to-Applications DMG
#   $DIST/mailcraft-cli-<version>-macos-<arch>.zip      the headless CLI
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build | --build-only] [--no-run]
#                                   [--ai cpu|gpu|vulkan|metal] [--bundle-model <model.gguf>]
#
# --ai: the smart Inbox's model engine. `cpu` (default) runs on the processor everywhere.
#   `gpu` gives each architecture its graphics card: Intel through Vulkan and MoltenVK, Apple
#   silicon through Metal; with --arch universal (the default) one app holds both, each slice
#   built with its own features and joined with lipo.
#   `vulkan` (Intel, --arch x86_64) and `metal` (Apple silicon, --arch aarch64) build one of them.
#   For Vulkan the loader and MoltenVK (>= 1.4.2, from Homebrew's molten-vk and vulkan-loader, or
#   $VULKAN_SDK) are copied into Contents/Frameworks with the driver file in
#   Contents/Resources/vulkan/icd.d, so the app needs nothing installed. Metal is the system's.
# --bundle-model: put a GGUF model in Contents/Resources/models/ (about 2.5 GB for Qwen3-4B).
#
# --no-run: don't run the built CLI at the end (the job that signs never runs what was built).
#
# Signing (env):
#   MACOS_SIGN_IDENTITY   codesign identity (name or SHA-1). Default "-" = ad-hoc (local testing;
#                         Gatekeeper will reject the result on other Macs). In CI, import-cert.sh sets it.
#   MACOS_KEYCHAIN        keychain holding the identity (optional)
# Notarization (env; all three needed, and a real identity):
#   APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"

ARCH=universal
SKIP_BUILD=0
BUILD_ONLY=0
NO_RUN=0
AI=cpu
BUNDLE_MODEL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --build-only) BUILD_ONLY=1; shift ;;
    --no-run) NO_RUN=1; shift ;;
    --ai) AI="$2"; shift 2 ;;
    --bundle-model) BUNDLE_MODEL="$2"; shift 2 ;;
    -h | --help) sed -n '2,27p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac
# The model engine for each target: its cargo features and whether Metal is built
# (`features_for <target>` sets FEATURES and METAL). One backend on the card: Metal beside Vulkan
# froze an Intel Mac with an AMD card, so an Intel slice never builds Metal.
case "$AI" in
  cpu | gpu) ;;
  vulkan) [[ $ARCH == x86_64 ]] || { echo "--ai vulkan is for Intel Macs: use --arch x86_64 (or --ai gpu)" >&2; exit 2; } ;;
  metal) [[ $ARCH == aarch64 ]] || { echo "--ai metal is for Apple silicon: use --arch aarch64 (or --ai gpu)" >&2; exit 2; } ;;
  *) echo "unknown --ai $AI" >&2; exit 2 ;;
esac
features_for() {
  FEATURES=()
  METAL=OFF
  [[ $AI == cpu ]] && return 0
  case "$1" in
    x86_64-apple-darwin) FEATURES=(--features ai-vulkan) ;;
    aarch64-apple-darwin) FEATURES=(--features ai-metal); METAL=ON ;;
  esac
}
# Whether the Intel slice runs on the graphics card (and so the bundle carries MoltenVK).
VULKAN=0
if [[ $AI != cpu ]] && [[ " ${TARGETS[*]} " == *" x86_64-apple-darwin "* ]]; then VULKAN=1; fi
if [[ -n "$BUNDLE_MODEL" ]]; then
  [[ -f "$BUNDLE_MODEL" ]] || { echo "--bundle-model: no such file: $BUNDLE_MODEL" >&2; exit 2; }
  [[ "$(head -c 4 "$BUNDLE_MODEL")" == "GGUF" ]] || { echo "--bundle-model: not a GGUF model: $BUNDLE_MODEL" >&2; exit 2; }
fi
if [[ $SKIP_BUILD == 1 ]] && [[ $BUILD_ONLY == 1 ]]; then
  echo "--skip-build and --build-only can't be used together" >&2
  exit 2
fi

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/MailCraft.app"
DMG="$DIST/mailcraft-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/mailcraft-cli-$VERSION-macos-$ARCH.zip"

NOTARIZE=0
if [[ $IDENTITY == "-" ]]; then
  warn "macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); artifacts are not notarized"
elif [[ -n "${APPLE_ID:-}" ]] && [[ -n "${APPLE_PASSWORD:-}" ]] && [[ -n "${APPLE_TEAM_ID:-}" ]]; then
  NOTARIZE=1
else
  warn "macOS: APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID incomplete; signed but not notarized"
fi

echo "==> MailCraft $VERSION for macOS ($ARCH, AI: $AI), identity: $IDENTITY, notarize: $NOTARIZE"

# ---- build -------------------------------------------------------------------------------------
if [[ $SKIP_BUILD == 0 ]]; then
  for t in "${TARGETS[@]}"; do
    features_for "$t"
    echo "==> building $t (${FEATURES[*]:-cpu}, Metal $METAL)"
    (cd "$ROOT" && GGML_METAL=$METAL cargo build --release --locked -p mailcraft --target "$t" ${FEATURES[@]+"${FEATURES[@]}"})
    (cd "$ROOT" && cargo build --release --locked -p mailcraft-cli --target "$t")
  done
fi

# CI compiles in a job of its own (--build-only), which has no signing secret, and hands the
# binaries by digest to a job that signs and packages them (--skip-build --no-run) without
# running cargo or them.
if [[ $BUILD_ONLY == 1 ]]; then
  echo "==> built; signing and packaging come next"
  exit 0
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin"
for bin in mailcraft mailcraft-cli; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$bin"); done
  lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  lipo -info "$WORK/bin/$bin"
done

# ---- signing helpers ---------------------------------------------------------------------------
sign() {
  # Hardened runtime + secure timestamp for a real identity; ad-hoc can't be timestamped.
  local ts=(--timestamp)
  [[ $IDENTITY == "-" ]] && ts=(--timestamp=none)
  local kc=()
  [[ -n "${MACOS_KEYCHAIN:-}" ]] && kc=(--keychain "$MACOS_KEYCHAIN")
  codesign --force --sign "$IDENTITY" ${kc[@]+"${kc[@]}"} "${ts[@]}" "$@"
}

notarize() {
  local file="$1" out id status
  echo "==> notarizing $(basename "$file") (this can take a few minutes)"
  out="$(xcrun notarytool submit "$file" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --timeout 1h --output-format json)" || true
  echo "$out"
  id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
  status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
  if [[ $status != "Accepted" ]]; then
    if [[ -n "$id" ]]; then
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" || true
    fi
    echo "error: notarization of $(basename "$file") failed (status: ${status:-unknown})" >&2
    exit 1
  fi
}

# ---- MailCraft.app ----------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Executable and icon carry the display name (CFBundleExecutable / CFBundleIconFile).
cp "$WORK/bin/mailcraft" "$APP/Contents/MacOS/MailCraft"
cp "$ROOT/assets/app-icon/mailcraft.icns" "$APP/Contents/Resources/MailCraft.icns"
# Licences of the embedded craft-fonts fonts (only when built with CRAFT_FONTS_DIR).
copy_font_licences "$APP/Contents/Resources"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${MAILCRAFT_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

ENTITLEMENTS="$HERE/entitlements.plist"
if [[ $VULKAN == 1 ]]; then
  # The Vulkan loader and MoltenVK, inside the bundle: no Homebrew or Vulkan SDK on the Mac that
  # runs it. The worker is pointed at the driver file by VK_ICD_FILENAMES (apps/mailcraft's
  # ai_runner.rs), which the loader reads before its own search.
  VK_PREFIX="${VULKAN_SDK:-}"
  MVK_LIB="${VK_PREFIX:+$VK_PREFIX/lib/libMoltenVK.dylib}"
  LOADER_LIB="${VK_PREFIX:+$VK_PREFIX/lib/libvulkan.1.dylib}"
  if [[ -z "$VK_PREFIX" ]]; then
    MVK_LIB="$(brew --prefix molten-vk)/lib/libMoltenVK.dylib"
    LOADER_LIB="$(brew --prefix vulkan-loader)/lib/libvulkan.1.dylib"
    MVK_VERSION="$(brew list --versions molten-vk | awk '{print $2}')"
    # 1.4.2 reports AMD's subgroup size as Metal's (32); older MoltenVK computes garbage there.
    if [[ "$(printf '%s\n1.4.2\n' "$MVK_VERSION" | sort -V | head -1)" != "1.4.2" ]]; then
      echo "MoltenVK $MVK_VERSION is too old: brew upgrade molten-vk (1.4.2 or newer)" >&2
      exit 1
    fi
  fi
  for lib in "$MVK_LIB" "$LOADER_LIB"; do
    [[ -f "$lib" ]] || { echo "missing $lib (brew install molten-vk vulkan-loader)" >&2; exit 1; }
  done
  mkdir -p "$APP/Contents/Frameworks" "$APP/Contents/Resources/vulkan/icd.d"
  cp -L "$MVK_LIB" "$APP/Contents/Frameworks/libMoltenVK.dylib"
  cp -L "$LOADER_LIB" "$APP/Contents/Frameworks/libvulkan.1.dylib"
  chmod u+w "$APP/Contents/Frameworks/"*.dylib
  install_name_tool -id "@rpath/libMoltenVK.dylib" "$APP/Contents/Frameworks/libMoltenVK.dylib"
  install_name_tool -id "@rpath/libvulkan.1.dylib" "$APP/Contents/Frameworks/libvulkan.1.dylib"
  # The executable finds the loader in Frameworks, wherever it was linked from.
  linked="$(otool -L "$APP/Contents/MacOS/MailCraft" | awk '/libvulkan/ {print $1; exit}')"
  [[ -n "$linked" ]] && install_name_tool -change "$linked" "@rpath/libvulkan.1.dylib" "$APP/Contents/MacOS/MailCraft"
  install_name_tool -add_rpath "@executable_path/../Frameworks" "$APP/Contents/MacOS/MailCraft"
  cat >"$APP/Contents/Resources/vulkan/icd.d/MoltenVK_icd.json" <<'ICD'
{
    "file_format_version": "1.0.0",
    "ICD": {
        "library_path": "../../../Frameworks/libMoltenVK.dylib",
        "api_version": "1.4.0",
        "is_portability_driver": true
    }
}
ICD
  echo "==> bundled: $(shasum -a 256 "$APP/Contents/Frameworks/libMoltenVK.dylib")"
  echo "==> bundled: $(shasum -a 256 "$APP/Contents/Frameworks/libvulkan.1.dylib")"
  # Both are Apache-2.0: their licences travel with them, and a build without them stops.
  licence() {
    local from
    for from in "$@"; do [[ -f "$from" ]] && { echo "$from"; return 0; }; done
    return 1
  }
  MVK_LICENCE="$(licence "$(dirname "$MVK_LIB")/../LICENSE" "${VK_PREFIX:+$VK_PREFIX/share/doc/MoltenVK/LICENSE}")" ||
    { echo "MoltenVK's licence wasn't found beside $MVK_LIB" >&2; exit 1; }
  LOADER_LICENCE="$(licence "$(dirname "$LOADER_LIB")/../LICENSE.txt" "$(dirname "$LOADER_LIB")/../LICENSE" "${VK_PREFIX:+$VK_PREFIX/share/doc/vulkan-loader/LICENSE.txt}")" ||
    { echo "the Vulkan loader's licence wasn't found beside $LOADER_LIB" >&2; exit 1; }
  cp "$MVK_LICENCE" "$APP/Contents/Resources/vulkan/MoltenVK-LICENSE.txt"
  cp "$LOADER_LICENCE" "$APP/Contents/Resources/vulkan/Vulkan-Loader-LICENSE.txt"
  for lib in "$APP/Contents/Frameworks/"*.dylib; do sign --options runtime "$lib"; done
  # Ad-hoc signatures have no team: library validation would refuse the bundled libraries.
  [[ $IDENTITY == "-" ]] && ENTITLEMENTS="$HERE/entitlements-adhoc.plist"
fi
if [[ -n "$BUNDLE_MODEL" ]]; then
  mkdir -p "$APP/Contents/Resources/models"
  cp "$BUNDLE_MODEL" "$APP/Contents/Resources/models/"
fi

# Sign inside-out: nested code first (the libraries above), then the executable, then the bundle
# itself (no --deep on the final signature).
sign --options runtime --entitlements "$ENTITLEMENTS" "$APP/Contents/MacOS/MailCraft"
sign --options runtime --entitlements "$ENTITLEMENTS" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"
# What each piece of code in the bundle runs on: the program holds every slice asked for; the
# bundled Vulkan libraries are for the Intel slice alone (Apple silicon uses Metal, the system's).
while IFS= read -r -d '' f; do
  if file -b "$f" | grep -q Mach-O; then echo "==> ${f#"$APP"/}: $(lipo -archs "$f")"; fi
done < <(find "$APP/Contents/MacOS" "$APP/Contents/Frameworks" -type f -print0 2>/dev/null)
for t in "${TARGETS[@]}"; do
  want="${t%%-*}"; [[ $want == aarch64 ]] && want=arm64
  lipo -archs "$APP/Contents/MacOS/MailCraft" | grep -qw "$want" || { echo "error: the program has no $want slice" >&2; exit 1; }
done

if [[ $NOTARIZE == 1 ]]; then
  ditto -c -k --keepParent "$APP" "$WORK/MailCraft-notarize.zip"
  notarize "$WORK/MailCraft-notarize.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
  spctl --assess --type execute -vvv "$APP"
fi

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/MailCraft.app"
ln -s /Applications "$STAGE/Applications"
# Finder window layout: background, icon size and positions (packaging/macos/dmg/README.md).
# MailCraft has no DMG artwork yet (packaging/macos/dmg/README.md): without it the DMG opens as a
# plain Finder window.
if [[ -f "$HERE/dmg/background.tiff" ]] && [[ -f "$HERE/dmg/dmg-layout.DS_Store" ]]; then
  mkdir -p "$STAGE/.background"
  cp "$HERE/dmg/background.tiff" "$STAGE/.background/background.tiff"
  cp "$HERE/dmg/dmg-layout.DS_Store" "$STAGE/.DS_Store"
fi
rm -f "$DMG"
# `create -srcfolder` copies the staged folder as it is. (`makehybrid -hfs`, which builds an image
# without attaching a device, gives every file and folder Finder information on current macOS,
# and the app's signature doesn't hold with that attached: `codesign --verify --strict` on the
# mounted image says "detritus not allowed".) Attaching can fail with "Resource busy" on a CI
# runner, so it is tried a few times.
# The volume name has no version: .DS_Store finds the background through an alias that includes it.
made=0
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "MailCraft" -srcfolder "$STAGE" -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$DMG"; then
    made=1
    break
  fi
  echo "hdiutil create failed (attempt $attempt of 5); trying again" >&2
  sleep $((attempt * 5))
done
if [[ $made != 1 ]]; then
  echo "error: could not build $DMG" >&2
  exit 1
fi
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
if [[ $NOTARIZE == 1 ]]; then
  notarize "$DMG"
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature -vvv "$DMG"
fi

# ---- CLI ---------------------------------------------------------------------------------------
echo "==> building $CLI_ZIP"
CLI_DIR="$WORK/mailcraft-cli-$VERSION-macos-$ARCH"
mkdir -p "$CLI_DIR"
cp "$WORK/bin/mailcraft-cli" "$CLI_DIR/"
copy_docs "$CLI_DIR"
sign --options runtime "$CLI_DIR/mailcraft-cli"
codesign --verify --strict --verbose=2 "$CLI_DIR/mailcraft-cli"
rm -f "$CLI_ZIP"
ditto -c -k --norsrc --noextattr --keepParent "$CLI_DIR" "$CLI_ZIP"
# A bare Mach-O can't carry a stapled ticket; Gatekeeper looks the notarization up online.
if [[ $NOTARIZE == 1 ]]; then notarize "$CLI_ZIP"; fi

if [[ $NO_RUN == 1 ]]; then
  echo "==> skipping mailcraft-cli --version (--no-run): a job without the signing secrets runs it"
else
  "$WORK/bin/mailcraft-cli" --version
fi
echo "==> done"
ls -lh "$DMG" "$CLI_ZIP"
