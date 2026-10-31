//! Auto-pairing: the closer a typed bracket or Markdown marker brings with
//! it, the closer a typed one steps over instead of doubling, the pair a
//! Backspace takes away whole, and the pair a selection gets wrapped in.
//!
//! Every rule here is one the probes of 2026-10-18 and 2026-10-25 observed
//! against Obsidian ("Observed auto-pair behavior" in the specification).
//! One rule covers every position either pass ran: a bracket closes when
//! the character ahead is whitespace, the end of the line or a closing
//! bracket; a quote or Markdown marker needs that and whitespace or the
//! start of the line behind. No context suppresses pairing. Stepping over
//! a closer is tracked by provenance — only a closer the editor itself
//! planted is skipped — which is the buffer's bookkeeping, not this
//! module's.

use crate::buffer::prev_grapheme_boundary;
use std::ops::Range;

/// Obsidian's two `app.json` auto-pair toggles. Both read back `true` on a
/// vault that never touched them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// `autoPairBrackets`: `(`, `[`, `{`, `"` and `'`.
    pub brackets: bool,
    /// `autoPairMarkdown`: the backtick, `*` and `_`, plus the four
    /// characters that only ever wrap.
    pub markdown: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            brackets: true,
            markdown: true,
        }
    }
}

/// What a typed pair character does at one selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Nothing special: write the character the way any other key writes
    /// one, replacing whatever is selected.
    Insert,
    /// Write the character and this closer, leaving the cursor between
    /// them.
    Close(char),
    /// Write nothing and step over the closer already ahead of the cursor.
    Skip,
    /// Put these either side of the selection, which survives over the
    /// text between them.
    Wrap(char, char),
}

/// Which toggle owns a pair character, and where it closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `(`, `[`, `{`: these close unless a word character follows.
    Bracket,
    /// `"` and `'`: the bracket toggle owns them, but they were seen to
    /// close only where the Markdown markers do.
    Quote,
    /// The backtick, `*` and `_`.
    Marker,
    /// `~`, `=`, `$` and `%`, which close in no position under either
    /// toggle and only ever wrap a selection.
    WrapOnly,
}

impl Kind {
    fn enabled(self, opts: Options) -> bool {
        match self {
            Kind::Bracket | Kind::Quote => opts.brackets,
            // The probe ran the selection rows with both toggles on, so
            // which one owns `~ = $ %` is a judgement call; their wraps
            // are all Markdown syntax, so the Markdown toggle gets them.
            Kind::Marker | Kind::WrapOnly => opts.markdown,
        }
    }
}

/// The closer `open` brings with it. `<` is absent on purpose: it was the
/// one character observed to replace a selection rather than wrap it.
fn pair(open: char) -> Option<(char, Kind)> {
    Some(match open {
        '(' => (')', Kind::Bracket),
        '[' => (']', Kind::Bracket),
        '{' => ('}', Kind::Bracket),
        '"' => ('"', Kind::Quote),
        '\'' => ('\'', Kind::Quote),
        '`' => ('`', Kind::Marker),
        '*' => ('*', Kind::Marker),
        '_' => ('_', Kind::Marker),
        '~' => ('~', Kind::WrapOnly),
        '=' => ('=', Kind::WrapOnly),
        '$' => ('$', Kind::WrapOnly),
        '%' => ('%', Kind::WrapOnly),
        _ => return None,
    })
}

/// The kind of pair a closing character ends, for the skip-over.
fn closer_kind(close: char) -> Option<Kind> {
    Some(match close {
        ')' | ']' | '}' => Kind::Bracket,
        '"' | '\'' => Kind::Quote,
        '`' | '*' | '_' => Kind::Marker,
        _ => return None,
    })
}

/// What typing `typed` over `sel` does. `sel` is a byte range into `text`,
/// empty for a bare cursor. `planted` says whether the character ahead of
/// the cursor is a closer the editor itself put there and has not been
/// left behind since — the one thing typing that character steps over.
pub fn act(text: &str, sel: Range<usize>, typed: char, opts: Options, planted: bool) -> Act {
    if !sel.is_empty() {
        return match pair(typed) {
            Some((close, kind)) if kind.enabled(opts) => Act::Wrap(typed, close),
            _ => Act::Insert,
        };
    }
    let at = sel.start;
    let next = text[at..].chars().next();
    // A closer typed over the one the editor planted steps past it; a
    // planted one with no opener behind it is written again (`x|)` typed
    // `)` gives `x))`), and so is one the editor planted on another line.
    if planted && next == Some(typed) {
        if let Some(kind) = closer_kind(typed) {
            if kind.enabled(opts) {
                return Act::Skip;
            }
        }
    }
    let Some((close, kind)) = pair(typed) else {
        return Act::Insert;
    };
    if !kind.enabled(opts) {
        return Act::Insert;
    }
    if closes(text, at, kind) {
        Act::Close(close)
    } else {
        Act::Insert
    }
}

