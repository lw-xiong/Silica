//! Windows resources for `silica.exe`.
//!
//! gpui's window class takes its icon from resource 1 of the running module
//! (`LoadImageW(module, PCWSTR(1), IMAGE_ICON, ..)`), and Explorer reads the same resource for
//! the file's icon, so embedding `resources/windows/silica.ico` under that id is what puts the
//! logo on the window, the taskbar button and the executable at once. `VERSIONINFO` is the
//! other half of the same story: without it the Properties dialog and SmartScreen's warning
//! have nothing to call the program but `silica.exe`.
//!
//! The `.ico` is cut from the logo master by `tools/app-icons/cut.py`; the macOS half of the
//! pair is `resources/macos/silica.icns`, which `tools/macos-app/bundle.sh` puts in the bundle.
//!
//! Everything here is a no-op off Windows, and gpui embeds its own manifest as resource 1 of
//! type `RT_MANIFEST`, which is a different type from `ICON` and so does not collide.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

fn windows_resources() {
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest_dir.join("resources/windows/silica.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    // `0.3.2` is `0,3,2,0` to the resource compiler, which counts to four and takes numbers only.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<&str> = version.split(['.', '-', '+']).collect();
    parts.resize(4, "0");
    let quad: Vec<String> = parts
        .iter()
        .map(|p| p.parse::<u16>().unwrap_or(0).to_string())
        .collect();
    let quad = quad.join(",");

    // Backslashes are escapes inside an .rc string, and the path is absolute because the
    // script is written next to the object file rather than beside the icon.
    let icon_path = icon.display().to_string().replace('\\', "\\\\");
    let rc = format!(
        r#"1 ICON "{icon_path}"

1 VERSIONINFO
FILEVERSION {quad}
PRODUCTVERSION {quad}
FILEOS 0x4L
FILETYPE 0x1L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Silica"
      VALUE "FileDescription", "Silica: a native knowledge base compatible with Obsidian vaults"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "silica"
      VALUE "LegalCopyright", "MIT licensed"
      VALUE "OriginalFilename", "silica.exe"
      VALUE "ProductName", "Silica"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );

    let script = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("silica.rc");
    std::fs::write(&script, rc).expect("write the resource script");
    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_required()
        .expect("compile the Windows resources");
}
