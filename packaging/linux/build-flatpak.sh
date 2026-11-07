#!/usr/bin/env bash
# Build a single-file Flatpak bundle from a `cargo xtask dist` folder, with flatpak alone. It wraps
# the same programs as the other Linux downloads; a Flathub submission would have to build from
# source instead.
# Usage: build-flatpak.sh STAGE OUT.flatpak VERSION
set -euo pipefail

stage=$1
out=$2
version=$3
here=$(cd "$(dirname "$0")" && pwd)
id=io.github.havan.SeaCraft
runtime_version=25.08

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

build=$work/build
mkdir -p "$build/files"
cat >"$build/metadata" <<EOF
[Application]
name=$id
runtime=org.freedesktop.Platform/x86_64/$runtime_version
sdk=org.freedesktop.Sdk/x86_64/$runtime_version
EOF
"$here/install-tree.sh" "$stage" "$build/files" "$version"

# Files are chosen in SeaCraft's own dialog, which types paths, so it needs to see the home folder.
flatpak build-finish "$build" \
    --command=seacraft-gui \
    --socket=wayland \
    --socket=fallback-x11 \
    --share=ipc \
    --device=dri \
    --filesystem=home

flatpak build-export "$work/repo" "$build" stable
flatpak build-bundle "$work/repo" "$out" "$id" stable \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
