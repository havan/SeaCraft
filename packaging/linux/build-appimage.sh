#!/usr/bin/env bash
# Build an AppImage from a `cargo xtask dist` folder.
# Usage: build-appimage.sh STAGE OUT.AppImage VERSION
#
# The two AppImage tools are downloaded by version and checked against these hashes. Graphics
# libraries are left out on purpose: they must come from the user's own system to match its driver.
set -euo pipefail

stage=$1
out=$2
version=$3
here=$(cd "$(dirname "$0")" && pwd)
id=io.github.havan.SeaCraft

tool_url=https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage
tool_sha256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
runtime_url=https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
runtime_sha256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL "$tool_url" -o "$work/appimagetool"
curl -fsSL "$runtime_url" -o "$work/runtime"
echo "$tool_sha256  $work/appimagetool" | sha256sum -c -
echo "$runtime_sha256  $work/runtime" | sha256sum -c -
chmod +x "$work/appimagetool"

app=$work/SeaCraft.AppDir
"$here/install-tree.sh" "$stage" "$app/usr" "$version"
cp "$here/$id.desktop" "$app/$id.desktop"
cp "$here/$id.svg" "$app/$id.svg"
ln -s "$id.svg" "$app/.DirIcon"
cat >"$app/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/seacraft-gui" "$@"
EOF
chmod +x "$app/AppRun"

# A build machine may have no FUSE, so the tool runs by unpacking itself.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 VERSION=$version \
    "$work/appimagetool" --runtime-file "$work/runtime" "$app" "$out"