/// Whether a pair of `kind` closes at `at`: the one rule the 2026-10-25
/// pass settled. A bracket closes when the character ahead is whitespace,
/// the end of the line or a closing bracket — punctuation ahead blocks it,
/// and what is behind does not matter. A quote or Markdown marker needs
/// that and whitespace or the start of the line behind.
fn closes(text: &str, at: usize, kind: Kind) -> bool {
    let next = text[at..].chars().next();
    let ahead_clear = next.is_none_or(|c| c.is_whitespace() || matches!(c, ')' | ']' | '}'));
    match kind {
        Kind::Bracket => ahead_clear,
        Kind::Quote | Kind::Marker => {
            let prev = text[..at].chars().next_back();
            ahead_clear && prev.is_none_or(char::is_whitespace)
        }
        Kind::WrapOnly => false,
    }
}

/// What a Backspace at `at` deletes: an empty pair either side of the
/// cursor goes whole, anything else loses one grapheme cluster. The pair
/// need not be one the editor inserted — a pasted `(|)` was observed to go
/// the same way.
pub fn backspace_range(text: &str, at: usize, opts: Options) -> Range<usize> {
    let start = prev_grapheme_boundary(text, at);
    if start == at {
        return at..at;
    }
    let Some((close, kind)) = lone_char(&text[start..at]).and_then(pair) else {
        return start..at;
    };
    // `~ = $ %` never plant a closer, so there is no pair of theirs to
    // take away.
    if kind == Kind::WrapOnly || !kind.enabled(opts) {
        return start..at;
    }
    match text[at..].chars().next() {
        Some(c) if c == close => start..at + c.len_utf8(),
        _ => start..at,
    }
}

