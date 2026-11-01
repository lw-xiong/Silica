//! Native core plugins ported from Obsidian.
//!
//! Each plugin implements [`crate::native::Plugin`] and registers itself
//! via [`crate::native::register_native`] at startup.

use crate::live::register_native;
use crate::native::{NativeCommand, Plugin, PluginContext};
use serde_json::Value;
use std::time::SystemTime;

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
                let today = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| {
                        let secs = d.as_secs();
                        let days = secs / 86400;
                        let year = 1970u32 + (days / 365) as u32;
                        let day_of_year = days % 365;
                        let month = 1u32 + (day_of_year / 30) as u32;
                        let day = 1u32 + (day_of_year % 30) as u32;
                        format!("{year:04}-{month:02}-{day:02}")
                    })
                    .unwrap_or_else(|_| "unknown".to_string());

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
                            let link_target = link
                                .get("link")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if link_target == current {
                                backlinks.push(file.clone());
                            }
                        }
                    }
                }
                if backlinks.is_empty() {
                    Ok(())
                } else {
                    Ok(())
                }
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
                let metadata = (ctx.metadata_cache)(&current)?;
                let meta: serde_json::Value =
                    serde_json::from_str(&metadata).map_err(|e| e.to_string())?;
                let links = meta
                    .get("links")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let targets: Vec<String> = links
                    .iter()
                    .filter_map(|l| l.get("link").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .collect();
                if targets.is_empty() {
                    Ok(())
                } else {
                    Ok(())
                }
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
                let md_files: Vec<String> = files
                    .into_iter()
                    .filter(|f| f.ends_with(".md"))
                    .collect();
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
                let _words = content.split_whitespace().count();
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
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| {
                        let secs = d.as_secs();
                        let days = secs / 86400;
                        let year = 1970u32 + (days / 365) as u32;
                        let day_of_year = days % 365;
                        let month = 1u32 + (day_of_year / 30) as u32;
                        let day = 1u32 + (day_of_year % 30) as u32;
                        format!("{year:04}-{month:02}-{day:02}")
                    })
                    .unwrap_or_else(|_| "unknown".to_string());
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

/// Web viewer: open external links.
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
                Err(format!(
                    "Web viewer: would open '{url}' (not yet implemented)"
                ))
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Publish: host notes online (placeholder).
pub struct Publish;

impl Plugin for Publish {
    fn id(&self) -> &'static str {
        "silica:publish"
    }

    fn name(&self) -> &'static str {
        "Publish"
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
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "publish:publish" => Err("Publish is not yet implemented".to_string()),
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Slides: create presentation from notes.
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
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "slides:create" => Err("Slides is not yet implemented".to_string()),
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// File recovery: restore recent snapshots (placeholder).
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
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "file-recovery:restore" => {
                Err("File recovery is not yet implemented".to_string())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Audio recorder: record audio notes (placeholder).
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
                Err("Audio recorder is not yet implemented".to_string())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Bookmarks: save shortcuts (placeholder).
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
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "bookmarks:show" => Err("Bookmarks is not yet implemented".to_string()),
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Footnotes view: show footnotes (placeholder).
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
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "footnotes:show" => {
                Err("Footnotes view is not yet implemented".to_string())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Tag pane: show all tags (placeholder).
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
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "tag-pane:show" => Err("Tag pane is not yet implemented".to_string()),
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Workspaces: save/load layouts (placeholder).
pub struct Workspaces;

impl Plugin for Workspaces {
    fn id(&self) -> &'static str {
        "silica:workspaces"
    }

    fn name(&self) -> &'static str {
        "Workspaces"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "workspaces:save".into(),
            name: "Save workspace".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "workspaces:save" => Err("Workspaces is not yet implemented".to_string()),
            _ => Err(format!("'{id}' is not a command this plugin registered")),
        }
    }
}

/// Slash commands: trigger commands via slash (placeholder).
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
        _value: Option<&Value>,
        _ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        match id {
            "slash-command:trigger" => {
                Err("Slash commands is not yet implemented".to_string())
            }
            _ => Err(format!("'{id}' is not a command this plugin registered")),
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
}
