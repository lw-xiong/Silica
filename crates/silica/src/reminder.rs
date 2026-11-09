//! Due-date reminders for task lines.
//!
//! A task line carries its date the way vault notes already write it —
//! the Tasks plugin's `📅 2026-11-10` marker (also its `⏳`/`🛫` pair) or
//! a Dataview inline field `[due:: 2026-11-10]` — and both are read off
//! the task index the search pane already builds, so a reminder costs no
//! second walk of the note.
//!
//! The scan runs on a timer while the app is open, and each due task is
//! said once: the keys already announced live in the vault's own config
//! folder, so a restart does not say the same thing twice.
//!
//! Two limits are deliberate, and they are the reason this is not more
//! than it is:
//!
//! * There is no background service, so nothing fires with the app
//!   closed. A date that comes due while Silica is shut is noticed on
//!   the next open instead. Nothing here claims otherwise.
//! * The notice is the app's own transient message — the same one a
//!   command reports through — not a native OS toast, and it expires on
//!   its own. A notification the user must not miss is a different
//!   feature with a different mechanism.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use silica_cache::CachedMetadata;
use silica_markdown::LineIndex;
use silica_vault::Vault;

use crate::search;
use crate::workspace::VaultState;

/// The config file the announced keys live in, inside the vault's own
/// config folder.
const FILE: &str = "reminders.json";

/// How often the due scan runs, while the app is open. A minute is the
/// granularity a reminder is worth; a vault walk per second is not.
pub const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Notices shown per scan before the rest are counted instead: a vault
/// where every task is overdue would otherwise stack the screen.
const MAX_NOTICES: usize = 5;

/// The date markers a vault note already writes. The first is the Tasks
/// plugin's due date; the other two are its scheduled and start dates,
/// which mean the same thing to a reminder.
const EMOJI_MARKERS: [char; 3] = ['📅', '⏳', '🛫'];

/// A task line whose due date has arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTask {
    /// The note's vault-relative path.
    pub path: String,
    /// The zero-based source line in that note.
    pub line: usize,
    /// The task's text, its markers stripped.
    pub text: String,
    /// The date it was due.
    pub due: NaiveDate,
}

impl DueTask {
    /// The key a reminder is announced by: the note, the line and the
    /// date. Editing the task's words does not announce it again;
    /// moving it to another line does, because that is another line.
    fn key(&self) -> String {
        format!("{}|{}|{}", self.path, self.line, self.due)
    }
}

/// The keys already announced, kept in the vault's config folder so a
/// restart does not repeat them. The file is written whole rather than
/// merged: a key that is no longer in it is a task that is no longer
/// due, which is what a merge would never drop.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ReminderStore {
    announced: Vec<String>,
}

impl ReminderStore {
    /// Whether this task's reminder has already been said.
    pub fn is_announced(&self, task: &DueTask) -> bool {
        self.announced.contains(&task.key())
    }

    /// Adds one task to the announced set, keeping the rest: the cursor
    /// command fires one reminder early and the poll must still know
    /// about the ones it said before.
    pub fn announce(&mut self, task: &DueTask) {
        let key = task.key();
        if !self.announced.contains(&key) {
            self.announced.push(key);
        }
    }

    /// Marks every due task announced and returns the ones that were
    /// not announced before — the only ones worth a notice. The set
    /// becomes the due list itself, so a task that stops being due
    /// (ticked off, edited, deleted) loses its key and is announced
    /// again when it comes back.
    pub fn record(&mut self, due: &[DueTask]) -> Vec<DueTask> {
        let known: HashSet<&String> = self.announced.iter().collect();
        let fresh: Vec<DueTask> = due
            .iter()
            .filter(|task| !known.contains(&task.key()))
            .cloned()
            .collect();
        self.announced = due.iter().map(DueTask::key).collect();
        fresh
    }
}

