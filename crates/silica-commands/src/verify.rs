//! `silica:palette-verify`: the audit behind the command of the same name.
//!
//! The palette is only as good as the three lists behind it: the ids the
//! registry knows (`commands.txt`), the names it shows (`names.txt`) and
//! the ids a `run_command` arm answers for (`RUNNABLE` and `EDITOR_ONLY`
//! in the workspace). Any one of them drifting — an id listed twice, a
//! name line with nothing after the tab, an id nothing can run — shows up
//! in the app as a palette row that does nothing or vanishes when asked
//! for, which is exactly what this audit is here to name before a user
//! finds it. The command writes what it finds to `Palette check.md` in the
//! vault, so the answer lives with the vault it describes.

use std::collections::BTreeSet;

/// Where the command writes its report: the vault's root, where the rest
/// of the vault's own documentation sits.
pub const REPORT_PATH: &str = "Palette check.md";

/// What is wrong with one registered command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// The id is registered but nothing runs it: no `run_command` arm
    /// answers for it and no native plugin claims it. Asking for it by
    /// id does nothing; whether the palette shows the row at all is
    /// `can_run`'s separate question, so such an id may be hidden rather
    /// than offered.
    NotRunnable,
    /// The id is registered twice: the second registration silently
    /// replaces the first, so whichever of the two wins is the one that
    /// runs and the other is unreachable.
    DuplicateId,
    /// The name line for a registered id is empty, so the palette has a
    /// row with no text on it.
    EmptyName,
}

impl IssueKind {
    /// The label the report uses for this kind of finding.
    pub fn label(self) -> &'static str {
        match self {
            IssueKind::NotRunnable => "Not runnable",
            IssueKind::DuplicateId => "Duplicate id",
            IssueKind::EmptyName => "Empty name",
        }
    }
}

/// One finding, with the id it is about and why it is a finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteIssue {
    pub kind: IssueKind,
    pub id: String,
    pub detail: String,
}

/// The registry as audited: the ids it registered, how many of them the
/// app can run, and what the audit found. [`PaletteAudit::text`] is the
/// report the command writes.
#[derive(Debug, Clone)]
pub struct PaletteAudit {
    /// Distinct registered ids.
    pub registered: usize,
    /// Registered ids the app can run (or that need an editor, which
    /// `can_run` answers for the same way).
    pub runnable: usize,
    pub issues: Vec<PaletteIssue>,
    text: String,
}

impl PaletteAudit {
    /// The report, ready to write to [`REPORT_PATH`].
    pub fn text(&self) -> &str {
        &self.text
    }

    /// True when nothing was found: a clean registry.
    pub fn ok(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Ids registered more than once, in first-seen order.
///
/// The duplicate is reported once per id, not once per line, because the
/// report is read by someone about to remove one of the two lines.
pub fn duplicate_ids<'a>(registered: &[&'a str]) -> Vec<&'a str> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut reported: BTreeSet<&str> = BTreeSet::new();
    let mut duplicates = Vec::new();
    for id in registered {
        if !seen.insert(id) && reported.insert(id) {
            duplicates.push(*id);
        }
    }
    duplicates
}

/// Registered ids whose name line has nothing after the tab.
///
/// A name of only whitespace is the same finding: the palette trims, so
/// the row shows nothing either way.
pub fn empty_names<'a>(named: &[(&'a str, &'a str)]) -> Vec<&'a str> {
    named
        .iter()
        .filter(|(_, name)| name.trim().is_empty())
        .map(|(id, _)| *id)
        .collect()
}

/// Registered ids nothing runs — no `run_command` arm and no native
/// plugin.
///
/// `runnable` is the app's full dispatch surface: `RUNNABLE` and
/// `EDITOR_ONLY` together, plus any id a native plugin answers for (the
/// workspace collects those at audit time; `can_run` consults only the
/// narrower `RUNNABLE` set, so a runnable id missing from it is hidden
/// from the palette rather than dead).
pub fn unrunnable_ids<'a>(registered: &[&'a str], runnable: &[&str]) -> Vec<&'a str> {
    registered
        .iter()
        .filter(|id| !runnable.contains(id))
        .copied()
        .collect()
}

/// Audits one registry snapshot: `registered` the ids it knows,
/// `named` the `(id, name)` pairs the palette shows, and `runnable` the
/// ids the app can run.
///
/// The findings come back duplicates first, then empty names, then the
/// registered ids nothing can run — the order the report groups them in,
/// and the order someone works through them: what is broken in the data,
/// then what is broken in the list, then what is simply not wired yet.
pub fn audit<'a>(
    registered: &[&'a str],
    named: &[(&'a str, &'a str)],
    runnable: &[&str],
) -> PaletteAudit {
    let distinct: BTreeSet<&str> = registered.iter().copied().collect();
    let runnable_set: BTreeSet<&str> = runnable.iter().copied().collect();
    let mut issues = Vec::new();
    for id in duplicate_ids(registered) {
        issues.push(PaletteIssue {
            kind: IssueKind::DuplicateId,
            id: id.to_string(),
            detail: "registered twice; the second entry replaces the first".to_string(),
        });
    }
    for id in empty_names(named) {
        issues.push(PaletteIssue {
            kind: IssueKind::EmptyName,
            id: id.to_string(),
            detail: "name line is empty".to_string(),
        });
    }
    for id in unrunnable_ids(registered, runnable) {
        if issues.iter().any(|issue| issue.id == id) {
            continue;
        }
        issues.push(PaletteIssue {
            kind: IssueKind::NotRunnable,
            id: id.to_string(),
            detail: "registered but no run_command arm answers for it".to_string(),
        });
    }
    PaletteAudit {
        registered: distinct.len(),
        runnable: distinct.intersection(&runnable_set).count(),
        text: report(registered.len(), distinct.len(), &runnable_set, &issues),
        issues,
    }
}

