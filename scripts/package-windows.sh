#!/usr/bin/env bash
# Build the Windows release exe (cross-compiling from Linux with MinGW, or natively
# with the GNU toolchain) and package a portable zip in dist/.
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET=x86_64-pc-windows-gnu
cargo build --release --target "$TARGET"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=dist/aqua-glass-$VERSION-windows-x64
rm -rf "$OUT" && mkdir -p "$OUT/docs"
cp "target/$TARGET/release/aqua-glass.exe" "$OUT/"
cp README.md CHANGELOG.md LICENSE "$OUT/"
cp docs/*.md "$OUT/docs/"
cat > "$OUT/PORTABLE-MODE.txt" <<'TXT'
Aqua Glass stores its data in %APPDATA%\AquaGlass by default.
To keep all data next to the exe instead, create an empty file named "portable.txt" in this folder.
TXT
(cd dist && rm -f "$(basename "$OUT").zip" && zip -qr "$(basename "$OUT").zip" "$(basename "$OUT")")
sha256sum "$OUT/aqua-glass.exe" "dist/$(basename "$OUT").zip" | tee "dist/SHA256SUMS-$VERSION.txt"
ls -la dist
