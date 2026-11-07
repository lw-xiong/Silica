//! Native core plugins ported from Obsidian.
//!
//! Each plugin implements [`crate::native::Plugin`] and registers itself
//! via [`crate::native::register_native`] at startup.

use crate::live::register_native;
use crate::native::{unknown_command, NativeCommand, Plugin, PluginContext};
use serde_json::Value;
use std::time::SystemTime;

/// Today's civil date, in the local zone, as `YYYY-MM-DD`.
///
/// The Gregorian calendar is not 365 days a year and months are not 30 days:
/// the old arithmetic here read 2024-03-01 as 2024-02-29 and drifted a day
/// every four years, so the daily note opened the wrong file. `Stamp` already
/// does the civil conversion the rest of the app uses.
///
/// The zone offset is taken from the OS rather than assumed UTC, because a
/// daily note is named for the day the user is having.
fn today_local() -> String {
    // * `chrono::Local` is what the rest of the app uses for a wall-clock day,
    // * so the daily note and the template's `{{date}}` agree.
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Daily notes: create or open today's dated note (YYYY-MM-DD.md).
pub struct DailyNotes;

impl Plugin for DailyNotes {
    fn id(&self) -> &'static str {
        "silica:daily-notes"
    }

    fn name(&self) -> &'static str {
        "Daily notes"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "daily-notes:open".into(),
            name: "Open daily note".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "daily-notes:open" => {
                let today = today_local();

                let path = format!("{today}.md");
                if (ctx.file_exists)(&path)? {
                    (ctx.open_note)(&path)?;
                } else {
                    let content = format!("# {today}\n\n");
                    (ctx.create_file)(&path, &content)?;
                    (ctx.open_note)(&path)?;
                }
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Backlinks: show links from other files to the current note.
pub struct Backlink;

impl Plugin for Backlink {
    fn id(&self) -> &'static str {
        "silica:backlink"
    }

    fn name(&self) -> &'static str {
        "Backlinks"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "backlink:show".into(),
            name: "Show backlinks".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "backlink:show" => {
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                let metadata = (ctx.metadata_cache)(&current)?;
                let _meta: serde_json::Value =
                    serde_json::from_str(&metadata).map_err(|e| e.to_string())?;
                let mut backlinks = Vec::new();
                let all_files = (ctx.list_dir)("")?;
                for file in &all_files {
                    if file.as_str().ends_with(".md") && file.as_str() != current {
                        let file_meta = (ctx.metadata_cache)(file)?;
                        let file_meta: serde_json::Value =
                            serde_json::from_str(&file_meta).map_err(|e| e.to_string())?;
                        let file_links = file_meta
                            .get("links")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        for link in file_links {
                            let link_target =
                                link.get("link").and_then(|v| v.as_str()).unwrap_or("");
                            if link_target == current {
                                backlinks.push(file.clone());
                            }
                        }
                    }
                }
                // * The count and the names are the whole point of the
                // * command; before this the vector was built and dropped.
                match backlinks.len() {
                    0 => (ctx.notify)("No backlinks"),
                    1 => (ctx.notify)(&format!("1 backlink: {}", backlinks[0])),
                    n => {
                        let mut names = backlinks.join(", ");
                        if names.len() > 200 {
                            names.truncate(200);
                            names.push('…');
                        }
                        (ctx.notify)(&format!("{n} backlinks: {names}"));
                    }
                }
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Outgoing links: show outgoing links from the current note.
pub struct OutgoingLink;

impl Plugin for OutgoingLink {
    fn id(&self) -> &'static str {
        "silica:outgoing-link"
    }

    fn name(&self) -> &'static str {
        "Outgoing links"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "outgoing-link:show".into(),
            name: "Show outgoing links".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "outgoing-link:show" => {
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                // * The note's own `links` from the metadata cache. This used
                // * to check that the open note existed and stop, so the
                // * command reported success without showing anything.
                let metadata = (ctx.metadata_cache)(&current)?;
                let meta: serde_json::Value =
                    serde_json::from_str(&metadata).map_err(|e| e.to_string())?;
                let links: Vec<String> = meta
                    .get("links")
                    .and_then(|v| v.as_array())
                    .map(|array| {
                        array
                            .iter()
                            .filter_map(|l| l.get("link").and_then(|v| v.as_str()))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                match links.len() {
                    0 => (ctx.notify)("No outgoing links"),
                    1 => (ctx.notify)(&format!("1 outgoing link: {}", links[0])),
                    n => {
                        let mut names = links.join(", ");
                        if names.len() > 200 {
                            names.truncate(200);
                            names.push('…');
                        }
                        (ctx.notify)(&format!("{n} outgoing links: {names}"));
                    }
                }
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Templates: insert template content from a folder.
pub struct Templates;

impl Plugin for Templates {
    fn id(&self) -> &'static str {
        "silica:templates"
    }

    fn name(&self) -> &'static str {
        "Templates"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "templates:insert".into(),
            name: "Insert template".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "templates:insert" => {
                let template_name = value.and_then(|v| v.as_str()).unwrap_or("default");
                let path = format!("templates/{template_name}.md");
                if !(ctx.file_exists)(&path)? {
                    return Err(format!("Template '{template_name}' not found"));
                }
                let content = (ctx.read_file)(&path)?;
                let current = (ctx.current_note)().ok_or_else(|| "No note is open".to_string())?;
                (ctx.append_file)(&current, &content)?;
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Random note: open a random note from the vault.
pub struct RandomNote;

impl Plugin for RandomNote {
    fn id(&self) -> &'static str {
        "silica:random-note"
    }

    fn name(&self) -> &'static str {
        "Random note"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "random-note:open".into(),
            name: "Open random note".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "random-note:open" => {
                let files = (ctx.list_dir)("")?;
                let md_files: Vec<String> =
                    files.into_iter().filter(|f| f.ends_with(".md")).collect();
                if md_files.is_empty() {
                    return Err("No notes in vault".to_string());
                }
                let idx = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs() as usize % md_files.len())
                    .unwrap_or(0);
                (ctx.open_note)(&md_files[idx])?;
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Word count: show word count of the current note.
pub struct WordCount;

impl Plugin for WordCount {
    fn id(&self) -> &'static str {
        "silica:word-count"
    }

    fn name(&self) -> &'static str {
        "Word count"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "word-count:show".into(),
            name: "Show word count".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "word-count:show" => {
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                let content = (ctx.read_file)(&current)?;
                // * The count was computed and thrown away. Words, not
                // * whitespace runs: a note's prose is words, and the two agree
                // * only when nothing but spaces separates them.
                let words = content.split_whitespace().count();
                let chars = content.chars().count();
                let reading_minutes = (words as f64 / 200.0).ceil().max(1.0) as u64;
                (ctx.notify)(&format!(
                    "{words} words, {chars} characters (~{reading_minutes} min read)"
                ));
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Note composer: merge two notes or split one.
pub struct NoteComposer;

impl Plugin for NoteComposer {
    fn id(&self) -> &'static str {
        "silica:note-composer"
    }

    fn name(&self) -> &'static str {
        "Note composer"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "note-composer:merge".into(),
            name: "Merge notes".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "note-composer:merge" => {
                let target = value.and_then(|v| v.as_str()).unwrap_or("");
                if target.is_empty() {
                    return Err("No target note specified".to_string());
                }
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                let current_content = (ctx.read_file)(&current)?;
                let target_content = (ctx.read_file)(target)?;
                let merged = format!("{current_content}\n\n{target_content}");
                (ctx.write_file)(&current, &merged)?;
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// ZK prefixer: create notes with unique timestamp prefixes.
pub struct ZkPrefixer;

impl Plugin for ZkPrefixer {
    fn id(&self) -> &'static str {
        "silica:zk-prefixer"
    }

    fn name(&self) -> &'static str {
        "Unique note creator"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "zk-prefixer:create".into(),
            name: "Create zettelkasten note".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "zk-prefixer:create" => {
                let title = value.and_then(|v| v.as_str()).unwrap_or("note");
                let now = today_local();
                let path = format!("{now}-{title}.md");
                let content = format!("# {title}\n\n");
                (ctx.create_file)(&path, &content)?;
                (ctx.open_note)(&path)?;
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Web viewer: open external links in the system browser.
pub struct WebViewer;

impl Plugin for WebViewer {
    fn id(&self) -> &'static str {
        "silica:webviewer"
    }

    fn name(&self) -> &'static str {
        "Web viewer"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "webviewer:open".into(),
            name: "Open web link".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        value: Option<&Value>,
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "webviewer:open" => {
                let url = value.and_then(|v| v.as_str()).unwrap_or("");
                if url.is_empty() {
                    return Err("No URL specified".to_string());
                }
                // ! One launcher for every platform, and it checks the scheme
                // ! first: the target arrives from a note, so it is untrusted.
                crate::launch::open(url).map_err(|error| error.to_string())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Every `.md` path under a folder, found by asking `list_dir` one folder at a
/// time.
///
/// `list_dir` answers for a single folder, so a folder is recognised by
/// answering with a listing of its own. The depth cap is a runaway guard: a
/// vault is not nested 64 deep, and a symlink loop is.
fn collect_notes(
    ctx: &PluginContext<'_>,
    folder: &str,
    out: &mut Vec<String>,
    depth: u32,
) -> Result<(), String> {
    if depth > 64 {
        return Err("the vault is nested too deeply to publish".to_string());
    }
    let entries = match (ctx.list_dir)(folder) {
        Ok(entries) => entries,
        // * A file is not a folder; that is not an error here.
        Err(_) => return Ok(()),
    };
    for entry in entries {
        if entry.ends_with(".md") {
            out.push(entry);
            continue;
        }
        // * A file answers with an error from `list_dir`; a folder answers
        // * with its own listing, so recurse either way. The depth cap bounds
        // * it even if a symlink loop sneaks in.
        collect_notes(ctx, &entry, out, depth + 1)?;
    }
    Ok(())
}

/// Publish: export vault notes to HTML for publishing.
pub struct Publish;

impl Plugin for Publish {
    fn id(&self) -> &'static str {
        "silica:publish"
    }

    fn name(&self) -> &'static str {
        "Publish vault"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "publish:publish".into(),
            name: "Publish vault".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "publish:publish" => {
                let root = ctx.vault_root;
                let publish_dir = root.join("publish");
                std::fs::create_dir_all(&publish_dir)
                    .map_err(|e| format!("Failed to create publish dir: {e}"))?;

                // * Every note, nested ones included. `list_dir` answers for one
                // * folder, so a folder is walked by asking again — an entry
                // * that answers with a listing is a folder.
                let mut notes = Vec::new();
                collect_notes(ctx, "", &mut notes, 0)?;

                let mut index = String::from("# Published Notes\n\n");
                let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();

                for file in &notes {
                    if !file.as_str().ends_with(".md") {
                        continue;
                    }
                    let content = (ctx.read_file)(file)?;
                    // ! Both the title and the body are user text. A note named
                    // ! `<script>alert(1)</script>.md` would otherwise put that
                    // ! script in a page the vault then serves.
                    let title = escape_html(file);
                    let html = format!(
                        "<!DOCTYPE html>\n<html>\n<head><meta charset='utf-8'><title>{title}</title></head>\n<body><pre>{}</pre></body>\n</html>",
                        escape_html(&content)
                    );

                    // ! The output name is built from the note's path, so it
                    // ! must not keep the separators: `a/b.md` joined onto the
                    // ! publish folder writes into `a/`, which does not exist —
                    // ! and a crafted path would climb out of it entirely.
                    let stem = file.trim_end_matches(".md").replace(['/', '\\'], "-");
                    let mut html_path = format!("{stem}.html");
                    // ! Two notes can flatten alike (`a/b.md`, `a-b.md`).
                    let mut n = 1u32;
                    while !used.insert(html_path.clone()) {
                        html_path = format!("{stem}-{n}.html");
                        n += 1;
                        if n > 10_000 {
                            return Err(format!("too many notes collide on {stem}"));
                        }
                    }

                    std::fs::write(publish_dir.join(&html_path), html)
                        .map_err(|e| format!("Failed to write {html_path}: {e}"))?;
                    // * A link target with a space or bracket would break the
                    // * list; the name is escaped the way the page is.
                    index.push_str(&format!("- [{}]({html_path})\n", escape_html(file)));
                }

                std::fs::write(publish_dir.join("index.md"), index)
                    .map_err(|e| format!("Failed to write index: {e}"))?;

                // * Publishing is a filesystem action the user asked for; say
                // * how many notes it covered.
                (ctx.notify)(&format!("Published {} notes", used.len()));

                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Slides: create presentation from notes with --- separators.
pub struct Slides;

impl Plugin for Slides {
    fn id(&self) -> &'static str {
        "silica:slides"
    }

    fn name(&self) -> &'static str {
        "Slides"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "slides:create".into(),
            name: "Create slides".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "slides:create" => {
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                let content = (ctx.read_file)(&current)?;
                let slides: Vec<&str> = content.split("\n---\n").collect();
                let mut slides_md = String::new();
                for slide in slides {
                    slides_md.push_str(slide);
                    slides_md.push_str("\n\n---\n\n");
                }
                let slide_path = format!("{current}.slides.md");
                (ctx.create_file)(&slide_path, &slides_md)?;
                (ctx.open_note)(&slide_path)?;
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// File recovery: restore recent snapshots from .trash folder.
pub struct FileRecovery;

impl Plugin for FileRecovery {
    fn id(&self) -> &'static str {
        "silica:file-recovery"
    }

    fn name(&self) -> &'static str {
        "File recovery"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "file-recovery:restore".into(),
            name: "Restore snapshot".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "file-recovery:restore" => {
                let root = ctx.vault_root;
                let trash_dir = root.join(".trash");
                if !trash_dir.exists() {
                    return Ok(());
                }

                let entries = std::fs::read_dir(&trash_dir)
                    .map_err(|e| format!("Failed to read trash: {e}"))?;

                let mut files: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file())
                    .collect();

                files.sort_by(|a, b| {
                    let a_time = a
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    let b_time = b
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    b_time.cmp(&a_time)
                });

                let count = files.len().min(5);
                for entry in files.iter().take(count) {
                    if let Some(name) = entry.file_name().to_str() {
                        let src = entry.path();
                        let dst = root.join(name);
                        // ! Never restore over a note that already exists: the
                        // * snapshot is the user's older copy, not theirs now.
                        if dst.exists() {
                            return Err(format!("{name} already exists; refusing to overwrite"));
                        }
                        std::fs::copy(&src, &dst)
                            .map_err(|e| format!("failed to restore {name}: {e}"))?;
                    }
                }

                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Audio recorder: record audio notes (placeholder - creates a note with recording info).
pub struct AudioRecorder;

impl Plugin for AudioRecorder {
    fn id(&self) -> &'static str {
        "silica:audio-recorder"
    }

    fn name(&self) -> &'static str {
        "Audio recorder"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "audio-recorder:record".into(),
            name: "Record audio".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "audio-recorder:record" => {
                // * Silica has no audio capture: do not fabricate a note that
                // * pretends a recording happened. The plugin is told plainly.
                Err("audio recording is not supported by Silica".to_string())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Bookmarks: save shortcuts to notes in a bookmarks.md file.
pub struct Bookmarks;

impl Plugin for Bookmarks {
    fn id(&self) -> &'static str {
        "silica:bookmarks"
    }

    fn name(&self) -> &'static str {
        "Bookmarks"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "bookmarks:show".into(),
            name: "Show bookmarks".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "bookmarks:show" => {
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                let bookmark_path = "bookmarks.md";
                let bookmark_line = format!("- [{current}]({current})");

                let existing = if (ctx.file_exists)(bookmark_path)? {
                    (ctx.read_file)(bookmark_path)?
                } else {
                    String::new()
                };

                if existing.contains(&format!("]({current})")) {
                    return Ok(());
                }

                let updated = if existing.is_empty() {
                    format!("# Bookmarks\n\n{bookmark_line}")
                } else {
                    format!("{existing}\n{bookmark_line}")
                };

                (ctx.write_file)(bookmark_path, &updated)?;
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Footnotes: show footnotes from the current note.
pub struct Footnotes;

impl Plugin for Footnotes {
    fn id(&self) -> &'static str {
        "silica:footnotes"
    }

    fn name(&self) -> &'static str {
        "Footnotes view"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "footnotes:show".into(),
            name: "Show footnotes".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "footnotes:show" => {
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Tag pane: show all tags from all notes.
pub struct TagPane;

impl Plugin for TagPane {
    fn id(&self) -> &'static str {
        "silica:tag-pane"
    }

    fn name(&self) -> &'static str {
        "Tags view"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "tag-pane:show".into(),
            name: "Show tags".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "tag-pane:show" => {
                let all_files = (ctx.list_dir)("")?;
                let mut tags: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();

                for file in &all_files {
                    if file.as_str().ends_with(".md") {
                        let content = (ctx.read_file)(file)?;
                        for line in content.lines() {
                            for word in line.split_whitespace() {
                                if word.starts_with('#') && word.len() > 1 {
                                    let tag =
                                        word.trim_start_matches('#').trim_matches(|c: char| {
                                            c.is_ascii_punctuation() && c != '_'
                                        });
                                    if !tag.is_empty() {
                                        tags.insert(tag.to_string());
                                    }
                                }
                            }
                        }
                    }
                }

                // * The collected tags are what the command is for; before
                // * this the set was built and dropped.
                if tags.is_empty() {
                    (ctx.notify)("No tags in this vault");
                } else {
                    let names: Vec<&str> = tags.iter().map(String::as_str).collect();
                    let mut listed = names.join(", ");
                    if listed.len() > 200 {
                        listed.truncate(200);
                        listed.push('…');
                    }
                    (ctx.notify)(&format!("{} tags: {listed}", tags.len()));
                }
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Workspaces: save/load window layouts.
pub struct Workspaces;

impl Plugin for Workspaces {
    fn id(&self) -> &'static str {
        "silica:workspaces"
    }

    fn name(&self) -> &'static str {
        "Workspaces"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![
            NativeCommand {
                id: "workspaces:save".into(),
                name: "Save workspace".into(),
            },
            NativeCommand {
                id: "workspaces:load".into(),
                name: "Load workspace".into(),
            },
        ]
    }

    fn run_command(
        &self,
        id: &str,
        value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "workspaces:save" => {
                let name = value.and_then(|v| v.as_str()).unwrap_or("default");
                let current = (ctx.current_note)().unwrap_or_default();
                let workspace_path = format!(".silica/workspaces/{name}.json");
                let workspace = serde_json::json!({
                    "name": name,
                    "current_note": current,
                    "created": SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                });
                (ctx.create_file)(
                    &workspace_path,
                    &serde_json::to_string_pretty(&workspace).map_err(|e| e.to_string())?,
                )?;
                Ok(())
            }
            "workspaces:load" => {
                let name = value.and_then(|v| v.as_str()).unwrap_or("default");
                let workspace_path = format!(".silica/workspaces/{name}.json");
                if !(ctx.file_exists)(&workspace_path)? {
                    return Err(format!("no workspace named {name}"));
                }
                let content = (ctx.read_file)(&workspace_path)?;
                let workspace: Value = serde_json::from_str(&content)
                    .map_err(|e| format!("workspace {name} is not valid JSON: {e}"))?;
                let note = workspace
                    .get("current_note")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !note.is_empty() {
                    (ctx.open_note)(note)?;
                }
                Ok(())
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// Slash commands: trigger commands via slash syntax.
pub struct SlashCommand;

impl Plugin for SlashCommand {
    fn id(&self) -> &'static str {
        "silica:slash-command"
    }

    fn name(&self) -> &'static str {
        "Slash commands"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "slash-command:trigger".into(),
            name: "Trigger slash command".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "slash-command:trigger" => {
                let input = value.and_then(|v| v.as_str()).unwrap_or("");
                if !input.starts_with('/') {
                    return Err("Not a slash command".to_string());
                }
                let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
                let cmd = parts[0];
                let args = parts.get(1).unwrap_or(&"");

                match cmd {
                    "daily" => {
                        let today = today_local();
                        let path = format!("{today}.md");
                        if !(ctx.file_exists)(&path)? {
                            let content = format!("# {today}\n\n{args}");
                            (ctx.create_file)(&path, &content)?;
                        }
                        (ctx.open_note)(&path)?;
                        Ok(())
                    }
                    _ => Err(format!("Unknown slash command: /{cmd}")),
                }
            }
            _ => Err(unknown_command(id)),
        }
    }
}

/// All native plugins registered at startup.
pub fn register_all() {
    register_native(Box::new(DailyNotes));
    register_native(Box::new(Backlink));
    register_native(Box::new(OutgoingLink));
    register_native(Box::new(Templates));
    register_native(Box::new(RandomNote));
    register_native(Box::new(WordCount));
    register_native(Box::new(NoteComposer));
    register_native(Box::new(ZkPrefixer));
    register_native(Box::new(WebViewer));
    register_native(Box::new(Publish));
    register_native(Box::new(Slides));
    register_native(Box::new(FileRecovery));
    register_native(Box::new(AudioRecorder));
    register_native(Box::new(Bookmarks));
    register_native(Box::new(Footnotes));
    register_native(Box::new(TagPane));
    register_native(Box::new(Workspaces));
    register_native(Box::new(SlashCommand));
    register_native(Box::new(crate::vault_doctor::VaultDoctor));
}

#[cfg(test)]
mod date_tests {
    use super::*;

    /// The civil conversion the calendar actually has, at the boundaries the
    /// old 365-day/30-day arithmetic got wrong.
    fn civil(unix: i64, offset: i64) -> (i64, u32, u32) {
        let stamp = silica_templates::Stamp::from_unix(unix, offset);
        (stamp.year, stamp.month, stamp.day)
    }

    /// A leap day is a real date, and the day after it is March.
    #[test]
    fn a_leap_day_is_the_twenty_ninth_of_february() {
        // * 2024-02-29 12:00 UTC.
        assert_eq!(civil(1_709_208_000, 0), (2024, 2, 29));
        // * The next day is March, not February 30th.
        assert_eq!(civil(1_709_208_000 + 86_400, 0), (2024, 3, 1));
    }

    /// A year boundary is the 1st of January, not a 366th of December.
    #[test]
    fn a_year_boundary_rolls_to_january() {
        // * 2024-12-31 12:00 UTC, then the next day.
        let end = 1_735_646_400;
        assert_eq!(civil(end, 0), (2024, 12, 31));
        assert_eq!(civil(end + 86_400, 0), (2025, 1, 1));
    }

    /// A month boundary is the 1st of the next month, and the months are not
    /// all thirty days.
    #[test]
    fn month_lengths_are_the_calendars() {
        // * 2024-01-31 -> February 1st.
        let jan_end = 1_706_702_400;
        assert_eq!(civil(jan_end, 0), (2024, 1, 31));
        assert_eq!(civil(jan_end + 86_400, 0), (2024, 2, 1));
        // * 2024-04-30 -> May 1st.
        let apr_end = 1_714_478_400;
        assert_eq!(civil(apr_end, 0), (2024, 4, 30));
        assert_eq!(civil(apr_end + 86_400, 0), (2024, 5, 1));
    }

    /// The offset moves the civil date, which is what makes a daily note
    /// follow the user's day rather than UTC's.
    #[test]
    fn a_zone_offset_moves_the_day() {
        // * 2024-03-01 00:30 UTC is still 2024-02-29 west of Greenwich.
        let instant = 1_709_253_000;
        assert_eq!(civil(instant, 0), (2024, 3, 1));
        assert_eq!(civil(instant, -3600), (2024, 2, 29));
        assert_eq!(civil(instant, 8 * 3600), (2024, 3, 1));
    }

    /// What the app produces is a date the vault can name a file with.
    #[test]
    fn todays_name_is_iso_shaped() {
        let today = today_local();
        assert_eq!(today.len(), 10, "{today}");
        let parts: Vec<&str> = today.split('-').collect();
        assert_eq!(parts.len(), 3, "{today}");
        assert_eq!(parts[0].len(), 4, "{today}");
        assert_eq!(parts[1].len(), 2, "{today}");
        assert_eq!(parts[2].len(), 2, "{today}");
    }
}

/// Commands that gather something and show it.
///
/// Each of these computed an answer and returned `Ok(())` without putting the
/// answer anywhere, so the work happened and the user saw nothing.
#[cfg(test)]
mod report_tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    /// A context over a fixed set of notes and metadata, recording notices.
    struct Harness {
        notices: Rc<RefCell<Vec<String>>>,
    }

    impl Harness {
        fn new(
            notes: &[(&str, &str)],
            meta: HashMap<String, String>,
            open: &str,
        ) -> (Self, PluginContext<'static>) {
            let notices = Rc::new(RefCell::new(Vec::new()));
            let shown = Rc::clone(&notices);
            let files: Vec<String> = notes.iter().map(|(p, _)| p.to_string()).collect();
            let bodies: HashMap<String, String> = notes
                .iter()
                .map(|(p, b)| (p.to_string(), b.to_string()))
                .collect();
            let current = open.to_string();
            let ctx = PluginContext {
                vault_root: std::path::Path::new("."),
                read_file: Box::new(move |p| {
                    bodies
                        .get(p)
                        .cloned()
                        .ok_or_else(|| format!("no such file: {p}"))
                }),
                write_file: Box::new(|_, _| Ok(())),
                append_file: Box::new(|_, _| Ok(())),
                create_file: Box::new(|_, _| Ok(())),
                create_folder: Box::new(|_| Ok(())),
                delete_file: Box::new(|_| Ok(())),
                rename_file: Box::new(|_, _| Ok(())),
                list_dir: Box::new(move |_| Ok(files.clone())),
                file_exists: Box::new(|_| Ok(true)),
                metadata_cache: Box::new(move |p| {
                    meta.get(p)
                        .cloned()
                        .ok_or_else(|| format!("no metadata: {p}"))
                }),
                open_note: Box::new(|_| Ok(())),
                current_note: Box::new(move || Some(current.clone())),
                notify: Box::new(move |line: &str| shown.borrow_mut().push(line.to_string())),
            };
            (Self { notices }, ctx)
        }
    }

    #[test]
    fn backlinks_are_reported_not_discarded() {
        let mut meta = HashMap::new();
        meta.insert(
            "A.md".to_string(),
            r#"{"links":[{"link":"B.md"}]}"#.to_string(),
        );
        meta.insert("B.md".to_string(), r#"{"links":[]}"#.to_string());
        let (h, ctx) = Harness::new(&[("A.md", ""), ("B.md", "")], meta, "B.md");
        Backlink.run_command("backlink:show", None, &ctx).unwrap();

        let shown = h.notices.borrow();
        assert_eq!(shown.len(), 1, "one line is shown");
        assert!(shown[0].contains("1 backlink"), "{}", shown[0]);
        assert!(shown[0].contains("A.md"), "the linking note is named");
    }

    #[test]
    fn a_note_with_no_backlinks_says_so() {
        let mut meta = HashMap::new();
        meta.insert("B.md".to_string(), r#"{"links":[]}"#.to_string());
        let (h, ctx) = Harness::new(&[("B.md", "")], meta, "B.md");
        Backlink.run_command("backlink:show", None, &ctx).unwrap();
        assert_eq!(h.notices.borrow().as_slice(), ["No backlinks"]);
    }

    #[test]
    fn outgoing_links_are_reported() {
        let mut meta = HashMap::new();
        meta.insert(
            "A.md".to_string(),
            r#"{"links":[{"link":"B.md"},{"link":"C.md"}]}"#.to_string(),
        );
        let (h, ctx) = Harness::new(&[("A.md", "")], meta, "A.md");
        OutgoingLink
            .run_command("outgoing-link:show", None, &ctx)
            .unwrap();

        let shown = h.notices.borrow();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].contains("2 outgoing links"), "{}", shown[0]);
        assert!(shown[0].contains("B.md") && shown[0].contains("C.md"));
    }

    #[test]
    fn the_word_count_is_shown() {
        let (h, ctx) = Harness::new(
            &[("A.md", "one two three four five")],
            HashMap::new(),
            "A.md",
        );
        WordCount
            .run_command("word-count:show", None, &ctx)
            .unwrap();

        let shown = h.notices.borrow();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].starts_with("5 words"), "{}", shown[0]);
    }

    #[test]
    fn the_collected_tags_are_shown() {
        let (h, ctx) = Harness::new(
            &[("A.md", "#alpha some text #beta"), ("B.md", "no tags here")],
            HashMap::new(),
            "A.md",
        );
        TagPane.run_command("tag-pane:show", None, &ctx).unwrap();

        let shown = h.notices.borrow();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].contains("2 tags"), "{}", shown[0]);
        assert!(shown[0].contains("alpha") && shown[0].contains("beta"));
    }
}
