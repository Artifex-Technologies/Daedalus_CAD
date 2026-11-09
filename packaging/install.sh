#!/bin/sh
# Builds Daedalus in release mode and installs it for the current user, so it
# shows up in the application menu. Run again after pulling to update.
# Uninstall: packaging/install.sh --uninstall
set -eu

cd "$(dirname "$0")/.."
BIN="$HOME/.local/bin/daedalus"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/scalable/apps"
MIME="$HOME/.local/share/mime"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN" "$APPS/daedalus.desktop" "$ICONS/daedalus.svg" "$MIME/packages/daedalus.xml"
    update-mime-database "$MIME" 2>/dev/null || true
    echo "Daedalus uninstalled."
    exit 0
fi

cargo build --release --locked -p daedalus-viewer
mkdir -p "$(dirname "$BIN")" "$APPS" "$ICONS"
install -m 755 target/release/daedalus-viewer "$BIN"
install -m 644 packaging/daedalus.svg "$ICONS/daedalus.svg"
# Absolute path in Exec, so the launcher works even if ~/.local/bin is not on PATH.
sed "s|^Exec=daedalus|Exec=$BIN|" packaging/daedalus.desktop > "$APPS/daedalus.desktop"
chmod 644 "$APPS/daedalus.desktop"
# .ddls (and .daedalus) files as Daedalus models, so a file manager opens them here; STEP's type comes with the system
mkdir -p "$MIME/packages"
install -m 644 packaging/daedalus-mime.xml "$MIME/packages/daedalus.xml"
update-mime-database "$MIME" 2>/dev/null || true
update-desktop-database "$APPS" 2>/dev/null || true
gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
echo "Installed: $BIN (run 'daedalus' or find Daedalus CAD in the application menu)"
