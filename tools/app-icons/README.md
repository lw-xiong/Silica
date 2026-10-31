# App icons

The icon inside the executables, cut from `docs/img/silica_v3_fixed.svg` the way the three SVG
copies of the logo are:

```
just icons                                             # or, without just:
nix-shell -p imagemagick --run tools/app-icons/cut.py
```

It writes two committed files, because neither platform can be handed an SVG:

| file | what reads it |
| --- | --- |
| `crates/silica/resources/windows/silica.ico` | `crates/silica/build.rs` embeds it as resource 1 of `silica.exe`, which is where gpui's window class looks for the window and taskbar icon and where Explorer looks for the file's |
| `crates/silica/resources/macos/silica.icns` | `tools/macos-app/bundle.sh` puts it in `Silica.app/Contents/Resources`, and `Info.plist`'s `CFBundleIconFile` names it |
| `crates/silica/resources/linux/icons/hicolor/` | `tools/linux-desktop/install.sh` copies the theme into the XDG data directory, where `silica.desktop`'s `Icon=silica` finds it |

Two drawings go in, because no single one survives the range from 16 px to 1024 px. At 48 px and
up it is the master itself on a dark rounded square — the same colour and the same 7/32 corner
radius as the site's favicon — keeping the outline, the gradients and all fourteen vesicles. At
32 px and below it is `site/sites/silica/assets/favicon.svg`, which is that master already
flattened to a silhouette and its six largest holes, for exactly this reason: the full mark turns
to mud by 16 px. The macOS entries are additionally inset to Apple's 824/1024 content box, which
is what makes a Dock icon the same visual size as the ones either side of it; the Windows entries
are full-bleed, which is what Explorer draws.

The `.ico` carries 16, 20, 24, 32, 48, 64, 128 and 256; the `.icns` carries the modern PNG entry
set (`icp4`, `icp5`, `ic07`–`ic14`); hicolor gets 16, 24, 32, 48, 64, 128, 256 and 512 plus the
scalable SVG, which is the plate with the master nested inside it rather than anything redrawn,
so it stays one drawing at any size. Both binary containers are written here rather than by
ImageMagick or `iconutil`, so the cut runs on any machine and not only on a Mac, and
`crates/silica/tests/icons.rs` parses all three committed sets back.

The cut is reproducible in pixels, not in bytes: librsvg and Inkscape render the master
identically, but two ImageMagick builds will compress the PNGs differently, so re-cutting with no
change to the master can still show a diff.
