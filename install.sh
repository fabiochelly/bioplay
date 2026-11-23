#!/bin/sh
# bioplay pour Linux : installation autonome, sans droits administrateur.
# Télécharge le paquet complet (bioplay, libmpv, ffmpeg…) depuis les Releases GitHub du dépôt,
# l'installe dans ~/.local, avec son entrée dans le menu des applications.
#
#   curl -fsSL https://raw.githubusercontent.com/fabiochelly/bioplay/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/fabiochelly/bioplay/main/install.sh | sh -s -- --uninstall
#
# Relancer la même commande met bioplay à jour.
set -eu

REPO=fabiochelly/bioplay
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
DEST=$DATA/bioplay
BIN=$HOME/.local/bin
APPS=$DATA/applications
ICONS=$DATA/icons/hicolor/scalable/apps

if [ "${1:-}" = "--uninstall" ]; then
    rm -rf "$DEST" "$BIN/bioplay" "$APPS/bioplay.desktop" "$ICONS/bioplay.svg"
    command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
    echo "bioplay désinstallé (vos réglages et positions de lecture restent dans ~/.local/state/bioplay)."
    exit 0
fi

arch=$(uname -m)
if [ "$arch" != "x86_64" ]; then
    echo "bioplay : pas encore de paquet pour $arch (seulement x86_64)." >&2
    exit 1
fi

url="https://github.com/$REPO/releases/latest/download/bioplay-linux-$arch.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Téléchargement de bioplay…"
if [ -n "${BIOPLAY_ARCHIVE:-}" ]; then
    # Essai : une archive locale au lieu du téléchargement.
    cp "$BIOPLAY_ARCHIVE" "$tmp/bioplay.tar.gz"
elif command -v curl >/dev/null; then
    curl -fSL --progress-bar "$url" -o "$tmp/bioplay.tar.gz"
elif command -v wget >/dev/null; then
    wget -q --show-progress -O "$tmp/bioplay.tar.gz" "$url"
else
    echo "bioplay : il faut curl ou wget." >&2
    exit 1
fi
tar -xzf "$tmp/bioplay.tar.gz" -C "$tmp"

mkdir -p "$DATA" "$BIN" "$APPS" "$ICONS"
rm -rf "$DEST"
mv "$tmp/bioplay" "$DEST"
ln -sf "$DEST/bin/bioplay" "$BIN/bioplay"
# Lanceur : chemin complet, même si ~/.local/bin n'est pas dans le PATH.
sed "s|^Exec=bioplay|Exec=$DEST/bin/bioplay|" "$DEST/share/applications/bioplay.desktop" > "$APPS/bioplay.desktop"
cp "$DEST/share/icons/hicolor/scalable/apps/bioplay.svg" "$ICONS/bioplay.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true

echo "bioplay est installé : dans le menu des applications, ou « bioplay » dans un terminal."
case ":$PATH:" in
    *":$BIN:"*) ;;
    *) echo "(Pour la commande « bioplay », ajoutez $BIN à votre PATH.)" ;;
esac
