#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
mkdir -p "$data_dir/applications" "$data_dir/icons/hicolor/scalable/apps"
cp "$project_dir/assets/optic-chat.svg" "$data_dir/icons/hicolor/scalable/apps/optic-chat.svg"
cat > "$data_dir/applications/app.opticchat.desktop" <<ENTRY
[Desktop Entry]
Type=Application
Name=Optic Chat
Comment=A screen-aware AI guide, beside your work
Exec="$project_dir/run.sh"
Icon=optic-chat
Terminal=false
Categories=Utility;
StartupNotify=false
ENTRY
command -v update-desktop-database >/dev/null && update-desktop-database "$data_dir/applications" || true
echo "Installed Applications → Optic Chat"
