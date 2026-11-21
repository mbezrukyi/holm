#!/usr/bin/env bash
# Builds the x86_64 AppImage of Holm and its SHA256SUMS in target/appimage.
#
# Usage: packaging/appimage/build.sh VERSION
#
# Run it on the oldest system Holm supports, today Ubuntu 22.04. The AppImage then runs on every
# system with the same glibc or a newer one.
set -euo pipefail

version="${1:?usage: packaging/appimage/build.sh VERSION}"
id=io.github.mbezrukyi.Holm
linuxdeploy_tag=1-alpha-20251107-1
linuxdeploy_sha256=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d

cd "$(dirname "$0")/../.."
out=target/appimage
name="Holm-${version}-x86_64.AppImage"

cargo build --release --locked -p holm

rm -rf "$out"
mkdir -p "$out"
curl -fsSL -o "$out/linuxdeploy" \
  "https://github.com/linuxdeploy/linuxdeploy/releases/download/$linuxdeploy_tag/linuxdeploy-x86_64.AppImage"
echo "$linuxdeploy_sha256  $out/linuxdeploy" | sha256sum -c --quiet -
chmod +x "$out/linuxdeploy"

# Extract and run, so the build needs no FUSE. libxkbcommon stays out: it reads the keyboard
# files of the system, and an old copy fails on the files of a newer system.
APPIMAGE_EXTRACT_AND_RUN=1 LDAI_OUTPUT="$out/$name" "$out/linuxdeploy" \
  --appdir "$out/AppDir" \
  --executable target/release/holm \
  --exclude-library 'libxkbcommon*' \
  --desktop-file "packaging/linux/$id.desktop" \
  --icon-file "packaging/linux/$id.svg" \
  --output appimage

(cd "$out" && sha256sum "$name" > SHA256SUMS)
