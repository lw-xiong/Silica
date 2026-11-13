//! Flashcards scanned from vault markdown, with SM-2 scheduling.
//!
//! A card is a `#flashcard` tag line plus the block under it, in any of
//! three shapes:
//!
//! ```text
//! #flashcard What is 2+2?::4
//! #flashcard
//! Multiline question
//! ?
//! Multiline answer
//! #flashcard The capital of France is ==Paris==
//! ```
//!
//! The first two split on `::` (or a `?` separator line) into front and
//! back; the last hides every `==answer==` span as a cloze. Schedules live
//! in `.obsidian/.silica-flashcards.json` (see [`STORE_NAME`]) and round
//! trip through [`load`]/[`save`]. No gpui: the app renders, this crate
//! only parses and schedules.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

/// The config file the app keeps in the vault's `.obsidian` folder.
pub const STORE_NAME: &str = ".silica-flashcards.json";

/// The tag a card starts on. `#flashcards` is a different tag and is not
/// matched.
pub const TAG: &str = "#flashcard";

/// One card: where it came from and what to ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// Stable id: FNV-1a of `source:line:block`, so re-scanning the same
    /// note yields the same id and the schedule survives edits elsewhere.
    pub id: String,
    /// Vault-relative note path.
    pub source: String,
    /// 1-indexed line the tag sits on.
    pub line: usize,
    pub kind: CardKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardKind {
    Basic { front: String, back: String },
    /// `text` holds the question with each answer replaced by `____`;
    /// `answers` holds the `==...==` contents in order.
    Cloze { text: String, answers: Vec<String> },
}

impl Card {
    /// The question side: the front, or the cloze with answers hidden.
    pub fn front(&self) -> String {
        match &self.kind {
            CardKind::Basic { front, .. } => front.clone(),
            CardKind::Cloze { text, .. } => text.clone(),
        }
    }

    /// The answer side: the back, or the cloze answers joined.
    pub fn back(&self) -> String {
        match &self.kind {
            CardKind::Basic { back, .. } => back.clone(),
            CardKind::Cloze { answers, .. } => answers.join(", "),
        }
    }
}

/// Parses every card in `text`, tagging each with `source`.
pub fn parse_cards(source: &str, text: &str) -> Vec<Card> {
    let mut cards = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((ix, line)) = lines.next() {
        let Some(tag_at) = find_tag(line) else {
            continue;
        };
        let line_no = ix + 1;
        let mut block = line[tag_at + TAG.len()..].trim().to_string();
        // * The block runs to the first blank line, heading, or next tag.
        while let Some(&(next_ix, next)) = lines.peek() {
            if next.trim().is_empty() || next.starts_with('#') || find_tag(next).is_some() {
                break;
            }
            if !block.is_empty() {
                block.push('\n');
            }
            block.push_str(next.trim_end());
            lines.next();
            let _ = next_ix;
        }
        if let Some(kind) = block_kind(&block) {
            let id = format!("{:016x}", fnv1a64(format!("{source}:{line_no}:{block}").as_bytes()));
            cards.push(Card {
                id,
                source: source.to_string(),
                line: line_no,
                kind,
            });
        }
    }
    cards
}

/// Where `TAG` occurs in `line` as a whole tag, not as a prefix of a
/// longer one (`#flashcards`).
fn find_tag(line: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = line[from..].find(TAG) {
        let at = from + rel;
        let after = line[at + TAG.len()..].chars().next();
        if !matches!(after, Some(c) if c.is_alphanumeric() || c == '-' || c == '_') {
            return Some(at);
        }
        from = at + TAG.len();
    }
    None
}

