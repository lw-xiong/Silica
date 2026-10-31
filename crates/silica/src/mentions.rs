//! Unlinked mentions: plain-text occurrences of note names and aliases that
//! are not already inside a wikilink, as the Backlinks and Outgoing links
//! panes list them.

use regex::RegexBuilder;
use std::collections::HashMap;
use std::ops::Range;

/// A name to look for and what it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name<'a> {
    pub text: &'a str,
    /// Vault path of the note the name belongs to.
    pub path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    /// The note the mentioned name belongs to.
    pub path: String,
    /// Byte offset of the line the mention is on.
    pub line_offset: usize,
    /// The mention inside the text.
    pub range: Range<usize>,
}

/// A matcher for a set of names, built once and run over many texts.
pub struct Finder {
    regex: regex::Regex,
    by_lower: HashMap<String, String>,
}

impl Finder {
    /// Names shorter than two characters are ignored; later duplicates of a
    /// name (case-insensitively) are dropped.
    pub fn new(names: &[Name<'_>]) -> Option<Finder> {
        let mut by_lower: HashMap<String, String> = HashMap::new();
        let mut parts: Vec<String> = Vec::new();
        for n in names {
            let text = n.text.trim();
            if text.chars().count() < 2 {
                continue;
            }
            let lower = text.to_lowercase();
            if by_lower.contains_key(&lower) {
                continue;
            }
            by_lower.insert(lower, n.path.to_string());
            parts.push(regex::escape(text));
        }
        if parts.is_empty() {
            return None;
        }
        // Longest first so "Plan B" wins over "Plan" at the same position.
        parts.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let pattern = format!(
            r"(?:^|[^\p{{L}}\p{{N}}_])({})(?:[^\p{{L}}\p{{N}}_]|$)",
            parts.join("|")
        );
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .size_limit(64 << 20)
            .build()
            .ok()?;
        Some(Finder { regex, by_lower })
    }

    /// Every mention in `text` outside `[[...]]` and outside code fences.
    pub fn find(&self, text: &str) -> Vec<Mention> {
        let mut out = Vec::new();
        let mut line_start = 0;
        let mut in_fence = false;
        for line in text.split_inclusive('\n') {
            let body = line.trim_end_matches(['\n', '\r']);
            if body.trim_start().starts_with("```") || body.trim_start().starts_with("~~~") {
                in_fence = !in_fence;
            } else if !in_fence {
                let mut at = 0;
                while let Some(caps) = self.regex.captures_at(body, at) {
                    let m = caps.get(1).unwrap();
                    at = m.end().max(at + 1);
                    if inside_link(body, m.start()) {
                        continue;
                    }
                    let lower = m.as_str().to_lowercase();
                    if let Some(path) = self.by_lower.get(&lower) {
                        out.push(Mention {
                            path: path.clone(),
                            line_offset: line_start,
                            range: line_start + m.start()..line_start + m.end(),
                        });
                    }
                }
            }
            line_start += line.len();
        }
        out
    }
}

/// Whether byte `at` of `line` sits inside `[[...]]`, `[...](...)` text or
/// an inline code span.
fn inside_link(line: &str, at: usize) -> bool {
    let before = &line[..at];
    let after = &line[at..];
    if let Some(open) = before.rfind("[[") {
        if !before[open..].contains("]]") && after.contains("]]") {
            return true;
        }
    }
    if before.matches('`').count() % 2 == 1 && after.contains('`') {
        return true;
    }
    if let Some(open) = before.rfind('[') {
        if !before[open..].contains(']') && after.contains("](") {
            return true;
        }
    }
    // The destination part of `[text](dest)`.
    if let Some(open) = before.rfind("](") {
        if !before[open..].contains(')') && after.contains(')') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_whole_word_names_outside_links_and_code() {
        let names = [
            Name {
                text: "Plan",
                path: "Projects/Plan.md",
            },
            Name {
                text: "Plan B",
                path: "Plan B.md",
            },
            Name {
                text: "Home",
                path: "Home.md",
            },
            Name {
                text: "x",
                path: "x.md",
            },
        ];
        let f = Finder::new(&names).unwrap();
        let text = "The plan is [[Plan]] and [Plan](Plan.md) and `Plan` and plan b.\nPlanning home\n```\nPlan\n```\nHOME.";
        let found = f.find(text);
        let shown: Vec<(&str, &str)> = found
            .iter()
            .map(|m| (m.path.as_str(), &text[m.range.clone()]))
            .collect();
        assert_eq!(
            shown,
            vec![
                ("Projects/Plan.md", "plan"),
                ("Plan B.md", "plan b"),
                ("Home.md", "home"),
                ("Home.md", "HOME"),
            ]
        );
        assert_eq!(found[2].line_offset, text.find("Planning").unwrap());
    }

    #[test]
    fn empty_name_sets_build_nothing() {
        assert!(Finder::new(&[]).is_none());
        assert!(Finder::new(&[Name {
            text: "a",
            path: "a.md"
        }])
        .is_none());
    }
}
