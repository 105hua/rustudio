#!/bin/sh
# Builds rustudio.app (and a zip of it) for macOS.
#
#   tools/macos/bundle.sh                # this Mac's architecture
#   tools/macos/bundle.sh --universal    # Apple Silicon + Intel in one app
#
# Output: target/macos/rustudio.app and target/macos/rustudio-<version>-macos-<arch>.zip
#
# The app is ad-hoc signed, not notarized: the first time, open it with
# right-click > Open (or System Settings > Privacy & Security > Open Anyway).
set -eu

cd "$(dirname "$0")/../.."
ROOT=$(pwd)

UNIVERSAL=0
for a in "$@"; do
    case "$a" in
        --universal) UNIVERSAL=1 ;;
        *) echo "unknown option: $a" >&2; exit 2 ;;
    esac
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
# The oldest macOS the app runs on (Rust's own minimum for Apple Silicon).
export MACOSX_DEPLOYMENT_TARGET=11.0

OUT="$ROOT/target/macos"
APP="$OUT/rustudio.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

if [ "$UNIVERSAL" = 1 ]; then
    ARCH=universal
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
        rustup target add "$t" >/dev/null 2>&1 || true
        cargo build --release --locked --target "$t"
    done
    lipo -create \
        target/aarch64-apple-darwin/release/rustudio \
        target/x86_64-apple-darwin/release/rustudio \
        -output "$APP/Contents/MacOS/rustudio"
else
    ARCH=$(uname -m)
    cargo build --release --locked
    cp target/release/rustudio "$APP/Contents/MacOS/rustudio"
fi

# Data files read at run time (platform::resource).
cp api-dump.json "$APP/Contents/Resources/"

# Icon: an .iconset from the PNGs in assets/icons.
ICONSET="$OUT/rustudio.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    cp "assets/icons/rustudio-$s.png" "$ICONSET/icon_${s}x${s}.png"
    d=$((s * 2))
    cp "assets/icons/rustudio-$d.png" "$ICONSET/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/rustudio.icns"
rm -rf "$ICONSET"

sed "s/@VERSION@/$VERSION/g" tools/macos/Info.plist > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# Ad-hoc signature: required to run on Apple Silicon; no Apple ID involved.
codesign --force --deep --sign - "$APP"
codesign --verify --strict "$APP"

ZIP="$OUT/rustudio-$VERSION-macos-$ARCH.zip"
rm -f "$ZIP"
(cd "$OUT" && ditto -c -k --keepParent rustudio.app "$ZIP")
echo "$APP"
echo "$ZIP"
