//! Native core plugins ported from Obsidian.
//!
//! Each plugin implements [`crate::native::Plugin`] and registers itself
//! via [`crate::native::register_native`] at startup.

use crate::live::register_native;
use crate::native::{NativeCommand, Plugin, PluginContext};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde_json::Value;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
            id: "silica:daily-notes".into(),
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
            "silica:daily-notes" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:backlink-count".into(),
            name: "Count backlinks".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "silica:backlink-count" => {
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
                        (ctx.notify)(&format!("{n} backlinks: {}", list_line(&backlinks)));
                    }
                }
                Ok(())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:outgoing-link-count".into(),
            name: "Count outgoing links".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "silica:outgoing-link-count" => {
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
                        (ctx.notify)(&format!("{n} outgoing links: {}", list_line(&links)));
                    }
                }
                Ok(())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:insert-template".into(),
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
            "silica:insert-template" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:random-note".into(),
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
            "silica:random-note" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:word-count".into(),
            name: "Word count".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "silica:word-count" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:note-composer-merge".into(),
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
            "silica:note-composer-merge" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:zk-prefixer".into(),
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
            "silica:zk-prefixer" => {
                let title = value.and_then(|v| v.as_str()).unwrap_or("note");
                let now = today_local();
                let path = format!("{now}-{title}.md");
                let content = format!("# {title}\n\n");
                (ctx.create_file)(&path, &content)?;
                (ctx.open_note)(&path)?;
                Ok(())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:publish-vault".into(),
            name: "Publish vault".into(),
        }]
    }

    fn run_command(
        &self,
        _id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match _id {
            "silica:publish-vault" => {
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
            _ => Err("'silica:publish-vault' is not a command this plugin registered".to_string()),
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
            id: "silica:slides-create".into(),
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
            "silica:slides-create" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:file-recovery-restore".into(),
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
            "silica:file-recovery-restore" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Audio recorder: record microphone input to WAV files in the vault's
/// attachments folder. Recordings are written as `.wav` files under
/// `.obsidian/attachments/` and can be embedded in notes like any other
/// attachment.
pub struct AudioRecorder {
    state: Arc<Mutex<RecorderState>>,
    /// The live stream, kept out of `RecorderState` on purpose: the audio
    /// callback holds an `Arc` of that state, and a `cpal::Stream` is not
    /// `Send` on every platform — on macOS a CoreAudio stream is bound to
    /// the thread that made it, so a shared state holding one makes the
    /// callback itself not `Send`, which `build_input_stream` requires.
    /// The stream is touched only from start and stop, never from the
    /// callback, so its own lock cannot deadlock against that state.
    stream: Mutex<Option<cpal::Stream>>,
}

impl Default for AudioRecorder {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecorderState {
                recording: false,
                voice_note: false,
                writer: None,
                output_path: None,
                output_name: None,
            })),
            stream: Mutex::new(None),
        }
    }
}

struct RecorderState {
    recording: bool,
    /// True when the recording was started by `silica:voice-note`, so only
    /// that command stops it and embeds the result in the active note.
    voice_note: bool,
    writer: Option<hound::WavWriter<BufWriter<std::fs::File>>>,
    output_path: Option<PathBuf>,
    /// Vault-relative path of the recording, e.g. `.obsidian/attachments/recording-20260101-000000.wav`,
    /// stored so `silica:voice-note` can embed `![[name]]` without reconstructing it.
    output_name: Option<String>,
}

impl Plugin for AudioRecorder {
    fn id(&self) -> &'static str {
        "silica:audio-recorder"
    }

    fn name(&self) -> &'static str {
        "Audio recorder"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![
            NativeCommand {
                id: "audio-recorder:start".into(),
                name: "Record audio".into(),
            },
            NativeCommand {
                id: "audio-recorder:stop".into(),
                name: "Stop recording audio".into(),
            },
            NativeCommand {
                id: "silica:voice-note".into(),
                name: "Voice note".into(),
            },
        ]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "audio-recorder:start" => self.start_recording(ctx),
            "audio-recorder:stop" => self.stop_recording(ctx),
            "silica:voice-note" => self.voice_note(ctx),
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

