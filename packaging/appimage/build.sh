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
runtime_tag=20251108
runtime_sha256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

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
curl -fsSL -o "$out/runtime" \
  "https://github.com/AppImage/type2-runtime/releases/download/$runtime_tag/runtime-x86_64"
echo "$runtime_sha256  $out/runtime" | sha256sum -c --quiet -

notices="$out/THIRD-PARTY-NOTICES.txt"
cargo about generate --fail packaging/notices/crates.hbs -o "$out/crates.txt"
{
  cat <<EOF
Third-party notices for Holm $version

Holm is Copyright (C) 2026 Mykola Bezrukyi and licensed under the GNU General Public License,
version 3 or later; see LICENSE. Its source is https://github.com/mbezrukyi/holm at the tag
v$version, with the changed copies of gpui-pre and gpui-pre-linux in vendor/.

This file holds the notices of the parts of Holm that others wrote:

1. Rust crates. The source of each is on crates.io at the version named here.
2. The Inter font.
3. The AppImage runtime.

================================================================================
1. Rust crates
================================================================================
EOF
  cat "$out/crates.txt"
  printf '\n================================================================================\n'
  printf '2. The Inter font\n'
  printf '================================================================================\n\n'
  cat crates/holm/fonts/Inter-LICENSE
  printf '\n================================================================================\n'
  printf '3. The AppImage runtime\n'
  printf '================================================================================\n\n'
  sed "s/{runtime_tag}/$runtime_tag/g" packaging/notices/appimage-runtime.txt
} > "$notices"
rm "$out/crates.txt"

doc="$out/AppDir/usr/share/doc/holm"
mkdir -p "$doc"
cp LICENSE "$notices" "$doc/"

# Extract and run, so the build needs no FUSE. libxkbcommon stays out: it reads the keyboard
# files of the system, and an old copy fails on the files of a newer system.
APPIMAGE_EXTRACT_AND_RUN=1 LDAI_OUTPUT="$out/$name" LDAI_RUNTIME_FILE="$out/runtime" "$out/linuxdeploy" \
  --appdir "$out/AppDir" \
  --executable target/release/holm \
  --exclude-library 'libxkbcommon*' \
  --desktop-file "packaging/linux/$id.desktop" \
  --icon-file "packaging/linux/$id.svg" \
  --output appimage

(cd "$out" && sha256sum "$name" > SHA256SUMS)
