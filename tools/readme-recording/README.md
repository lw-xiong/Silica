# README recordings

The GIFs and stills in `docs/img/app` are real sessions of Silica on the `docs/silica-demo`
vault, driven over the control socket (`--control`) and captured one step at a time.

`record.sh` starts a private headless sway session (one 2× output, the wallpaper you pass as its
background, Silica floating in it with a margin on every side), launches a release build on a
scratch copy of the vault, runs each `clip_*.py`, and hands the frames to `compose.py`, which
scales them and encodes a GIF with ffmpeg; frames that carry a name are also written as PNG stills
(`workspace.png`, `search.png`, `autocomplete.png`).

The app dlopens wayland at run time, so on NixOS the recording needs `shell.nix`'s libraries as
well as the capture tools; without them Silica dies with `NoWaylandLib` before the socket appears.

```
cargo build --release -p silica
export LD_LIBRARY_PATH=$(nix-shell shell.nix --run 'echo $LD_LIBRARY_PATH')
nix-shell -p sway grim ffmpeg imagemagick --run 'tools/readme-recording/record.sh ~/wallpaper.png'
nix-shell -p sway grim ffmpeg imagemagick --run 'tools/readme-recording/record.sh ~/wallpaper.png tour'   # one clip
```

The wallpaper is whatever you pass; it is not in the repository, and `record.sh` falls back to
`tools/readme-recording/wallpaper.png` only if you put one there.

Nothing on the real desktop is touched or captured: the session is headless, so it works with the
monitors asleep and with other windows in front. `rec.py` is the step recorder (open, command,
keys, type, click, scroll, frame), `ctl.py` a one-shot socket client for poking at the running app
(`ctl.py state '{"rows": 20}'`). Pointer coordinates are window-local; under sway gpui's origin
sits 24 px inside the frame, which `rec.py` corrects for.
