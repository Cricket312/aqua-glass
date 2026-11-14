#!/usr/bin/env bash
# Guards the portable/desktop boundary:
#  1. the portable crates must not depend on UI, audio-output or OS-specific crates
#     (dependency tree resolved for Android; low-level std helpers such as
#     windows-sys under socket2/ring only appear for Windows targets);
#  2. aqua-core must compile for Android (needs: rustup target add aarch64-linux-android).
#     aqua-providers/aqua-radio also build for Android, but `ring` (TLS) compiles C
#     code and needs the Android NDK: use `cargo ndk -t arm64-v8a check -p aqua-radio`.
set -euo pipefail
cd "$(dirname "$0")/.."
forbidden='^(eframe|egui|winit|glow|cpal|librespot|symphonia|windows|windows-sys|winreg|keyring|tray-icon|souvlaki|global-hotkey|rfd|tauri-winrt-notification|discord-rich-presence|open) v'
status=0
for c in aqua-core aqua-providers aqua-radio; do
  bad=$(cargo tree -p "$c" -e normal --prefix none --target aarch64-linux-android | grep -E "$forbidden" | sort -u || true)
  if [ -n "$bad" ]; then echo "✗ $c depends on platform crates:"; echo "$bad"; status=1; else echo "✓ $c has no UI/audio/OS-specific dependencies"; fi
done
if rustup target list --installed 2>/dev/null | grep -q aarch64-linux-android; then
  cargo check -q -p aqua-core --target aarch64-linux-android && echo "✓ aqua-core compiles for aarch64-linux-android"
  if command -v cargo-ndk >/dev/null; then cargo ndk -t arm64-v8a check -q -p aqua-radio && echo "✓ aqua-providers + aqua-radio compile for Android (NDK)"; fi
else
  echo "• skip Android check (rustup target add aarch64-linux-android)"
fi
exit $status
