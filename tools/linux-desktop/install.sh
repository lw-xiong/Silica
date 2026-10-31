#!/usr/bin/env bash
# Put Silica in the desktop's menu and give its window an icon.
#
#   tools/linux-desktop/install.sh                  # from the repository, or the archive
#   tools/linux-desktop/install.sh --binary ~/bin/silica
#   tools/linux-desktop/install.sh --prefix /usr/local/share   # system-wide, needs root
#   tools/linux-desktop/install.sh --uninstall
#
# A Linux desktop keeps an application's icon in neither the executable nor a bundle but in the
# icon theme, looked up by the `Icon=` name in a .desktop file. The task bar finds the entry for
# a running window by matching its Wayland app_id (or X11 WM_CLASS) against `StartupWMClass`,
# which is why the window is drawn with the generic icon until both files are installed.
#
# Everything lands under the XDG data directory, which is per-user by default and needs no root.
set -euo pipefail

prefix=${XDG_DATA_HOME:-$HOME/.local/share}
binary=
uninstall=false

while [ $# -gt 0 ]; do
  case $1 in
    --uninstall) uninstall=true ;;
    --binary) binary=${2:?--binary needs a path}; shift ;;
    --prefix) prefix=${2:?--prefix needs a directory}; shift ;;
    -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]}" | cut -c3-; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
desktop="$prefix/applications/silica.desktop"
icons="$prefix/icons/hicolor"

# Refresh whatever caches this desktop keeps. Both are best-effort: a missing tool means the
# desktop reads the directories directly, and GNOME and KDE both notice the new files anyway.
refresh() {
  command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$prefix/applications" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    gtk-update-icon-cache -qtf "$icons" 2>/dev/null || true
}

if $uninstall; then
  rm -f "$desktop"
  # Only the files this script wrote, so an icon theme of the user's own is left alone.
  find "$icons" -name silica.png -path '*/apps/*' -delete 2>/dev/null || true
  rm -f "$icons/scalable/apps/silica.svg"
  refresh
  echo "removed $desktop and the silica icons under $icons"
  exit 0
fi

# Two layouts: the repository, and the release archive, where install.sh sits beside the binary.
if [ -d "$here/../../crates/silica/resources/linux" ]; then
  source_dir="$here/../../crates/silica/resources/linux"
elif [ -d "$here/icons/hicolor" ]; then
  source_dir="$here"
else
  echo "cannot find silica.desktop and icons/ next to $here" >&2
  exit 1
fi

# The entry ships `Exec=silica`, which is right only when the binary is on PATH. It is not when
# the archive has been unpacked into a folder, so the absolute path goes in instead.
if [ -z "$binary" ]; then
  if [ -x "$here/silica" ]; then
    binary="$here/silica"
  elif command -v silica >/dev/null 2>&1; then
    binary=silica
  else
    echo "no silica binary found; pass --binary <path>" >&2
    exit 1
  fi
fi
if [ "$binary" != "silica" ]; then
  binary=$(cd "$(dirname "$binary")" && pwd)/$(basename "$binary")
  [ -x "$binary" ] || { echo "$binary is not executable" >&2; exit 1; }
fi

mkdir -p "$prefix/applications"
# A path with a space in it has to be quoted for the Exec key, and `exec` is the only key that
# changes, so the rest of the entry is copied through untouched.
exec_line="Exec=$binary %f"
case $binary in *[[:space:]]*) exec_line="Exec=\"$binary\" %f" ;; esac
sed "s|^Exec=.*|$exec_line|" "$source_dir/silica.desktop" > "$desktop"

mkdir -p "$icons"
cp -r "$source_dir/icons/hicolor/." "$icons/"

refresh
echo "installed $desktop"
echo "  Exec=$binary"
echo "  icons under $icons"
