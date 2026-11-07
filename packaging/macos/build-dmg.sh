#!/usr/bin/env bash
# Make a disk image from a `cargo xtask dist` folder: SeaCraft.app, with the licences inside it.
# Usage: build-dmg.sh STAGE OUT.dmg VERSION
set -euo pipefail

stage=$1
out=$2
version=$3
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

volume=$work/volume
app=$volume/SeaCraft.app
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$stage/seacraft-gui" "$stage/seacraft" "$app/Contents/MacOS/"
sed -e "s/@VERSION@/$version/g" "$here/Info.plist" >"$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"

iconset=$work/SeaCraft.iconset
mkdir "$iconset"
source_icon=$here/../../crates/seacraft-gui/assets/icon-1024.png
for size in 16 32 128 256 512; do
    double=$((size * 2))
    sips -z "$size" "$size" "$source_icon" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z "$double" "$double" "$source_icon" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns -o "$app/Contents/Resources/SeaCraft.icns" "$iconset"

# The licences travel with the programs: people drag only the app out of the image.
cp -R "$stage/LICENSE" "$stage/NOTICE" "$stage/ATTRIBUTION.md" "$stage/DISCLAIMER.md" \
    "$stage/README.txt" "$stage/THIRD-PARTY-LICENSES.txt" "$stage/LICENSES" "$stage/examples" \
    "$app/Contents/Resources/"

# An ad-hoc signature proves no one, but an Apple silicon Mac will not run a program without one.
codesign --force --sign - "$app/Contents/MacOS/seacraft"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"

cp "$stage/README.txt" "$volume/README.txt"
ln -s /Applications "$volume/Applications"
hdiutil create -ov -format UDZO -volname "SeaCraft $version" -srcfolder "$volume" "$out"
