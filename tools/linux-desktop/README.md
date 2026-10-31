# Desktop entry

A Linux desktop keeps an application's icon in neither the executable nor a bundle: it reads a
name out of a `.desktop` file and looks that name up in the icon theme. So the Linux half of the
icon work is two installed files rather than anything embedded, and `install.sh` is what puts
them in place:

```
just desktop-install                                      # a release build, then both files
tools/linux-desktop/install.sh --binary ~/bin/silica      # an existing binary
tools/linux-desktop/install.sh --prefix /usr/local/share  # system-wide, needs root
tools/linux-desktop/install.sh --uninstall
```

Everything lands under `$XDG_DATA_HOME` (`~/.local/share` by default), so nothing needs root:
`applications/silica.desktop`, and the hicolor theme from
`crates/silica/resources/linux/icons`, cut from the logo master by `tools/app-icons/cut.py`.
It then refreshes `update-desktop-database` and `gtk-update-icon-cache` if either is installed,
and `--uninstall` removes exactly the files it wrote.

`silica.desktop` ships `Exec=silica`, which is right when the binary is on PATH and wrong for an
unpacked archive, so the script substitutes the absolute path of the binary it found. It reads
its sources either from the repository or from beside itself, which is where the release archive
puts it, next to the binary.

Two keys in the entry are load-bearing and easy to break:

* **`Icon=silica`** is a theme name, not a path. It has to be the stem the hicolor PNGs were
  written under.
* **`StartupWMClass=silica`** is how the task bar ties a running window back to this entry, and
  so to the icon. It has to equal the `app_id` GPUI is given in `WindowOptions` (`src/main.rs`) —
  the Wayland app_id, the X11 `WM_CLASS`. Nothing but `crates/silica/tests/icons.rs` connects the
  two strings, and the window falls back to the generic icon the moment they drift apart.

`MimeType=inode/directory` is what puts Silica in a file manager's *Open With* for a folder,
which is the shape its argument has: a vault is a directory. With no argument at all Silica opens
empty, and the sidebar's footer has *Open vault…*.
