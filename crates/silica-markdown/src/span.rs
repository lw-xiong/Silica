//! Byte spans and line/column positions.

/// Byte range into the source text, `start` inclusive and `end` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Slice of `src` covered by this span.
    pub fn of<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }

    /// Smallest span covering both.
    pub fn union(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// Line and column are zero-based, like Obsidian's `Pos`; `col` counts bytes
/// from the line start and `offset` bytes from the start of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
    pub offset: usize,
}

/// A start and end position, like Obsidian's `Loc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Loc {
    pub start: Pos,
    pub end: Pos,
}

/// Maps byte offsets to line/column positions.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(src: &str) -> LineIndex {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { starts }
    }

    /// Number of lines (a trailing newline starts an empty final line).
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// Byte offset at which line `line` starts.
    pub fn line_start(&self, line: usize) -> usize {
        self.starts[line]
    }

    pub fn pos(&self, offset: usize) -> Pos {
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Pos {
            line: line as u32,
            col: (offset - self.starts[line]) as u32,
            offset,
        }
    }

    pub fn loc(&self, span: Span) -> Loc {
        Loc {
            start: self.pos(span.start),
            end: self.pos(span.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_follow_newlines() {
        let idx = LineIndex::new("ab\ncd\n");
        assert_eq!(idx.line_count(), 3);
        assert_eq!(
            idx.pos(0),
            Pos {
                line: 0,
                col: 0,
                offset: 0
            }
        );
        assert_eq!(
            idx.pos(3),
            Pos {
                line: 1,
                col: 0,
                offset: 3
            }
        );
        assert_eq!(
            idx.pos(4),
            Pos {
                line: 1,
                col: 1,
                offset: 4
            }
        );
        assert_eq!(
            idx.pos(6),
            Pos {
                line: 2,
                col: 0,
                offset: 6
            }
        );
    }
}