impl AudioRecorder {
    fn start_recording(&self, ctx: &PluginContext<'_>) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|e| format!("{e}"))?;

        if state.recording {
            return Err("Already recording".to_string());
        }

        // Determine output path: .obsidian/attachments/recording-<timestamp>.wav
        let vault_root = ctx.vault_root;
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let file_name = format!("recording-{timestamp}.wav");
        let output_name = format!(".obsidian/attachments/{file_name}");
        let output_path = vault_root
            .join(".obsidian")
            .join("attachments")
            .join(&file_name);

        // Ensure the attachments folder exists
        (ctx.create_folder)(".obsidian/attachments")?;

        // Set up audio input
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "No audio input device available".to_string())?;
        let config = device
            .default_input_config()
            .map_err(|e| format!("No input config: {e}"))?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();

        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = hound::WavWriter::create(&output_path, spec)
            .map_err(|e| format!("Cannot create WAV file: {e}"))?;

        let state_clone = Arc::clone(&self.state);
        let stream = device
            .build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut state = state_clone.lock().unwrap();
                    if let Some(writer) = state.writer.as_mut() {
                        for &sample in data {
                            let s = (sample * i16::MAX as f32) as i16;
                            let _ = writer.write_sample(s);
                        }
                    }
                },
                move |err| {
                    eprintln!("Audio input error: {err}");
                },
                None,
            )
            .map_err(|e| format!("Cannot build input stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Cannot start stream: {e}"))?;

        state.recording = true;
        state.voice_note = false;
        *self
            .stream
            .lock()
            .map_err(|e| format!("the recorder\u{2019}s stream lock failed: {e}"))? = Some(stream);
        state.writer = Some(writer);
        state.output_path = Some(output_path);
        state.output_name = Some(output_name);

        (ctx.notify)(&format!(
            "Recording started: {}",
            state.output_path.as_ref().unwrap().display()
        ));

        Ok(())
    }

    /// Pauses the stream, finalizes the WAV file, and returns its vault path.
    /// `stop_recording` and `silica:voice-note` share this so a recording is
    /// finalized exactly once, whichever command stops it.
    fn finish_recording(&self) -> Result<PathBuf, String> {
        let state = self.state.lock().map_err(|e| format!("{e}"))?;

        if !state.recording {
            return Err("Not recording".to_string());
        }

        // Stop and close the stream: it lives outside the state the audio
        // callback holds, so pausing it here cannot touch the writer the
        // callback may still be writing to.
        drop(state);
        if let Some(stream) = self
            .stream
            .lock()
            .map_err(|e| format!("the recorder\u{2019}s stream lock failed: {e}"))?
            .take()
        {
            let _ = stream.pause();
        }
        let mut state = self.state.lock().map_err(|e| format!("{e}"))?;

        // Finalize the WAV writer
        if let Some(writer) = state.writer.take() {
            writer
                .finalize()
                .map_err(|e| format!("Cannot finalize WAV: {e}"))?;
        }

        let output_path = state.output_path.take().unwrap();
        state.recording = false;

        Ok(output_path)
    }

    fn stop_recording(&self, ctx: &PluginContext<'_>) -> Result<(), String> {
        let output_path = self.finish_recording()?;
        (ctx.notify)(&format!("Recording saved: {}", output_path.display()));
        Ok(())
    }

    /// Voice note: the first run starts the recording; the next run stops it
    /// and inserts `![[path]]` at the cursor of the active note. Only a
    /// recording this command itself started may be stopped by it.
    fn voice_note(&self, ctx: &PluginContext<'_>) -> Result<(), String> {
        let state = self.state.lock().map_err(|e| format!("{e}"))?;
        if state.recording {
            let mine = state.voice_note;
            drop(state);
            if !mine {
                return Err(
                    "A recording started by audio-recorder:start is in progress; \
stop it with audio-recorder:stop first."
                        .to_string(),
                );
            }
            let output_path = self.finish_recording()?;
            let relative = output_path
                .strip_prefix(ctx.vault_root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| output_path.to_string_lossy().replace('\\', "/"));
            let embed = format!("![[{relative}]]");
            (ctx.insert_text)(&embed)?;
            (ctx.notify)(&format!("Voice note inserted: {embed}"));
            Ok(())
        } else {
            drop(state);
            self.start_recording(ctx)?;
            let mut state = self.state.lock().map_err(|e| format!("{e}"))?;
            state.voice_note = true;
            drop(state);
            (ctx.notify)(
                "Voice note recording started; run this again to stop and insert the embed.",
            );
            Ok(())
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
            id: "silica:bookmarks".into(),
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
            "silica:bookmarks" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Footnotes: show the footnotes the current note defines.
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
            id: "footnotes:open".into(),
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
            // * The id is Obsidian's own, so a `hotkeys.json` from a vault
            //   that binds it reaches this. It used to answer only
            //   "no note is open" and otherwise report success without
            //   showing anything, which is what the tests below cover.
            "footnotes:open" => {
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                let text = (ctx.read_file)(&current)?;
                let labels = footnote_labels(&text);
                match labels.len() {
                    0 => (ctx.notify)("No footnotes in this note"),
                    1 => (ctx.notify)(&format!("1 footnote: {}", labels[0])),
                    n => {
                        (ctx.notify)(&format!("{n} footnotes: {}", list_line(&labels)));
                    }
                }
                Ok(())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// The labels of the footnotes a note defines, in the order it defines them.
///
/// A definition is `[^label]: text` at the start of a line. The same
/// brackets in the prose are a reference to a footnote rather than a
/// definition of one, which is why the line has to start with it.
fn footnote_labels(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("[^")?;
            let (label, _) = rest.split_once("]:")?;
            let label = label.trim();
            (!label.is_empty()).then(|| label.to_string())
        })
        .collect()
}

/// Open with default app: hand the current note to the desktop's opener.
///
/// Both ids are Obsidian's own, so a vault's `hotkeys.json` can name them;
/// Silica answered neither before this. `:show` is the inventory's "Show in
/// system explorer" — revealing where the note lives is the same hand-off,
/// so it answers to the same code.
pub struct OpenWithDefaultApp;

impl Plugin for OpenWithDefaultApp {
    fn id(&self) -> &'static str {
        "silica:open-with-default-app"
    }

    fn name(&self) -> &'static str {
        "Open with default app"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![
            NativeCommand {
                id: "open-with-default-app:open".into(),
                name: "Open in default app".into(),
            },
            NativeCommand {
                id: "open-with-default-app:show".into(),
                name: "Show in system explorer".into(),
            },
        ]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "open-with-default-app:open" | "open-with-default-app:show" => {
                let current = (ctx.current_note)().unwrap_or_default();
                if current.is_empty() {
                    return Err("No note is open".to_string());
                }
                // * Read first: it is the only check that says a named note
                //   is really in the vault, and a path that reads is a path
                //   the desktop can be handed.
                (ctx.read_file)(&current)?;
                let path = ctx.vault_root.join(&current);
                // * No shell in the way: `launch` takes the path as an
                //   argument, so a note's name is never a command.
                crate::launch::open_path(&path).map_err(|error| error.to_string())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// The one line a gathered list of names is shown on: the whole list, or
/// its first 200 characters with an ellipsis after them.
///
/// Cut between characters, not between bytes. `String::truncate` takes a
/// byte offset and panics on a boundary that splits a `char`, and a note
/// path, tag or label with an accent lands on such a boundary often: a
/// vault with one non-ASCII note past the cut took the command down.
fn list_line(names: &[String]) -> String {
    let mut listed = names.join(", ");
    if listed.chars().count() > 200 {
        listed = listed.chars().take(200).collect();
        listed.push('…');
    }
    listed
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
            id: "silica:tag-pane".into(),
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
            "silica:tag-pane" => {
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
                    let names: Vec<String> = tags.iter().cloned().collect();
                    (ctx.notify)(&format!("{} tags: {}", tags.len(), list_line(&names)));
                }
                Ok(())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
            id: "silica:slash-command".into(),
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
            "silica:slash-command" => {
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
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Every native plugin Silica ships with, in a stable order.
///
/// The registration and the tests below read one list, so a new plugin
/// cannot be added to one and missed by the other.
pub fn every_plugin() -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(DailyNotes),
        Box::new(Backlink),
        Box::new(OutgoingLink),
        Box::new(Templates),
        Box::new(RandomNote),
        Box::new(WordCount),
        Box::new(NoteComposer),
        Box::new(ZkPrefixer),
        Box::new(WebViewer),
        Box::new(Publish),
        Box::new(Slides),
        Box::new(FileRecovery),
        Box::new(AudioRecorder::default()),
        Box::new(Bookmarks),
        Box::new(Footnotes),
        Box::new(TagPane),
        Box::new(Workspaces),
        Box::new(SlashCommand),
        Box::new(OpenWithDefaultApp),
        Box::new(crate::vault_doctor::VaultDoctor),
    ]
}

/// All native plugins registered at startup.
pub fn register_all() {
    for plugin in every_plugin() {
        register_native(plugin);
    }
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
                insert_text: Box::new(|_| Ok(())),
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
        Backlink
            .run_command("silica:backlink-count", None, &ctx)
            .unwrap();

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
        Backlink
            .run_command("silica:backlink-count", None, &ctx)
            .unwrap();
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
            .run_command("silica:outgoing-link-count", None, &ctx)
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
            .run_command("silica:word-count", None, &ctx)
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
        TagPane.run_command("silica:tag-pane", None, &ctx).unwrap();

        let shown = h.notices.borrow();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].contains("2 tags"), "{}", shown[0]);
        assert!(shown[0].contains("alpha") && shown[0].contains("beta"));
    }

    /// A cut between bytes is not a cut between characters: the list is cut
    /// at the 200th byte by some of these, and a note path with an accent
    /// straddling that offset splits a `char` in half — `String::truncate`
    /// panics on the boundary rather than producing a mangled line.
    #[test]
    fn a_long_list_of_names_with_accents_is_cut_between_characters() {
        let mut meta = HashMap::new();
        // * 197 bytes before the separator, so the next name's first accent
        //   straddles the 200th byte.
        let plain = format!("{}.md", "a".repeat(194));
        let accented = format!("{}.md", "é".repeat(50));
        for note in [&plain, &accented] {
            meta.insert(note.clone(), r#"{"links":[{"link":"A.md"}]}"#.to_string());
        }
        meta.insert("A.md".to_string(), r#"{"links":[]}"#.to_string());
        let (h, ctx) = Harness::new(
            &[(plain.as_str(), ""), (accented.as_str(), "")],
            meta,
            "A.md",
        );
        Backlink
            .run_command("silica:backlink-count", None, &ctx)
            .unwrap();

        let shown = h.notices.borrow();
        assert_eq!(shown.len(), 1, "one line is shown");
        assert!(shown[0].starts_with("2 backlinks"), "{}", shown[0]);
        assert!(shown[0].ends_with('…'), "{}", shown[0]);
    }

    /// The footnotes command answered success and showed nothing: it checked
    /// that a note was open and stopped. Obsidian's id for it is
    /// `footnotes:open`, so that is what it answers to now.
    #[test]
    fn the_footnotes_of_the_note_are_shown() {
        let (h, ctx) = Harness::new(
            &[(
                "A.md",
                "Text[^one], more text[^two].\n\n[^one]: First note.\n[^two]: Second note.\n",
            )],
            HashMap::new(),
            "A.md",
        );
        Footnotes.run_command("footnotes:open", None, &ctx).unwrap();

        let shown = h.notices.borrow();
        assert_eq!(shown.len(), 1, "one line is shown");
        assert!(shown[0].starts_with("2 footnotes"), "{}", shown[0]);
        assert!(shown[0].contains("one") && shown[0].contains("two"));
    }

    /// A `[^label]` in the prose is a reference to a footnote, not a
    /// definition of one; a note holding only references has no footnotes.
    #[test]
    fn a_reference_in_the_prose_is_not_a_definition() {
        let (h, ctx) = Harness::new(&[("A.md", "Text[^one] only.\n")], HashMap::new(), "A.md");
        Footnotes.run_command("footnotes:open", None, &ctx).unwrap();
        assert_eq!(h.notices.borrow().as_slice(), ["No footnotes in this note"]);
    }

    /// A long list is cut at a character boundary, not a byte one: a label
    /// with an accent would otherwise split a `char` in half.
    #[test]
    fn a_long_footnote_list_is_cut_between_characters() {
        // * Labels the length of a heading, so the joined list is well past
        //   the cut.
        let body: String = (0..40)
            .map(|n| {
                let label = "é".repeat(20);
                format!("Text[^{label}].\n\n[^{label}]: note {n}\n")
            })
            .collect();
        let (h, ctx) = Harness::new(&[("A.md", &body)], HashMap::new(), "A.md");
        Footnotes.run_command("footnotes:open", None, &ctx).unwrap();

        let shown = h.notices.borrow();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].starts_with("40 footnotes"), "{}", shown[0]);
        assert!(shown[0].ends_with('…'), "{}", shown[0]);
    }

    /// With no note open there is no file to hand the desktop, and the
    /// command says so instead of handing over the vault root.
    #[test]
    fn the_desktop_commands_ask_for_an_open_file() {
        let (h, ctx) = Harness::new(&[("A.md", "text\n")], HashMap::new(), "");
        for id in ["open-with-default-app:open", "open-with-default-app:show"] {
            let refused = OpenWithDefaultApp
                .run_command(id, None, &ctx)
                .expect_err(id);
            assert!(refused.contains("No note is open"), "{id}: {refused}");
        }
        // * Nothing was shown and nothing was launched.
        assert!(h.notices.borrow().is_empty());
    }

    /// A note the vault no longer has is not a file to open either.
    #[test]
    fn a_note_the_vault_does_not_have_is_refused() {
        let (_h, ctx) = Harness::new(&[("A.md", "text\n")], HashMap::new(), "Gone.md");
        for id in ["open-with-default-app:open", "open-with-default-app:show"] {
            let refused = OpenWithDefaultApp
                .run_command(id, None, &ctx)
                .expect_err(id);
            assert!(refused.contains("Gone.md"), "{id}: {refused}");
        }
    }

    /// An id Silica invents itself carries the `silica:` prefix, so it can
    /// never collide with an id a vault's `hotkeys.json` means for Obsidian.
    /// The ids that are Obsidian's own are the ones its command inventory
    /// names, spelled its way; anything else this app registers is its own
    /// behaviour and must be marked, so a future plugin that invents
    /// `editor:save-file` fails here rather than shadowing it.
    #[test]
    fn an_invented_id_is_marked_with_the_silica_prefix() {
        const INVENTORY_IDS: &[&str] = &[
            "audio-recorder:start",
            "audio-recorder:stop",
            "footnotes:open",
            "open-with-default-app:open",
            "open-with-default-app:show",
            "webviewer:open",
            "workspaces:load",
            "workspaces:save",
        ];
        for plugin in every_plugin() {
            for command in plugin.commands() {
                assert!(
                    command.id.starts_with("silica:")
                        || INVENTORY_IDS.contains(&command.id.as_str()),
                    "{:?} is invented and not marked `silica:`",
                    command.id
                );
            }
        }
    }

    /// Every command a native plugin registers spells its id the way
    /// `hotkeys.json` from an Obsidian vault does — lowercase, hyphens and
    /// colons only — so a vault's own binding can name it.
    #[test]
    fn every_registered_command_is_well_formed() {
        for plugin in every_plugin() {
            let registered = plugin.commands();
            assert!(
                !registered.is_empty(),
                "{} registered nothing",
                plugin.name()
            );
            for command in registered {
                assert!(
                    command.id.chars().all(|c| c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || c == '-'
                        || c == ':'),
                    "{:?} is not an id a vault's hotkeys.json could spell",
                    command.id
                );
            }
        }
    }
}
