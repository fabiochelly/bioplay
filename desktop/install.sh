#!/usr/bin/env bash
# Installe bioplay pour l'utilisateur courant (~/.local), sans toucher au lecteur par défaut.
# Pour en faire le lecteur par défaut des MKV/MP4 :
#   xdg-mime default bioplay.desktop video/x-matroska video/mp4 video/webm
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p bioplay
install -Dm755 target/release/bioplay ~/.local/bin/bioplay
install -Dm644 desktop/data/bioplay.desktop ~/.local/share/applications/bioplay.desktop
install -Dm644 desktop/data/bioplay.svg ~/.local/share/icons/hicolor/scalable/apps/bioplay.svg
update-desktop-database ~/.local/share/applications 2>/dev/null || true
echo "bioplay installé : ~/.local/bin/bioplay"