/// The report the command writes: counts, then every finding grouped by
/// kind, with the id and why it is one.
fn report(
    listed: usize,
    registered: usize,
    runnable: &BTreeSet<&str>,
    issues: &[PaletteIssue],
) -> String {
    let mut text = String::from("# Palette check\n\n");
    text.push_str(&format!(
        "- Registered: {registered}\n- Runnable: {}\n- Duplicate ids: {}\n- Empty names: {}\n- Not runnable: {}\n- Findings: {}\n",
        runnable.len(),
        issues
            .iter()
            .filter(|issue| issue.kind == IssueKind::DuplicateId)
            .count(),
        issues
            .iter()
            .filter(|issue| issue.kind == IssueKind::EmptyName)
            .count(),
        issues
            .iter()
            .filter(|issue| issue.kind == IssueKind::NotRunnable)
            .count(),
        issues.len(),
    ));
    if issues.is_empty() {
        text.push_str(
            "\nNo findings: every registered id is runnable, named and registered once.\n",
        );
        return text;
    }
    for kind in [
        IssueKind::DuplicateId,
        IssueKind::EmptyName,
        IssueKind::NotRunnable,
    ] {
        let group: Vec<&PaletteIssue> = issues.iter().filter(|issue| issue.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        text.push_str(&format!("\n## {} ({})\n\n", kind.label(), group.len()));
        for issue in group {
            text.push_str(&format!("- `{}`: {}\n", issue.id, issue.detail));
        }
    }
    // * The number of lines the id lists hold, against the distinct ids
    //   they amount to: a duplicate is the one gap between the two, and
    //   the report says so rather than leaving it to be worked out.
    if listed > registered {
        text.push_str(&format!(
            "\nThe id list holds {listed} lines, {registered} distinct ids.\n"
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A registry that lies is the thing this exists for: an id registered
    /// twice, a command with no name, and a registered id nothing can run.
    /// The fake registry below is exactly that, and every one of the three
    /// must come back as a finding.
    #[test]
    fn a_registry_that_lies_is_caught() {
        let registered = [
            "editor:save-file",
            // * Registered a second time: the second entry is the bug.
            "editor:save-file",
            "app:go-back",
            "graph:open",
        ];
        let named = [
            ("editor:save-file", "Save current file"),
            // * The duplicate line, with no name on it.
            ("editor:save-file", ""),
            ("app:go-back", "Navigate back"),
        ];
        let runnable = ["editor:save-file", "app:go-back"];
        let audit = audit(&registered, &named, &runnable);

        let duplicates: Vec<&str> = audit
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::DuplicateId)
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(duplicates, vec!["editor:save-file"]);
        let empty: Vec<&str> = audit
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::EmptyName)
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(empty, vec!["editor:save-file"]);
        let unrunnable: Vec<&str> = audit
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::NotRunnable)
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(unrunnable, vec!["graph:open"]);
        assert_eq!(audit.registered, 3, "three distinct ids");
        assert_eq!(audit.runnable, 2);
        assert_eq!(audit.issues.len(), 3);
    }

    /// The shipped inventory is the thing the command reports on, so it
    /// must be clean: an id twice in `commands.txt` or a name line with
    /// nothing after the tab is a bug in the data, not a finding to file.
    #[test]
    fn the_shipped_inventory_is_clean() {
        let registered = crate::known_ids();
        let named = crate::known_names();
        let audit = audit(&registered, &named, &[]);
        assert_eq!(audit.registered, registered.len());
        for issue in &audit.issues {
            assert_ne!(
                issue.kind,
                IssueKind::DuplicateId,
                "{} is registered twice",
                issue.id
            );
            assert_ne!(
                issue.kind,
                IssueKind::EmptyName,
                "{} has an empty name",
                issue.id
            );
        }
    }

    /// The report is the deliverable: it names every finding, counts what
    /// it found, and says what was checked so the file answers on its own.
    #[test]
    fn the_report_names_every_finding() {
        let registered = ["editor:save-file", "editor:save-file", "graph:open"];
        let named = [
            ("editor:save-file", "Save current file"),
            ("editor:save-file", ""),
        ];
        let runnable = ["editor:save-file"];
        let audit = audit(&registered, &named, &runnable);
        let text = audit.text();
        assert!(text.starts_with("# Palette check"), "{text}");
        assert!(text.contains("Registered: 2"), "{text}");
        assert!(text.contains("Runnable: 1"), "{text}");
        assert!(text.contains("Findings: 3"), "{text}");
        assert!(text.contains("Duplicate id"), "{text}");
        assert!(text.contains("Empty name"), "{text}");
        assert!(text.contains("Not runnable"), "{text}");
        assert!(text.contains("graph:open"), "{text}");
        assert!(text.contains("editor:save-file"), "{text}");
        assert!(!audit.ok(), "{}", text);
    }

    /// A clean registry gets a report that says so, rather than a file
    /// with empty sections under it.
    #[test]
    fn a_clean_registry_reports_no_findings() {
        let audit = audit(
            &["editor:save-file"],
            &[("editor:save-file", "Save current file")],
            &["editor:save-file"],
        );
        assert!(audit.issues.is_empty());
        assert!(audit.text().contains("Findings: 0"), "{}", audit.text());
        assert!(audit.ok(), "{}", audit.text());
    }
}