/// Today, local: a due date is a calendar date, and the calendar the
/// dates are written on is the user's.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// The due date on a task line, if it carries one. Both syntaxes vaults
/// write are read; anything else on the line is not a date.
pub fn parse_due(line: &str) -> Option<NaiveDate> {
    inline_field_due(line).or_else(|| emoji_due(line))
}

/// `[due:: 2026-11-10]`, or the parenthesised form. The key is `due`;
/// another inline field is not a due date.
fn inline_field_due(line: &str) -> Option<NaiveDate> {
    for (open, close) in [('[', ']'), ('(', ')')] {
        let mut at = 0usize;
        while let Some(off) = line[at..].find(open) {
            let start = at + off;
            let body = &line[start + open.len_utf8()..];
            let Some(end) = body.find(close) else {
                break;
            };
            let inner = body[..end].trim();
            if let Some(value) = inner.strip_prefix("due::").map(str::trim) {
                if let Some(date) = parse_date(value) {
                    return Some(date);
                }
            }
            at = start + open.len_utf8() + end + close.len_utf8();
        }
    }
    None
}

/// `📅 2026-11-10`: the marker followed by a date, wherever on the line
/// the marker sits.
fn emoji_due(line: &str) -> Option<NaiveDate> {
    for marker in EMOJI_MARKERS {
        let mut at = 0usize;
        while let Some(off) = line[at..].find(marker) {
            let start = at + off;
            let rest = line[start + marker.len_utf8()..].trim_start();
            if let Some(token) = rest.split_whitespace().next() {
                if let Some(date) = parse_date(token) {
                    return Some(date);
                }
            }
            at = start + marker.len_utf8();
        }
    }
    None
}

/// A date, and only a date. `2026-11-10` and `2026/11/10` both count;
/// `2026-13-45` is not a date and `soon` never was one.
fn parse_date(token: &str) -> Option<NaiveDate> {
    let token = token.trim_matches(|c: char| matches!(c, ']' | ')' | ',' | ';'));
    ["%Y-%m-%d", "%Y/%m/%d"]
        .iter()
        .find_map(|format| NaiveDate::parse_from_str(token, format).ok())
}

/// The task body on one raw markdown line, and whether it is ticked
/// off. The list marker and the checkbox come off; `None` means the
/// line is not a task at all, so prose never becomes a reminder.
///
/// A checkbox holding anything but a space counts as done, which is the
/// rule the task index already applies to `- [x]`, `- [/]`, `- [>]`.
pub fn task_line_body(raw: &str) -> Option<(bool, &str)> {
    let line = raw.trim_start();
    let rest = strip_list_marker(line).trim_start();
    let inner = rest.strip_prefix('[')?;
    let end = inner.find(']')?;
    let done = inner[..end].chars().any(|c| c != ' ');
    Some((done, inner[end + 1..].trim()))
}

