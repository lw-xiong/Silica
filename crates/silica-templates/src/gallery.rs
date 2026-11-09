//! Built-in templates Silica ships with, for the offline template gallery.
//!
//! The gallery lists generic notes Silica itself wrote
//! (a daily note, a meeting note, a project brief). Picking one installs a
//! copy into the vault's template folder (`templates.json`'s `folder`),
//! so `insert-template` can expand its `{{date}}`, `{{time}}` and
//! `{{title}}` placeholders and insert the note into the current note.
//!
//! Everything here is clean-room: the bodies were written for Silica, not
//! copied from any other product, and nothing is fetched from the network.

use std::path::{Path, PathBuf};

/// One template in the offline gallery.
#[derive(Debug, Clone)]
pub struct GalleryTemplate {
    pub id: &'static str,
    /// What the gallery lists it as.
    pub name: &'static str,
    /// One line shown under the name.
    pub description: &'static str,
    /// Markdown Silica wrote itself; may contain `{{date}}`, `{{time}}`,
    /// `{{title}}` placeholders, expanded by [`crate::expand`] on insert.
    pub body: &'static str,
}

/// The offline gallery, in listing order.
pub const GALLERY: &[GalleryTemplate] = &[
    GalleryTemplate {
        id: "daily-note",
        name: "Daily note",
        description: "A note for one day, dated and ready to fill in.",
        body: "# {{date}}\n\n## Yesterday\n\n...\n\n## Today\n\n...\n\n## Tasks\n\n- [ ]\n",
    },
    GalleryTemplate {
        id: "meeting-note",
        name: "Meeting note",
        description: "Meeting notes with attendees, agenda and action items.",
        body: "# Meeting with {{title}}\n\n{{date}} {{time}}\n\n## Attendees\n\n...\n\n## Agenda\n\n- ...\n\n## Notes\n\n...\n\n## Action items\n\n- [ ]\n",
    },
    GalleryTemplate {
        id: "project-brief",
        name: "Project brief",
        description: "A one-page brief: purpose, goals, timeline, owner.",
        body: "# {{title}}\n\n{{date}}\n\n## Purpose\n\n...\n\n## Goals\n\n- ...\n\n## Timeline\n\n...\n\n## Owner\n\n...\n",
    },
];

/// Finds a gallery template by id.
pub fn find(id: &str) -> Option<&'static GalleryTemplate> {
    GALLERY.iter().find(|t| t.id == id)
}

/// Writes `template` into `folder` (vault-relative) as `{id}.md`, only when
/// that file does not exist yet, creating any missing directories, and
/// returns the vault-relative path written. A file that is already there is
/// left untouched and reported as `AlreadyExists`.
pub fn install(template: &GalleryTemplate, folder: &str, root: &Path) -> Result<PathBuf, String> {
    let folder = folder.trim_matches('/');
    let file_name = format!("{}.md", template.id);
    let dest = if folder.is_empty() {
        PathBuf::from(&file_name)
    } else {
        PathBuf::from(format!("{folder}/{file_name}"))
    };
    let full = root.join(&dest);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not create {parent:?}: {e}"))?;
    }
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&full)
    {
        Ok(mut file) => {
            std::io::Write::write_all(&mut file, template.body.as_bytes())
                .map_err(|e| format!("could not write {dest:?}: {e}"))?;
            Ok(dest)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(format!("'{dest:?}' already exists"))
        }
        Err(e) => Err(format!("could not create {dest:?}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "silica-templates-gallery-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("sandbox dir");
        dir
    }

    #[test]
    fn the_gallery_lists_three_generic_templates() {
        let ids: Vec<&str> = GALLERY.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["daily-note", "meeting-note", "project-brief"]);
        for t in GALLERY {
            assert!(!t.id.is_empty(), "{t:?} has no id");
            assert!(!t.name.is_empty(), "{} has no name", t.id);
            assert!(!t.body.is_empty(), "{} has no body", t.id);
            assert_eq!(find(t.id).map(|t| t.id), Some(t.id));
        }
    }

    #[test]
    fn install_writes_each_template_once_into_the_folder() {
        let root = sandbox();
        let t = find("daily-note").expect("daily-note in gallery");
        let dest = install(t, "Templates", &root).expect("writes the file");
        assert_eq!(dest, Path::new("Templates").join("daily-note.md"));
        let full = root.join(&dest);
        assert!(full.exists());
        let written = std::fs::read_to_string(&full).expect("reads back");
        assert_eq!(written, t.body);
        // * A second install of the same id is an error: nothing is lost.
        let err = install(t, "Templates", &root).expect_err("refuses to overwrite");
        assert!(err.contains("already exists"), "{err}");
    }

    #[test]
    fn install_puts_templates_at_the_vault_root_when_folder_is_empty() {
        let root = sandbox();
        let t = find("project-brief").expect("project-brief in gallery");
        let dest = install(t, "", &root).expect("writes the file");
        assert_eq!(dest, PathBuf::from("project-brief.md"));
        assert!(root.join("project-brief.md").exists());
    }
}
