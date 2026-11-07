#!/usr/bin/env bash
# Lay out a `cargo xtask dist` folder under a prefix, the way a Linux desktop expects it.
# Usage: install-tree.sh STAGE PREFIX VERSION
set -euo pipefail

stage=$1
prefix=$2
version=$3
here=$(cd "$(dirname "$0")" && pwd)
id=io.github.havan.SeaCraft

install -Dm755 "$stage/seacraft-gui" "$prefix/bin/seacraft-gui"
install -Dm755 "$stage/seacraft" "$prefix/bin/seacraft"
install -Dm644 "$here/$id.desktop" "$prefix/share/applications/$id.desktop"
install -Dm644 "$here/$id.svg" "$prefix/share/icons/hicolor/scalable/apps/$id.svg"
install -Dm644 "$here/../../crates/seacraft-gui/assets/icon-256.png" \
    "$prefix/share/icons/hicolor/256x256/apps/$id.png"
install -d "$prefix/share/metainfo"
sed -e "s/@VERSION@/$version/" -e "s/@DATE@/$(date -u +%F)/" "$here/$id.metainfo.xml" \
    >"$prefix/share/metainfo/$id.metainfo.xml"
if command -v appstreamcli >/dev/null; then
    appstreamcli validate --no-net "$prefix/share/metainfo/$id.metainfo.xml"
fi

# The licences travel with the programs.
docs=$prefix/share/doc/seacraft
install -d "$docs"
cp -r "$stage/LICENSE" "$stage/NOTICE" "$stage/ATTRIBUTION.md" "$stage/DISCLAIMER.md" \
    "$stage/README.txt" "$stage/THIRD-PARTY-LICENSES.txt" "$stage/LICENSES" "$stage/examples" "$docs/"