/// Takes a list marker off the front, if there is one.
fn strip_list_marker(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix(['-', '*', '+']) {
        return rest;
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 {
        if let Some(rest) = line[digits..].strip_prefix(['.', ')']) {
            return rest;
        }
    }
    line
}

/// The task text with its date markers taken off: a reminder saying
/// "pay rent" reads better than one saying "pay rent 📅 2026-01-05",
/// and the date is already in the notice's own words.
fn display_text(body: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut drop_next = false;
    for token in body.split_whitespace() {
        if drop_next {
            drop_next = false;
            if parse_date(token).is_some() {
                continue;
            }
        }
        if token.starts_with(EMOJI_MARKERS.as_slice()) || token.contains("due::") {
            drop_next = true;
            continue;
        }
        kept.push(token);
    }
    kept.join(" ")
}

/// Every task line in the vault whose due date has arrived, in date
/// order. The task lines come from the index the search pane builds —
/// the same list items `task:` matches on — so a reminder and a search
/// can never disagree about what a task is.
pub fn scan_vault(state: &VaultState, today: NaiveDate) -> Vec<DueTask> {
    let cache = state.cache();
    let texts = state.texts();
    let mut due: Vec<DueTask> = Vec::new();
    for (path, meta) in cache.files() {
        let Some(text) = texts.get(path) else {
            continue;
        };
        due.extend(due_in_note(path, text, meta, today));
    }
    due.sort_by(|a, b| (a.due, &a.path, a.line).cmp(&(b.due, &b.path, b.line)));
    due
}

/// The due task lines in one note: the task lines of `text` as the
/// index sees them, each carrying a date that has arrived.
fn due_in_note(path: &str, text: &str, meta: &CachedMetadata, today: NaiveDate) -> Vec<DueTask> {
    let (_, _, _, tasks) = search::inputs(meta, text);
    let index = LineIndex::new(text);
    tasks
        .into_iter()
        .filter_map(|task| {
            let at = task.range.start;
            let raw = text.get(task.range)?;
            let (done, body) = task_line_body(raw)?;
            if done {
                return None;
            }
            let due = parse_due(body)?;
            if due > today {
                return None;
            }
            Some(DueTask {
                path: path.to_string(),
                line: index.pos(at).line as usize,
                text: display_text(body),
                due,
            })
        })
        .collect()
}

/// The announced set as the vault holds it. A file nobody can parse
/// announces nothing yet — the same as no file at all — because a
/// reminder must never be the reason a vault is unhappy; the vault's own
/// reader already takes the byte order mark off.
fn read_store(vault: &Vault) -> ReminderStore {
    match vault.read_config::<ReminderStore>(FILE) {
        Ok(Some(store)) => store,
        Ok(None) | Err(_) => ReminderStore::default(),
    }
}

/// One due scan: the notices to show, the tasks already said before held
/// back. The store is written either way, so a restart does not repeat
/// what the screen already said.
pub fn poll(state: &VaultState) -> Vec<String> {
    let due = scan_vault(state, today());
    let vault = state.vault();
    let mut store = read_store(&vault);
    let fresh = store.record(&due);
    if let Err(err) = vault.replace_config(FILE, &store) {
        return vec![format!("{FILE} could not be written: {err}")];
    }
    drop(vault);
    let mut notices: Vec<String> = fresh.iter().take(MAX_NOTICES).map(notice).collect();
    let more = fresh.len().saturating_sub(MAX_NOTICES);
    if more > 0 {
        notices.push(format!("Reminder: {more} more task(s) are due."));
    }
    notices
}

/// What a due task says when it is noticed.
fn notice(task: &DueTask) -> String {
    let when = if task.due == today() {
        "due today".to_string()
    } else {
        format!("due {}", task.due)
    };
    format!(
        "Reminder: {} — {} ({}:{})",
        task.text,
        when,
        task.path,
        task.line + 1
    )
}

/// What `silica:reminder-add` found on the cursor's line.
pub enum Added {
    /// The task is already due: the notice that was shown, which the
    /// poll will not say again.
    Fired(String),
    /// The date is still ahead: nothing to say now, and the poll says
    /// it when the date arrives.
    Waiting(NaiveDate),
    /// This task was already announced for this date.
    Already,
}

/// Registers the reminder on one raw markdown line — the `silica:` end
/// of the feature. The line has to be a task, and the task has to carry
/// a date; anything else is a refusal that says which is missing.
pub fn register_line(
    state: &VaultState,
    path: &str,
    line: usize,
    raw: &str,
) -> Result<Added, String> {
    let Some((done, body)) = task_line_body(raw) else {
        return Err("Put the cursor on a task line: `- [ ] something`".to_string());
    };
    if done {
        return Err("That task is ticked off".to_string());
    }
    let Some(due) = parse_due(body) else {
        return Err(
            "That task has no due date. Write `📅 2026-11-10` or `[due:: 2026-11-10]` on the line."
                .to_string(),
        );
    };
    let task = DueTask {
        path: path.to_string(),
        line,
        text: display_text(body),
        due,
    };
    if due > today() {
        return Ok(Added::Waiting(due));
    }
    let vault = state.vault();
    let mut store = read_store(&vault);
    if store.is_announced(&task) {
        return Ok(Added::Already);
    }
    store.announce(&task);
    if let Err(err) = vault.replace_config(FILE, &store) {
        return Err(format!("{FILE} could not be written: {err}"));
    }
    Ok(Added::Fired(notice(&task)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_cache::index_markdown;

    /// Four tasks and a date in the past, the way a vault writes them:
    /// one overdue pending, one overdue ticked off, one still ahead, one
    /// with no date at all, and one dated through an inline field.
    const NOTE: &str = "# Bills\n\n- [ ] pay rent \u{1f4c5} 2026-01-05\n- [x] file taxes \u{1f4c5} 2026-01-06\n- [ ] call mum \u{1f4c5} 2027-03-01\n- [ ] water the plants\n- [ ] dentist [due:: 2026-02-02]\n";

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 2).expect("a real date")
    }

    fn due_of(content: &str) -> Vec<DueTask> {
        let meta = index_markdown(content);
        due_in_note("note.md", content, &meta, today())
    }

    /// A vault with `.obsidian` in it, and the store file's own text as
    /// given — a real vault, so a round trip goes through the vault's
    /// own writer and reader rather than a copy of their rules.
    fn vault_with(text: Option<&str>) -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".obsidian")).unwrap();
        if let Some(text) = text {
            std::fs::write(dir.path().join(".obsidian").join(FILE), text).unwrap();
        }
        let vault = Vault::open(dir.path()).unwrap();
        (dir, vault)
    }

    #[test]
    fn a_task_past_its_due_date_is_found() {
        let due = due_of(NOTE);
        assert_eq!(due.len(), 2, "only the due pending tasks: {due:?}");
        assert_eq!(due[0].text, "pay rent");
        assert_eq!(due[0].path, "note.md");
        assert_eq!(due[0].due, NaiveDate::from_ymd_opt(2026, 1, 5).unwrap());
        assert_eq!(due[1].text, "dentist");
        assert_eq!(due[1].due, NaiveDate::from_ymd_opt(2026, 2, 2).unwrap());
    }

    #[test]
    fn a_due_task_is_found_on_the_line_it_is_written_on() {
        let due = due_of(NOTE);
        assert_eq!(due[0].line, 2);
        assert_eq!(due[1].line, 6);
    }

    #[test]
    fn a_task_ordered_or_bulleted_is_found_the_same_way() {
        let content = "1. [ ] first \u{1f4c5} 2026-01-01\n* [x] second \u{1f4c5} 2026-01-02\n+ [ ] third \u{1f4c5} 2026-01-03\n";
        let due = due_of(content);
        let texts: Vec<&str> = due.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["first", "third"]);
    }

    #[test]
    fn a_done_task_never_reminds() {
        let content = "- [x] file taxes \u{1f4c5} 2026-01-06\n";
        assert!(due_of(content).is_empty());
    }

    #[test]
    fn a_future_due_date_waits() {
        let content = "- [ ] holiday \u{1f4c5} 2027-03-01\n";
        assert!(due_of(content).is_empty());
    }

    #[test]
    fn the_due_date_comes_from_a_marker_or_an_inline_field() {
        for line in [
            "- [ ] dentist \u{1f4c5} 2026-02-02",
            "- [ ] dentist [due:: 2026-02-02]",
            "- [ ] dentist (due:: 2026-02-02)",
            "- [ ] dentist \u{23f3} 2026-02-02",
        ] {
            let due = due_of(&format!("{line}\n"));
            assert_eq!(due.len(), 1, "{line} carries a due date");
            assert_eq!(due[0].due, NaiveDate::from_ymd_opt(2026, 2, 2).unwrap());
        }
    }

    #[test]
    fn a_date_that_is_not_a_date_is_ignored() {
        for line in [
            "- [ ] dentist \u{1f4c5} soon",
            "- [ ] dentist \u{1f4c5} 2026-13-45",
            "- [ ] dentist [due:: someday]",
            "- [ ] dentist [status:: done]",
        ] {
            assert!(
                due_of(&format!("{line}\n")).is_empty(),
                "{line} has no date"
            );
        }
    }

    #[test]
    fn the_task_text_shows_without_its_markers() {
        assert_eq!(display_text("pay rent \u{1f4c5} 2026-01-05"), "pay rent");
        assert_eq!(display_text("dentist [due:: 2026-02-02]"), "dentist");
        assert_eq!(display_text("dentist [due::2026-02-02]"), "dentist");
        assert_eq!(
            display_text("dentist \u{1f4c5} 2026-01-05 [due:: 2026-01-05]"),
            "dentist"
        );
    }

    #[test]
    fn a_task_line_is_recognised_through_its_markers() {
        assert_eq!(
            task_line_body("- [ ] something"),
            Some((false, "something"))
        );
        assert_eq!(task_line_body("  * [x] done"), Some((true, "done")));
        assert_eq!(task_line_body("1. [ ] ordered"), Some((false, "ordered")));
        assert_eq!(task_line_body("2) [/] started"), Some((true, "started")));
        assert_eq!(task_line_body("just prose"), None);
        assert_eq!(task_line_body("- a plain list item"), None);
    }

    #[test]
    fn a_due_task_is_announced_once() {
        let due = due_of(NOTE);
        let mut store = ReminderStore::default();
        let first = store.record(&due);
        assert_eq!(first.len(), 2, "both due tasks are said the first time");
        let again = store.record(&due);
        assert!(again.is_empty(), "the second scan says nothing: {again:?}");
    }

    #[test]
    fn a_task_that_leaves_the_due_list_can_be_announced_again() {
        let due = due_of(NOTE);
        let mut store = ReminderStore::default();
        store.record(&due);
        // * The task is ticked off: it is no longer due, so its key goes.
        store.record(&[]);
        assert!(store.announced.is_empty());
        let back = store.record(&due);
        assert_eq!(back.len(), 2, "overdue again is said again");
    }

    #[test]
    fn one_task_can_be_announced_without_touching_the_others() {
        let due = due_of(NOTE);
        let mut store = ReminderStore::default();
        store.record(&due);
        store.announce(&due[0]);
        assert_eq!(store.announced.len(), 2);
    }

    /// The store round-trips through the vault's own writer and reader:
    /// what one scan wrote comes back as the announced set the next scan
    /// reads, so a restart does not say the same thing twice.
    #[test]
    fn a_store_round_trips_through_the_vaults_own_files() {
        let (_dir, vault) = vault_with(None);
        let due = due_of(NOTE);
        let mut store = ReminderStore::default();
        store.record(&due);
        vault.replace_config(FILE, &store).expect("written");

        let back = read_store(&vault);
        assert_eq!(back.announced.len(), due.len());
        assert!(back.is_announced(&due[0]));
        assert!(back.is_announced(&due[1]));
    }

    /// A file nobody can parse announces nothing yet — the same as no
    /// file at all — because a reminder must never be the reason a vault
    /// is unhappy.
    #[test]
    fn a_file_nobody_can_parse_announces_nothing_yet() {
        let (_dir, vault) = vault_with(Some("not json at all"));
        assert!(read_store(&vault).announced.is_empty());
    }

    #[test]
    fn a_notice_names_the_task_its_date_and_its_line() {
        let task = DueTask {
            path: "bills.md".to_string(),
            line: 4,
            text: "pay rent".to_string(),
            due: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
        };
        assert_eq!(
            notice(&task),
            "Reminder: pay rent — due 2026-01-05 (bills.md:5)"
        );
    }
}
