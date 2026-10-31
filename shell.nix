# Development shell for NixOS and other Nix users.
#
# gpui links against libxcb and libxkbcommon at build time and dlopens
# wayland, libvulkan, and libGL at run time. `nix-shell` puts all of them
# on the path so `cargo test --workspace` and `cargo run -p silica` work.
{
  pkgs ? import <nixpkgs> { },
}:
let
  runtimeLibs = with pkgs; [
    libxcb
    libxkbcommon
    wayland
    vulkan-loader
    libGL
    fontconfig
    freetype
  ];
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
  ];
  buildInputs = runtimeLibs;
  shellHook = ''
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath runtimeLibs}:/run/opengl-driver/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  '';
}