/// The one character a cluster is made of, if it is made of one: a `(`
/// carrying a combining mark is its own cluster and no half of a pair.
fn lone_char(cluster: &str) -> Option<char> {
    let mut chars = cluster.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act_at(text: &str, at: usize, typed: char) -> Act {
        act(text, at..at, typed, Options::default(), false)
    }

    fn act_planted(text: &str, at: usize, typed: char) -> Act {
        act(text, at..at, typed, Options::default(), true)
    }

    /// The probe's positions for the three brackets: the character ahead
    /// decides, and punctuation ahead blocks a bracket too.
    #[test]
    fn brackets_close_on_the_character_ahead() {
        for open in ['(', '[', '{'] {
            let close = pair(open).unwrap().0;
            assert_eq!(act_at("word", 0, open), Act::Insert, "|word {open}");
            assert_eq!(act_at("dont", 3, open), Act::Insert, "don|t {open}");
            assert_eq!(act_at("word", 4, open), Act::Close(close), "word| {open}");
            assert_eq!(act_at("say ", 4, open), Act::Close(close), "say | {open}");
            assert_eq!(
                act_at("say  word", 4, open),
                Act::Close(close),
                "say | word"
            );
            assert_eq!(act_at("say .", 4, open), Act::Insert, "say |. {open}");
            assert_eq!(act_at("say.", 4, open), Act::Close(close), "say.| {open}");
            assert_eq!(act_at("()", 1, open), Act::Close(close), "(|) {open}");
            assert_eq!(act_at("a\nb", 1, open), Act::Close(close), "a|\nb {open}");
        }
        // Mid-line: a space ahead is enough, whatever came before.
        assert_eq!(act_at("para text here", 4, '('), Act::Close(')'));
        assert_eq!(act_at("x = 1", 3, '('), Act::Close(')'));
    }

    /// Quotes and markers share one rule: whitespace or the end ahead,
    /// and whitespace or the start behind.
    #[test]
    fn quotes_and_markers_need_space_on_both_sides() {
        for open in ['"', '\'', '`', '*', '_'] {
            assert_eq!(act_at("word", 0, open), Act::Insert, "|word {open}");
            assert_eq!(act_at("dont", 3, open), Act::Insert, "don|t {open}");
            assert_eq!(act_at("word", 4, open), Act::Insert, "word| {open}");
            assert_eq!(act_at("say ", 4, open), Act::Close(open), "say | {open}");
            assert_eq!(act_at("say  word", 4, open), Act::Close(open), "say | word");
            assert_eq!(act_at("say .", 4, open), Act::Insert, "say |. {open}");
            assert_eq!(act_at("say.", 4, open), Act::Insert, "say.| {open}");
            assert_eq!(act_at("()", 1, open), Act::Insert, "(|) {open}");
            assert_eq!(act_at("\tx ", 3, open), Act::Close(open), "\\tx | {open}");
        }
        assert_eq!(act_at("para text here", 4, '*'), Act::Insert);
        assert_eq!(act_at("x = 1", 3, '*'), Act::Insert);
        // An empty document is the start of a line and the end of one.
        assert_eq!(act_at("", 0, '*'), Act::Close('*'));
        // Inside a closed code span the backtick ahead blocks the pair.
        assert_eq!(act_at("x `say `", 7, '('), Act::Insert);
        assert_eq!(act_at("x `say `", 7, '*'), Act::Insert);
    }

    #[test]
    fn four_characters_never_close() {
        for c in ['~', '=', '$', '%', '<'] {
            assert_eq!(act_at("", 0, c), Act::Insert, "{c} on an empty line");
            assert_eq!(act_at("say ", 4, c), Act::Insert, "say |{c}");
        }
    }

    /// Stepping over a closer is provenance: the editor's own closer is
    /// skipped, for every pair alike, and a planted one is written again.
    #[test]
    fn a_closer_skips_only_the_one_the_editor_planted() {
        for (open, close) in [
            ('(', ')'),
            ('[', ']'),
            ('{', '}'),
            ('"', '"'),
            ('\'', '\''),
            ('`', '`'),
            ('*', '*'),
            ('_', '_'),
        ] {
            let pair: String = [open, close].iter().collect();
            assert_eq!(act_planted(&pair, 1, close), Act::Skip, "{pair}");
            let planted = format!("x{close}");
            assert_eq!(act_at(&planted, 1, close), Act::Insert, "x|{close}");
        }
        // A second bracket nests: the closing bracket ahead lets it close.
        assert_eq!(act_planted("()", 1, '('), Act::Close(')'));
        assert_eq!(act_planted("[[]]", 2, '['), Act::Close(']'));
        // A second `~` is just another character.
        assert_eq!(act_at("~~", 1, '~'), Act::Insert);
    }

    #[test]
    fn every_pair_character_wraps_a_selection_except_the_angle_bracket() {
        for (open, close) in [
            ('(', ')'),
            ('[', ']'),
            ('{', '}'),
            ('"', '"'),
            ('\'', '\''),
            ('`', '`'),
            ('*', '*'),
            ('_', '_'),
            ('~', '~'),
            ('=', '='),
            ('$', '$'),
            ('%', '%'),
        ] {
            let got = act("alpha beta", 0..5, open, Options::default(), false);
            assert_eq!(got, Act::Wrap(open, close), "wrapping with {open}");
        }
        assert_eq!(
            act("alpha beta", 0..5, '<', Options::default(), false),
            Act::Insert
        );
    }

    #[test]
    fn each_toggle_owns_its_own_characters() {
        let brackets = Options {
            brackets: true,
            markdown: false,
        };
        let markdown = Options {
            brackets: false,
            markdown: true,
        };
        let off = Options {
            brackets: false,
            markdown: false,
        };
        assert_eq!(act("say ", 4..4, '(', brackets, false), Act::Close(')'));
        assert_eq!(act("say ", 4..4, '*', brackets, false), Act::Insert);
        assert_eq!(act("say ", 4..4, '(', markdown, false), Act::Insert);
        assert_eq!(act("say ", 4..4, '*', markdown, false), Act::Close('*'));
        for c in ['(', '[', '{', '"', '\'', '`', '*', '_'] {
            assert_eq!(
                act("say ", 4..4, c, off, false),
                Act::Insert,
                "{c} with both off"
            );
            assert_eq!(
                act("a b", 0..1, c, off, false),
                Act::Insert,
                "{c} wrap with both off"
            );
        }
        // The skip is part of the feature, so it goes away with it.
        assert_eq!(act("()", 1..1, ')', off, true), Act::Insert);
        assert_eq!(act("\"\"", 1..1, '"', off, true), Act::Insert);
    }

    #[test]
    fn backspace_takes_an_empty_pair_whole() {
        let opts = Options::default();
        assert_eq!(backspace_range("()", 1, opts), 0..2);
        assert_eq!(backspace_range("**", 1, opts), 0..2);
        assert_eq!(backspace_range("[]", 1, opts), 0..2);
        // Content inside: only the opener goes.
        assert_eq!(backspace_range("(x)", 1, opts), 0..1);
        // A plain character loses one.
        assert_eq!(backspace_range("plain", 3, opts), 2..3);
        // Nothing before the cursor.
        assert_eq!(backspace_range("ab", 0, opts), 0..0);
        // Multi-byte characters still go one character at a time.
        assert_eq!(backspace_range("é", 2, opts), 0..2);
    }

    /// One Backspace takes a whole grapheme cluster, so an accent, a flag
    /// pair or a joined emoji goes with the character it belongs to.
    #[test]
    fn backspace_takes_a_grapheme_cluster_whole() {
        let opts = Options::default();
        assert_eq!(backspace_range("cafe\u{301}", 6, opts), 3..6);
        assert_eq!(backspace_range("\u{1f1eb}\u{1f1f7}", 8, opts), 0..8);
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";
        assert_eq!(backspace_range(family, family.len(), opts), 0..family.len());
        // An opener carrying a combining mark is its own cluster, not half
        // of a pair, so the closer after it stays.
        assert_eq!(backspace_range("(\u{301})", 3, opts), 0..3);
    }

    #[test]
    fn backspace_leaves_pairs_alone_when_their_toggle_is_off() {
        let off = Options {
            brackets: false,
            markdown: false,
        };
        assert_eq!(backspace_range("()", 1, off), 0..1);
        assert_eq!(backspace_range("**", 1, off), 0..1);
        // `~ = $ %` never planted a closer, so they never take two.
        assert_eq!(backspace_range("~~", 1, Options::default()), 0..1);
    }
}