/// Turns a card block into a kind: `::` or a `?` line splits front/back,
/// otherwise `==...==` spans make a cloze, otherwise there is no card.
fn block_kind(block: &str) -> Option<CardKind> {
    if let Some(split) = block.find("::") {
        let front = block[..split].trim().to_string();
        let back = block[split + 2..].trim().to_string();
        if front.is_empty() && back.is_empty() {
            return None;
        }
        return Some(CardKind::Basic { front, back });
    }
    if let Some(qpos) = block.lines().position(|l| l.trim_start().starts_with('?')) {
        // * A `?` on its own line separates a multiline question from its
        // * answer, the way the spaced-repetition plugin writes them.
        let front = block.lines().take(qpos).collect::<Vec<_>>().join("\n");
        let back = block
            .lines()
            .skip(qpos + 1)
            .collect::<Vec<_>>()
            .join("\n");
        let front = front.trim().to_string();
        let back = back.trim().to_string();
        if !front.is_empty() || !back.is_empty() {
            return Some(CardKind::Basic { front, back });
        }
    }
    if block.contains("==") {
        let mut text = String::with_capacity(block.len());
        let mut answers = Vec::new();
        let mut rest = block;
        while let Some(open) = rest.find("==") {
            text.push_str(&rest[..open]);
            let after_open = &rest[open + 2..];
            match after_open.find("==") {
                Some(close) => {
                    let answer = after_open[..close].trim().to_string();
                    if !answer.is_empty() {
                        answers.push(answer);
                        text.push_str("____");
                    } else {
                        text.push_str("==");
                    }
                    rest = &after_open[close + 2..];
                }
                None => {
                    text.push_str(&rest[open..]);
                    rest = "";
                    break;
                }
            }
        }
        text.push_str(rest);
        if answers.is_empty() {
            return None;
        }
        return Some(CardKind::Cloze {
            text: text.trim().to_string(),
            answers,
        });
    }
    None
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// How a card was last answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    Again,
    Hard,
    Good,
    Easy,
}

impl Grade {
    /// SM-2 quality: below 3 resets the card.
    fn quality(self) -> u8 {
        match self {
            Grade::Again => 0,
            Grade::Hard => 3,
            Grade::Good => 4,
            Grade::Easy => 5,
        }
    }
}

/// One card's SM-2 state, as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default = "default_ease")]
    pub ease: f64,
    #[serde(default)]
    pub interval_days: f64,
    #[serde(default)]
    pub repetitions: u32,
    #[serde(default)]
    pub lapses: u32,
    /// `YYYY-MM-DD` of the last review.
    #[serde(default)]
    pub last_review: Option<String>,
    /// `YYYY-MM-DD` the card is due again; `None` before the first review.
    #[serde(default)]
    pub due: Option<String>,
}

fn default_ease() -> f64 {
    2.5
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            ease: default_ease(),
            interval_days: 0.0,
            repetitions: 0,
            lapses: 0,
            last_review: None,
            due: None,
        }
    }
}

/// The whole store: card id to schedule, persisted whole.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub cards: BTreeMap<String, Schedule>,
}

/// Reads the store at `path`; a missing file is an empty store.
pub fn load(path: &Path) -> std::io::Result<Store> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(e),
    }
}

/// Writes the store atomically: temp file in the same folder, then rename.
pub fn save(path: &Path, store: &Store) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(store)?)?;
    std::fs::rename(&tmp, path)
}

/// Records one review of `id` on `today`, SM-2 style.
pub fn review(store: &mut Store, id: &str, grade: Grade, today: NaiveDate) {
    let entry = store.cards.entry(id.to_string()).or_default();
    let q = grade.quality();
    if q < 3 {
        entry.repetitions = 0;
        entry.interval_days = 1.0;
        entry.lapses += 1;
    } else {
        entry.interval_days = match entry.repetitions {
            0 => 1.0,
            1 => 6.0,
            _ => (entry.interval_days * entry.ease).round().max(1.0),
        };
        entry.repetitions += 1;
        let q = f64::from(q);
        entry.ease = (entry.ease + (0.1 - (5.0 - q) * (0.08 + (5.0 - q) * 0.02))).max(1.3);
    }
    entry.last_review = Some(today.to_string());
    entry.due = Some((today + Duration::days(entry.interval_days.ceil() as i64)).to_string());
}

/// Whether `card` is due on `today`: a card with no schedule is new, and
/// new cards are due.
pub fn is_due(card: &Card, store: &Store, today: NaiveDate) -> bool {
    match store.cards.get(&card.id) {
        None => true,
        Some(s) => match s.due.as_deref().and_then(parse_date) {
            None => true,
            Some(due) => due <= today,
        },
    }
}

/// The due subset of `cards`, oldest due first, then by id.
pub fn due_cards<'a>(cards: &'a [Card], store: &Store, today: NaiveDate) -> Vec<&'a Card> {
    let mut due: Vec<&Card> = cards.iter().filter(|c| is_due(c, store, today)).collect();
    due.sort_by(|a, b| {
        let da = store.cards.get(&a.id).and_then(|s| s.due.as_deref()).map(str::to_string);
        let db = store.cards.get(&b.id).and_then(|s| s.due.as_deref()).map(str::to_string);
        da.cmp(&db).then_with(|| a.id.cmp(&b.id))
    });
    due
}

fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()
    }

    #[test]
    fn parses_a_single_line_card() {
        let cards = parse_cards("A.md", "#flashcard What is 2+2?::4\n");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].source, "A.md");
        assert_eq!(cards[0].line, 1);
        assert_eq!(
            cards[0].kind,
            CardKind::Basic {
                front: "What is 2+2?".to_string(),
                back: "4".to_string()
            }
        );
        assert_eq!(cards[0].front(), "What is 2+2?");
        assert_eq!(cards[0].back(), "4");
    }

    #[test]
    fn parses_a_multiline_card_with_a_question_mark_separator() {
        let text = "#flashcard\nLine one\nLine two\n?\nThe answer\n\nnext para\n";
        let cards = parse_cards("A.md", text);
        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].kind,
            CardKind::Basic {
                front: "Line one\nLine two".to_string(),
                back: "The answer".to_string()
            }
        );
    }

    #[test]
    fn parses_a_cloze_card_and_hides_the_answers() {
        let cards = parse_cards("A.md", "#flashcard The capital of France is ==Paris== and of Japan is ==Tokyo==.\n");
        assert_eq!(cards.len(), 1);
        let CardKind::Cloze { text, answers } = &cards[0].kind else {
            panic!("expected a cloze: {:?}", cards[0].kind);
        };
        assert_eq!(text, "The capital of France is ____ and of Japan is ____.");
        assert_eq!(answers, &vec!["Paris".to_string(), "Tokyo".to_string()]);
        assert_eq!(cards[0].back(), "Paris, Tokyo");
    }

    #[test]
    fn ignores_plain_text_and_longer_tags() {
        let text = "no tag here\n#flashcards is a different tag\n#flashcard\n\n";
        assert!(parse_cards("A.md", text).is_empty());
    }

    #[test]
    fn ids_are_stable_across_rescans() {
        let text = "#flashcard Q::A\n";
        let first = parse_cards("A.md", text);
        let second = parse_cards("A.md", text);
        assert_eq!(first[0].id, second[0].id);
        assert_ne!(parse_cards("B.md", text)[0].id, first[0].id);
    }

    #[test]
    fn sm2_walks_one_six_then_ease_and_resets_on_again() {
        let mut store = Store::default();
        let card = parse_cards("A.md", "#flashcard Q::A\n").remove(0);
        let day = today();

        assert!(is_due(&card, &store, day), "a new card is due");
        review(&mut store, &card.id, Grade::Good, day);
        let s = store.cards.get(&card.id).unwrap();
        assert_eq!(s.repetitions, 1);
        assert_eq!(s.interval_days, 1.0);
        assert_eq!(s.due.as_deref(), Some("2026-11-14"));
        assert!(!is_due(&card, &store, day), "just reviewed: not due today");
        assert!(is_due(&card, &store, day + Duration::days(1)));

        review(&mut store, &card.id, Grade::Good, day + Duration::days(1));
        let s = store.cards.get(&card.id).unwrap();
        assert_eq!(s.repetitions, 2);
        assert_eq!(s.interval_days, 6.0);

        review(&mut store, &card.id, Grade::Easy, day + Duration::days(7));
        let s = store.cards.get(&card.id).unwrap();
        assert_eq!(s.repetitions, 3);
        // * The interval grows by the ease *before* this review (2.5), and
        //   Easy then lifts the ease to 2.6.
        assert_eq!(s.interval_days, 15.0);
        assert!((s.ease - 2.6).abs() < 1e-9);

        review(&mut store, &card.id, Grade::Again, day + Duration::days(30));
        let s = store.cards.get(&card.id).unwrap();
        assert_eq!(s.repetitions, 0);
        assert_eq!(s.interval_days, 1.0);
        assert_eq!(s.lapses, 1);
        assert_eq!(s.due.as_deref(), Some("2026-12-14"));
    }

    #[test]
    fn due_cards_sorts_oldest_due_first() {
        let mut store = Store::default();
        let cards = parse_cards("A.md", "#flashcard one::1\n#flashcard two::2\n");
        let day = today();
        review(&mut store, &cards[1].id, Grade::Good, day); // due tomorrow
        let due = due_cards(&cards, &store, day);
        assert_eq!(due.len(), 1, "only the unseen card is due");
        assert_eq!(due[0].id, cards[0].id);
    }

    #[test]
    fn store_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("silica-flashcards-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(".obsidian").join(STORE_NAME);
        let mut store = Store::default();
        review(&mut store, "abc123", Grade::Hard, today());
        save(&path, &store).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(store, loaded);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_of_a_missing_file_is_an_empty_store() {
        let path = std::env::temp_dir().join(format!("silica-flashcards-missing-{}", std::process::id()));
        let store = load(&path).expect("missing file is empty");
        assert_eq!(store, Store::default());
    }
}
