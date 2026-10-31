//! The icons each desktop reads are committed binaries, cut from the logo master by
//! `tools/app-icons/cut.py`. Nothing in an ordinary build reads them - `build.rs` embeds the
//! `.ico` only when the target is Windows, and the `.icns` and the hicolor tree are packaging -
//! so a truncated or half-written re-cut would otherwise go unnoticed until someone looked at a
//! task bar. These parse what is committed and check it still says what the three platforms
//! need it to say.

use std::fs;
use std::path::PathBuf;

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

fn resource(path: &str) -> Vec<u8> {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join(path);
    fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()))
}

/// Width and height out of a PNG's IHDR, which is always the first chunk.
fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], PNG_MAGIC, "not a PNG");
    assert_eq!(&png[12..16], b"IHDR", "first chunk is not IHDR");
    let dim = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    (dim(16), dim(20))
}

#[test]
fn the_windows_icon_carries_every_shell_size() {
    let ico = resource("windows/silica.ico");
    assert_eq!(&ico[..4], &[0, 0, 1, 0], "not an ICONDIR of icons");
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;

    let mut sizes = Vec::new();
    for i in 0..count {
        let entry = &ico[6 + 16 * i..22 + 16 * i];
        // 0 in the width byte means 256: the field is one byte and the size is not.
        let declared = if entry[0] == 0 { 256 } else { entry[0] as u32 };
        assert_eq!(entry[0], entry[1], "entry {i} is not square");

        let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let payload = &ico[offset..offset + len];
        // PNG rather than a DIB throughout, which every Windows since Vista reads and which
        // keeps the 256 px entry from costing a quarter of a megabyte.
        assert_eq!(
            png_size(payload),
            (declared, declared),
            "entry {i} lies about its size"
        );
        sizes.push(declared);
    }
    assert_eq!(sizes, vec![16, 20, 24, 32, 48, 64, 128, 256]);
}

#[test]
fn the_macos_icon_carries_the_modern_entry_set() {
    let icns = resource("macos/silica.icns");
    assert_eq!(&icns[..4], b"icns", "not an icns container");
    let declared = u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize;
    assert_eq!(
        declared,
        icns.len(),
        "the container's length disagrees with the file's"
    );

    // The 1x small icons, then the retina set. 256 and 512 appear twice because 256 @1x and
    // 128 @2x are different entries of the same pixel size, and Finder reads both.
    let expected = [
        (*b"icp4", 16),
        (*b"icp5", 32),
        (*b"ic11", 32),
        (*b"ic12", 64),
        (*b"ic07", 128),
        (*b"ic13", 256),
        (*b"ic08", 256),
        (*b"ic14", 512),
        (*b"ic09", 512),
        (*b"ic10", 1024),
    ];

    let mut found = Vec::new();
    let mut at = 8;
    while at < icns.len() {
        let kind: [u8; 4] = icns[at..at + 4].try_into().unwrap();
        let len = u32::from_be_bytes(icns[at + 4..at + 8].try_into().unwrap()) as usize;
        assert!(
            len >= 8 && at + len <= icns.len(),
            "entry {kind:?} runs past the end"
        );
        let (w, h) = png_size(&icns[at + 8..at + len]);
        assert_eq!(w, h, "entry {kind:?} is not square");
        found.push((kind, w));
        at += len;
    }
    assert_eq!(found, expected);
}

#[test]
fn the_hicolor_theme_carries_every_size_the_desktop_entry_names() {
    let icons = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/linux/icons/hicolor");

    for size in [16, 24, 32, 48, 64, 128, 256, 512] {
        let png = icons.join(format!("{size}x{size}/apps/silica.png"));
        let bytes = fs::read(&png).unwrap_or_else(|e| panic!("{}: {e}", png.display()));
        // The directory name is the contract the icon theme spec is read by, so a file that
        // disagrees with the directory it sits in is worse than a missing one.
        assert_eq!(
            png_size(&bytes),
            (size, size),
            "{} is the wrong size",
            png.display()
        );
    }

    let svg = fs::read_to_string(icons.join("scalable/apps/silica.svg")).expect("scalable");
    assert!(
        svg.starts_with("<?xml"),
        "the scalable icon is not an XML document"
    );
    assert!(
        svg.contains(r#"viewBox="0 0 1024 1024""#),
        "the scalable icon is not square"
    );
}

#[test]
fn the_desktop_entry_matches_the_window_it_belongs_to() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let entry = fs::read_to_string(crate_dir.join("resources/linux/silica.desktop"))
        .expect("silica.desktop");
    let value = |key: &str| {
        entry
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("no {key} in silica.desktop"))
            .trim()
    };

    // `Icon` is a name looked up in the theme, not a path: it has to be the stem the hicolor
    // files were written under, or every desktop draws the generic icon instead.
    assert_eq!(value("Icon"), "silica");

    // The task bar ties a running window to this entry by matching the Wayland app_id (the
    // X11 WM_CLASS) against StartupWMClass. Nothing but this test connects the two strings,
    // and the window keeps the generic icon the moment they drift apart.
    let main = fs::read_to_string(crate_dir.join("src/main.rs")).expect("main.rs");
    let app_id = main
        .split("app_id: Some(\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("app_id in WindowOptions");
    assert_eq!(value("StartupWMClass"), app_id);
}
