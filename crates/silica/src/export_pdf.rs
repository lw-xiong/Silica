//! `silica:export-pdf`: write the open note to a PDF beside it.
//!
//! Obsidian's export prints through the platform's PDF engine. This builds
//! the file itself instead -- no dependency, no spawn of a renderer -- from
//! the note's own text laid out on US Letter pages in the PDF base-14
//! Helvetica and Courier fonts, so a viewer needs nothing the PDF spec does
//! not already promise. What a note looks like is not what it says, so the
//! text is what travels: headings, paragraphs, list items, code, table rows
//! and quoted blocks, in reading order.
//!
//! The writer is the only part of the command that must be exact: a PDF with
//! a wrong byte offset in its xref is a file a viewer refuses to open, so the
//! offsets are computed over the bytes that are actually written and the
//! tests read the file back rather than trusting the shape.

use silica_markdown::ast::{Block, BlockKind, ListKind};
use silica_markdown::plain_text;
use std::path::Path;

/// US Letter, in points: 8.5in x 11in at 72dpi.
pub const PAGE_WIDTH: f32 = 612.0;
/// US Letter height, in points.
pub const PAGE_HEIGHT: f32 = 792.0;
/// Text margins on all four sides, in points.
const MARGIN: f32 = 60.0;
/// Body text size, in points.
const BODY_SIZE: f32 = 11.0;
/// Distance between body baselines, in points.
const BODY_LEADING: f32 = 15.0;
/// Space between blocks, in points.
const BLOCK_GAP: f32 = 8.0;
/// Title size on the first page, in points.
const TITLE_SIZE: f32 = 20.0;
/// Distance between title baselines, in points.
const TITLE_LEADING: f32 = 26.0;
/// Code size, in points; Courier at this size is the PDF's monospace face.
const CODE_SIZE: f32 = 10.0;
/// Distance between code baselines, in points.
const CODE_LEADING: f32 = 13.0;
/// Width of one character of Courier, per font unit.
const MONO_WIDTH: f32 = 600.0;

/// Where a note's export lands: the `.pdf` next to the note, so the vault
/// holds the pair the way Obsidian's "Export to PDF" leaves it.
///
/// The path's own extension is replaced rather than appended: a note is
/// `Notes/Trip.md`, not `Notes/Trip.md.pdf`. A path with no extension gains
/// one, and a path whose name is only an extension (`.hidden`) keeps it as
/// the name and gains the extension, the way [`std::path::Path`] spells it.
pub fn pdf_target(note_path: &str) -> String {
    Path::new(note_path)
        .with_extension("pdf")
        .to_string_lossy()
        .replace('\\', "/")
}

/// Renders `markdown` as the pages of a PDF document headed by `title`.
///
/// The returned bytes are a complete, self-contained PDF file: write them to
/// the path [`pdf_target`] names for the note.
pub fn note_to_pdf(title: &str, markdown: &str) -> Vec<u8> {
    assemble(&layout(title, markdown), title)
}

/// The three faces the PDF needs, all of them base-14: a viewer has them
/// already, so nothing is embedded and nothing is subset.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Font {
    Helvetica,
    HelveticaBold,
    Courier,
}

impl Font {
    /// The resource name this face is bound to in every page's dictionary.
    fn pdf_name(self) -> &'static str {
        match self {
            Font::Helvetica => "F1",
            Font::HelveticaBold => "F2",
            Font::Courier => "F3",
        }
    }

    /// The advance width of `ch`, in points, at a font size of one point.
    fn width(self, ch: char) -> f32 {
        if self == Font::Courier {
            return MONO_WIDTH / 1000.0;
        }
        let code = ch as u32;
        let units = if !(0x20..=0x7e).contains(&code) {
            // Nothing printable here: a width for the shape a replacement
            // will take, so the wrap does not depend on which character
            // the encoder drops.
            556
        } else {
            let table = match self {
                Font::Helvetica => &HELVETICA,
                Font::HelveticaBold | Font::Courier => &HELVETICA_BOLD,
            };
            table[(code - 0x20) as usize]
        };
        f32::from(units) / 1000.0
    }
}

/// Helvetica's advance widths for the printable ASCII range, in font units
/// per thousand.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Helvetica-Bold's advance widths for the printable ASCII range.
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 667, 667, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 333, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 833, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// One run of text on a page.
struct Run {
    font: Font,
    size: f32,
    /// Points from the left edge of the paper.
    x: f32,
    /// Points from the bottom edge of the paper to the baseline.
    y: f32,
    text: String,
}

/// A page is its runs in drawing order, plus its rules.
struct Page {
    items: Vec<Item>,
}

/// What a page draws.
enum Item {
    Run(Run),
    Rule(f32),
}

/// What the note says, laid out: the note's own text in reading order,
/// independent of the PDF syntax that will carry it.
struct Layout {
    pages: Vec<Page>,
    items: Vec<Item>,
    /// Points from the top margin, down to the next baseline.
    cursor: f32,
    indent: f32,
}

impl Layout {
    fn new() -> Layout {
        Layout {
            pages: Vec::new(),
            items: Vec::new(),
            cursor: 0.0,
            indent: 0.0,
        }
    }

    /// Points a line may occupy: the paper less both margins less this
    /// block's own indent.
    fn budget(&self) -> f32 {
        PAGE_WIDTH - MARGIN - (MARGIN + self.indent)
    }

    fn page_break(&mut self) {
        self.pages.push(Page {
            items: std::mem::take(&mut self.items),
        });
        self.cursor = 0.0;
    }

    fn height_left(&self) -> f32 {
        PAGE_HEIGHT - 2.0 * MARGIN - self.cursor
    }

    fn empty_line(&mut self, font: Font, size: f32, leading: f32, text: &str) {
        if self.height_left() < leading {
            self.page_break();
        }
        self.items.push(Item::Run(Run {
            font,
            size,
            x: MARGIN + self.indent,
            y: PAGE_HEIGHT - MARGIN - self.cursor,
            text: text.to_string(),
        }));
        self.cursor += leading;
    }

    /// A blank space of `amount` points, which never starts a page and so
    /// never leaves a gap at the top of one.
    fn gap(&mut self, amount: f32) {
        if self.cursor == 0.0 {
            return;
        }
        self.cursor += amount;
        if self.height_left() <= 0.0 {
            self.page_break();
        }
    }

    /// Draws a rule at the cursor and moves past it.
    fn rule(&mut self) {
        if self.height_left() < 6.0 {
            self.page_break();
        }
        self.items
            .push(Item::Rule(PAGE_HEIGHT - MARGIN - self.cursor));
        self.cursor += BLOCK_GAP;
    }

    fn finish(mut self) -> Vec<Page> {
        if !self.items.is_empty() || self.pages.is_empty() {
            self.pages.push(Page { items: self.items });
        }
        self.pages
    }
}

/// The note's text on pages, starting with its title.
fn layout(title: &str, markdown: &str) -> Vec<Page> {
    let document = silica_markdown::parse(markdown);
    let mut out = Layout::new();
    out.gap(BLOCK_GAP);
    out.lines(Font::HelveticaBold, TITLE_SIZE, TITLE_LEADING, "", title);
    out.gap(BLOCK_GAP);
    for block in &document.blocks {
        out.block(block);
    }
    out.finish()
}

impl Layout {
    /// A block of the note, in reading order.
    fn block(&mut self, block: &Block<'_>) {
        match &block.kind {
            BlockKind::Yaml { yaml } => {
                // The properties block is part of the note, so it travels:
                // its own lines, as written, in the monospace face a viewer
                // always has.
                self.gap(BLOCK_GAP);
                self.lines(Font::Courier, CODE_SIZE, CODE_LEADING, "", "Properties");
                self.gap(4.0);
                self.indent += 12.0;
                for line in yaml.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    self.lines(Font::Courier, CODE_SIZE, CODE_LEADING, "", line);
                }
                self.indent -= 12.0;
                self.gap(BLOCK_GAP);
            }
            BlockKind::Paragraph { content } => {
                self.paragraph(content);
                self.gap(BLOCK_GAP);
            }
            BlockKind::Heading { level, content, .. } => {
                self.gap(BLOCK_GAP * 1.5);
                let (size, leading) = match level {
                    1 => (16.0, 21.0),
                    2 => (14.0, 19.0),
                    3 => (12.5, 17.0),
                    _ => (11.5, 15.5),
                };
                self.lines(Font::HelveticaBold, size, leading, "", &plain_text(content));
                self.gap(BLOCK_GAP * 0.5);
            }
            BlockKind::ThematicBreak => {
                self.gap(BLOCK_GAP);
                self.rule();
            }
            BlockKind::CodeBlock { content, .. } => {
                // Every line is written as the note wrote it: no wrapping,
                // no reflow, the indentation intact.
                self.gap(BLOCK_GAP);
                let prefix = "    ";
                for line in content.lines() {
                    if line.trim().is_empty() {
                        self.empty_line(Font::Courier, CODE_SIZE, CODE_LEADING, prefix);
                    } else {
                        self.lines(Font::Courier, CODE_SIZE, CODE_LEADING, prefix, line);
                    }
                }
                self.gap(BLOCK_GAP);
            }
            BlockKind::HtmlBlock { content, .. } => {
                // Raw HTML is not what a PDF shows, but it is not this
                // export's business to decide what the note meant by it:
                // the lines are printed as the note holds them.
                self.gap(BLOCK_GAP);
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        self.lines(Font::Courier, CODE_SIZE, CODE_LEADING, "", line);
                    }
                }
                self.gap(BLOCK_GAP);
            }
            BlockKind::BlockQuote { children } => self.nested(children, 16.0),
            BlockKind::Callout {
                title, children, ..
            } => {
                self.gap(BLOCK_GAP);
                for head in title {
                    if let BlockKind::Paragraph { content } = &head.kind {
                        let callout = plain_text(content);
                        if !callout.trim().is_empty() {
                            self.lines(Font::HelveticaBold, BODY_SIZE, BODY_LEADING, "", &callout);
                        }
                    }
                }
                self.nested(children, 16.0);
            }
            BlockKind::List { kind, items, .. } => {
                self.gap(BLOCK_GAP);
                for (index, item) in items.iter().enumerate() {
                    self.list_item(item, index, kind);
                }
                self.gap(BLOCK_GAP);
            }
            BlockKind::Table { header, rows, .. } => {
                self.gap(BLOCK_GAP);
                let head: Vec<String> = header
                    .iter()
                    .map(|cell| plain_text(&cell.content).trim().to_string())
                    .collect();
                self.lines(
                    Font::HelveticaBold,
                    BODY_SIZE,
                    BODY_LEADING,
                    "",
                    &head.join("  |  "),
                );
                for row in rows {
                    let row: Vec<String> = row
                        .iter()
                        .map(|cell| plain_text(&cell.content).trim().to_string())
                        .collect();
                    self.lines(
                        Font::Courier,
                        CODE_SIZE,
                        CODE_LEADING,
                        "",
                        &row.join("  |  "),
                    );
                }
                self.gap(BLOCK_GAP);
            }
            BlockKind::Math { content } => {
                self.gap(BLOCK_GAP);
                self.lines(Font::Courier, CODE_SIZE, CODE_LEADING, "", content);
                self.gap(BLOCK_GAP);
            }
            // A comment is never rendered anywhere, so it is not rendered
            // here either.
            BlockKind::Comment { .. } => {}
            BlockKind::FootnoteDefinition { label, children } => {
                self.gap(BLOCK_GAP);
                let prefix = format!("[{label}] ");
                for child in children {
                    if let BlockKind::Paragraph { content } = &child.kind {
                        self.lines(
                            Font::Courier,
                            CODE_SIZE,
                            CODE_LEADING,
                            &prefix,
                            &plain_text(content),
                        );
                    }
                }
                self.gap(BLOCK_GAP);
            }
        }
    }

    /// Children drawn one step in from the parent's own indent.
    fn nested(&mut self, children: &[Block<'_>], by: f32) {
        self.indent += by;
        for child in children {
            self.block(child);
        }
        self.indent -= by;
    }

    fn list_item(
        &mut self,
        item: &silica_markdown::ast::ListItem<'_>,
        index: usize,
        kind: &ListKind,
    ) {
        self.indent += 14.0;
        // The number is the list's own start plus this item's place, so a
        // list that begins at 3 is numbered from 3.
        let bullet = match kind {
            ListKind::Bullet { .. } => "\u{2022} ".to_string(),
            ListKind::Ordered { start, delimiter } => {
                let _ = delimiter;
                format!("{}. ", start + index as u64)
            }
        };
        let task = match item.task {
            Some(' ') => "[ ] ",
            Some(_) => "[x] ",
            None => "",
        };
        let prefix = format!("{bullet}{task}");
        let mut first = true;
        for child in &item.children {
            match &child.kind {
                BlockKind::Paragraph { content } => {
                    self.lines(
                        Font::Helvetica,
                        BODY_SIZE,
                        BODY_LEADING,
                        if first { &prefix } else { "" },
                        &plain_text(content),
                    );
                    first = false;
                }
                BlockKind::List { kind, items, .. } => {
                    for (index, item) in items.iter().enumerate() {
                        self.list_item(item, index, kind);
                    }
                    first = false;
                }
                _ => {
                    self.block(child);
                    first = false;
                }
            }
        }
        self.indent -= 14.0;
    }

    /// The text of a paragraph.
    fn paragraph(&mut self, content: &[silica_markdown::ast::Inline<'_>]) {
        self.lines(
            Font::Helvetica,
            BODY_SIZE,
            BODY_LEADING,
            "",
            &plain_text(content),
        );
    }

    /// Wraps `text` into lines at this block's indent and draws them,
    /// `prefix` riding on the first line only.
    ///
    /// Words wider than the line are broken mid-word rather than left
    /// hanging past the margin: a viewer clips nothing, so nothing is put
    /// there in the first place.
    fn lines(&mut self, font: Font, size: f32, leading: f32, prefix: &str, text: &str) {
        if font == Font::Courier {
            self.mono_lines(font, size, leading, prefix, text);
            return;
        }
        let limit = self.budget();
        let space = font.width(' ') * size;
        let mut taken = measure(prefix, font, size);
        let mut line = String::from(prefix);
        let mut started = false;
        for word in text.split(' ') {
            if word.is_empty() {
                continue;
            }
            let width = measure(word, font, size);
            let room = limit - taken - if started { space } else { 0.0 };
            if started && width <= room {
                line.push(' ');
                line.push_str(word);
                taken += space + width;
                continue;
            }
            if started {
                self.empty_line(font, size, leading, &line);
                line = String::new();
                // `taken` is set again below, by whatever starts the next
                // line: the prefix rides the first line only.
            }
            if width > limit {
                // A word that cannot fit on a line of its own.
                let mut chunk = String::new();
                let mut chunk_width = 0.0;
                for ch in word.chars() {
                    let ch_width = font.width(ch) * size;
                    if chunk_width + ch_width > limit {
                        self.empty_line(font, size, leading, &chunk);
                        chunk.clear();
                        chunk_width = 0.0;
                    }
                    chunk.push(ch);
                    chunk_width += ch_width;
                }
                line.push_str(&chunk);
                taken = chunk_width;
                started = !chunk.is_empty();
                continue;
            }
            line.push_str(word);
            taken = width;
            started = true;
        }
        if started || !line.is_empty() {
            self.empty_line(font, size, leading, &line);
        }
    }

    /// Courier is one width for every character, so a line is wrapped by
    /// counting characters and the note's own indentation survives it.
    fn mono_lines(&mut self, font: Font, size: f32, leading: f32, prefix: &str, text: &str) {
        let step = (font.width(' ') * size).max(1.0);
        let limit = self.budget();
        let columns = ((limit - measure(prefix, font, size)) / step)
            .floor()
            .max(1.0) as usize;
        let mut chars = text.chars();
        let mut line = String::from(prefix);
        let mut used = 0;
        loop {
            match chars.next() {
                None => {
                    self.empty_line(font, size, leading, &line);
                    break;
                }
                Some(ch) => {
                    if used >= columns {
                        self.empty_line(font, size, leading, &line);
                        line = String::new();
                        used = 0;
                    }
                    line.push(ch);
                    used += 1;
                }
            }
        }
    }
}

/// The width of `text` in `font` at `size`, in points.
fn measure(text: &str, font: Font, size: f32) -> f32 {
    text.chars().map(|ch| font.width(ch) * size).sum()
}

/// The bytes of a PDF holding these pages: header, objects, xref, trailer.
fn assemble(pages: &[Page], title: &str) -> Vec<u8> {
    // 1 catalog, 2 pages, 3-5 the three fonts, then two objects per page,
    // and the info dictionary last.
    let info = 6 + 2 * pages.len();
    let total = info;
    let mut objects: Vec<Vec<u8>> = Vec::with_capacity(total);
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", 7 + 2 * i))
        .collect();
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        )
        .into_bytes(),
    );
    for name in ["Helvetica", "Helvetica-Bold", "Courier"] {
        objects.push(
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /{name} \
                 /Encoding /WinAnsiEncoding >>"
            )
            .into_bytes(),
        );
    }
    for (index, page) in pages.iter().enumerate() {
        let content = content_stream(page);
        objects.push(content);
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >> >> \
                 /Contents {} 0 R >>",
                PAGE_WIDTH,
                PAGE_HEIGHT,
                6 + 2 * index
            )
            .into_bytes(),
        );
    }
    objects.push(
        format!(
            "<< /Producer {} /Title {} >>",
            literal("Silica"),
            literal(title)
        )
        .into_bytes(),
    );

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n");
    let mut offsets = vec![0usize; total];
    for (index, body) in objects.iter().enumerate() {
        offsets[index] = out.len();
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let startxref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", total + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{startxref}\n%%EOF\n",
            total + 1,
            info
        )
        .as_bytes(),
    );
    out
}

/// A page's content stream: one text object per run, and the rules between
/// them. Written uncompressed -- `/Length` counts the bytes either way, and
/// a stream a viewer can read as text is one a person can read too.
fn content_stream(page: &Page) -> Vec<u8> {
    let mut body = String::new();
    for item in &page.items {
        match item {
            Item::Run(run) => {
                body.push_str("BT\n");
                body.push_str(&format!(
                    "/{} {} Tf\n1 0 0 1 {} {} Tm\n{} Tj\nET\n",
                    run.font.pdf_name(),
                    run.size,
                    trim_float(run.x),
                    trim_float(run.y),
                    literal(&run.text)
                ));
            }
            Item::Rule(y) => {
                body.push_str(&format!(
                    "0.75 w 0.45 0.45 0.45 RG {} {} m {} {} l S\n",
                    trim_float(MARGIN),
                    trim_float(*y),
                    trim_float(PAGE_WIDTH - MARGIN),
                    trim_float(*y)
                ));
            }
        }
    }
    let mut out = format!("<< /Length {} >>\nstream\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(b"\nendstream");
    out
}

/// A PDF literal string: the text as WinAnsi bytes, with the three
/// characters that are syntax in a string escaped and nothing else touched.
fn literal(text: &str) -> String {
    let mut out = String::from("(");
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            _ => out.push_str(&encode_char(ch)),
        }
    }
    out.push(')');
    out
}

/// One character as the WinAnsi bytes a base-14 font reads. A character
/// the encoding has no place for becomes a `?`, because a viewer's
/// replacement would be a width and a glyph this file cannot promise.
fn encode_char(ch: char) -> String {
    let code = ch as u32;
    if code < 0x20 || code == 0x7f {
        // Control bytes are not text anywhere: they come back as spaces.
        return String::from(" ");
    }
    let byte = if code < 0x80 {
        code as u8
    } else {
        match ch {
            '\u{20ac}' => 0x80,
            '\u{201a}' => 0x82,
            '\u{0192}' => 0x83,
            '\u{201e}' => 0x84,
            '\u{2026}' => 0x85,
            '\u{2020}' => 0x86,
            '\u{2021}' => 0x87,
            '\u{02c6}' => 0x88,
            '\u{2030}' => 0x89,
            '\u{0160}' => 0x8a,
            '\u{2039}' => 0x8b,
            '\u{0152}' => 0x8c,
            '\u{017d}' => 0x8e,
            '\u{2018}' => 0x91,
            '\u{2019}' => 0x92,
            '\u{201c}' => 0x93,
            '\u{201d}' => 0x94,
            '\u{2022}' => 0x95,
            '\u{2013}' => 0x96,
            '\u{2014}' => 0x97,
            '\u{02dc}' => 0x98,
            '\u{2122}' => 0x99,
            '\u{0161}' => 0x9a,
            '\u{203a}' => 0x9b,
            '\u{0153}' => 0x9c,
            '\u{017e}' => 0x9e,
            '\u{0178}' => 0x9f,
            _ if (0xa0..=0xff).contains(&code) => code as u8,
            _ => b'?',
        }
    };
    if byte < 0x20 || (0x7f..=0xff).contains(&byte) {
        format!("\\{byte:03o}")
    } else {
        (byte as char).to_string()
    }
}

/// A coordinate as a PDF number: the shortest spelling that keeps its
/// value, so a reader of the file is not shown `60.000000`.
fn trim_float(value: f32) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded == rounded.trunc() {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PDF read back: where each object sits, and the strings each page's
    /// content stream shows. Written against the bytes rather than the
    /// writer's own structures, so a page that is missing shows up here.
    #[derive(Default)]
    struct Opened {
        objects: Vec<(usize, String)>,
        shown: Vec<String>,
        pages: usize,
    }

    /// The file as text, with the header's binary marker put back where it
    /// belongs. Those four bytes are what marks a PDF as binary and are not
    /// text, so they are traded for spaces of the same count: every offset
    /// in the xref then still lands on the byte it was written against.
    fn as_text(pdf: &[u8]) -> String {
        let mut bytes = pdf.to_vec();
        let marker = b"%\xe2\xe3\xcf\xd3\n";
        if let Some(at) = bytes
            .windows(marker.len())
            .position(|window| window == marker.as_slice())
        {
            for byte in &mut bytes[at..at + marker.len()] {
                *byte = b' ';
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn open(pdf: &[u8]) -> Opened {
        let text = as_text(pdf);
        let mut out = Opened {
            pages: text.matches("/Type /Page /Parent").count(),
            ..Opened::default()
        };
        let mut line_starts = text
            .char_indices()
            .filter(|(_, c)| c == &'\n')
            .map(|(i, _)| i + 1)
            .collect::<Vec<usize>>();
        line_starts.insert(0, 0);
        for (offset, _) in text.match_indices(" obj") {
            let start = line_starts
                .iter()
                .rev()
                .copied()
                .find(|start| *start <= offset)
                .unwrap_or(0);
            let head = text[start..offset].trim();
            out.objects.push((
                start,
                head.split_whitespace().next().unwrap_or("").to_string(),
            ));
        }
        for stream in text.split("stream").skip(1) {
            let body = stream.split("endstream").next().unwrap_or("");
            out.shown.extend(literals(body));
        }
        out
    }

    /// The literal strings a content stream shows, escapes resolved, in
    /// order, with WinAnsi turned back into characters.
    fn literals(body: &str) -> Vec<String> {
        let bytes = body.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'(' {
                i += 1;
                continue;
            }
            i += 1;
            let mut depth = 1;
            let mut text = Vec::new();
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    b'(' => {
                        depth += 1;
                        text.push(b'(');
                        i += 1;
                    }
                    b')' => {
                        depth -= 1;
                        if depth > 0 {
                            text.push(b')');
                        }
                        i += 1;
                    }
                    b'\\' => {
                        i += 1;
                        match bytes.get(i) {
                            Some(b'n') | Some(b'r') | Some(b't') | Some(b'b') | Some(b'f') => {
                                text.push(b' ');
                                i += 1;
                            }
                            Some(b'\\') => {
                                text.push(b'\\');
                                i += 1;
                            }
                            Some(b'(') => {
                                text.push(b'(');
                                i += 1;
                            }
                            Some(b')') => {
                                text.push(b')');
                                i += 1;
                            }
                            Some(d) if (b'0'..=b'7').contains(d) => {
                                let mut code = 0;
                                for _ in 0..3 {
                                    match bytes.get(i) {
                                        Some(d) if (b'0'..=b'7').contains(d) => {
                                            code = code * 8 + (d - b'0');
                                            i += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                text.push(code);
                            }
                            _ => {}
                        }
                    }
                    other => {
                        text.push(other);
                        i += 1;
                    }
                }
            }
            if depth == 0 {
                out.push(text.iter().map(|b| winansi(*b)).collect());
            }
        }
        out
    }

    /// A WinAnsi byte as the character a base-14 font reads from it.
    fn winansi(byte: u8) -> char {
        match byte {
            0x80 => '\u{20ac}',
            0x82 => '\u{201a}',
            0x83 => '\u{0192}',
            0x84 => '\u{201e}',
            0x85 => '\u{2026}',
            0x86 => '\u{2020}',
            0x87 => '\u{2021}',
            0x88 => '\u{02c6}',
            0x89 => '\u{2030}',
            0x8a => '\u{0160}',
            0x8b => '\u{2039}',
            0x8c => '\u{0152}',
            0x8e => '\u{017d}',
            0x91 => '\u{2018}',
            0x92 => '\u{2019}',
            0x93 => '\u{201c}',
            0x94 => '\u{201d}',
            0x95 => '\u{2022}',
            0x96 => '\u{2013}',
            0x97 => '\u{2014}',
            0x98 => '\u{02dc}',
            0x99 => '\u{2122}',
            0x9a => '\u{0161}',
            0x9b => '\u{203a}',
            0x9c => '\u{0153}',
            0x9e => '\u{017e}',
            0x9f => '\u{0178}',
            byte => byte as char,
        }
    }

    #[test]
    fn the_note_lands_as_a_pdf_beside_it() {
        assert_eq!(pdf_target("Trip.md"), "Trip.pdf");
        assert_eq!(pdf_target("Notes/Trip.md"), "Notes/Trip.pdf");
        assert_eq!(pdf_target("Notes/Trip.markdown"), "Notes/Trip.pdf");
        assert_eq!(pdf_target("Notes.v2/Trip"), "Notes.v2/Trip.pdf");
        assert_eq!(pdf_target("Trip"), "Trip.pdf");
    }

    /// The whole command, at the file: the note's own path becomes the PDF's
    /// path, and what is read back out of it is a PDF.
    #[test]
    fn the_export_lands_beside_the_note_as_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("Notes/Trip.md");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::write(&note, "# Trip\n\nHello.\n").unwrap();
        let target = dir.path().join(pdf_target("Notes/Trip.md"));

        std::fs::write(&target, note_to_pdf("Trip", "# Trip\n\nHello.\n")).unwrap();

        let bytes = std::fs::read(&target).unwrap();
        assert!(bytes.starts_with(b"%PDF-"), "the file is not a PDF");
        assert_eq!(note.with_extension("pdf"), target);
        assert!(opened_keeps_its_note(&bytes));
    }

    fn opened_keeps_its_note(pdf: &[u8]) -> bool {
        open(pdf).shown.join("\n").contains("Hello.")
    }

    #[test]
    fn a_pdf_is_a_pdf_from_its_first_byte_to_its_last() {
        let pdf = note_to_pdf("Trip", "# Trip\n\nHello (world).\n");
        assert!(pdf.starts_with(b"%PDF-"), "no PDF header");
        let tail = String::from_utf8_lossy(&pdf[pdf.len() - 64..]).to_string();
        assert!(tail.contains("startxref"), "no startxref");
        assert!(tail.trim_end().ends_with("%%EOF"), "no EOF marker");
    }

    /// A PDF with a wrong xref is a file a viewer refuses to open, so the
    /// offsets in the table are read back and compared with where each
    /// object really sits.
    #[test]
    fn every_xref_offset_lands_on_its_object() {
        let pdf = note_to_pdf("Trip", "# Trip\n\nHello.\n");
        let text = as_text(&pdf);
        let (table, size) = xref_table(&text);
        assert!(size >= 5, "only {size} objects");
        for (number, offset) in &table {
            let at = (*offset).min(text.len());
            let head = format!("{number} 0 obj");
            assert_eq!(
                &text[at..(at + head.len()).min(text.len())],
                head,
                "xref says object {number} is at {offset}, which is not {number} 0 obj"
            );
        }
        // Every entry but the first is an object in use; the first is the
        // free object the table always opens with.
        assert_eq!(table.len() + 1, size, "one xref entry is the free object");
    }

    #[test]
    fn the_pages_show_the_note_and_repeat_it_over_as_many_pages_as_it_needs() {
        let mut markdown = String::from("# Trip\n\n");
        for i in 0..200 {
            markdown.push_str(&format!("Line {i} of the trip.\n\n"));
        }
        let pdf = note_to_pdf("Trip", &markdown);
        let opened = open(&pdf);
        let shown = opened.shown.join("\n");
        assert!(shown.contains("Trip"), "the title is not on any page");
        assert!(
            shown.contains("Line 0 of the trip."),
            "the first line is missing"
        );
        assert!(
            shown.contains("Line 199 of the trip."),
            "the last line is missing"
        );
        assert!(
            opened.pages >= 2,
            "200 lines fit on one page: {} pages",
            opened.pages
        );
        assert!(
            opened.shown.len() > 200,
            "200 lines did not lay out across pages: {} runs",
            opened.shown.len()
        );
    }

    /// The parentheses a note holds are literal PDF syntax, so what the note
    /// says has to survive the trip through the string writer both ways.
    #[test]
    fn what_the_note_says_is_what_the_page_shows() {
        let pdf = note_to_pdf(
            "Trip",
            "Back\\slash and (parentheses) and a curly \u{2019}quote\u{2019}.\n",
        );
        let opened = open(&pdf);
        let shown = opened.shown.join("\n");
        assert!(
            shown.contains("Back\\slash and (parentheses) and a curly \u{2019}quote\u{2019}."),
            "escaped text did not round-trip: {shown}"
        );
    }

    #[test]
    fn the_note_is_not_one_runaway_line() {
        let long = "word ".repeat(200);
        let pdf = note_to_pdf("Trip", &long);
        for run in open(&pdf).shown {
            assert!(
                run.len() <= 110,
                "one line holds {} bytes: {run}",
                run.len()
            );
        }
    }

    fn xref_table(text: &str) -> (Vec<(usize, usize)>, usize) {
        let start = text.rfind("startxref").expect("no startxref in the file") + "startxref".len();
        let offset: usize = text[start..]
            .split_whitespace()
            .next()
            .expect("no offset after startxref")
            .parse()
            .expect("the offset is not a number");
        let (head, entries) = text[offset..]
            .split_once("xref\n")
            .expect("the offset does not land on the xref table");
        assert!(head.trim().is_empty(), "startxref points into an object");
        let (subsection, rows) = entries
            .split_once('\n')
            .expect("no subsection header under xref");
        let (first, count) = subsection
            .split_once(' ')
            .expect("the subsection header has no object range");
        let first: usize = first.trim().parse().expect("not a number");
        let count: usize = count.trim().parse().expect("not a number");
        let mut out = Vec::new();
        for (i, row) in rows.lines().take(count).enumerate() {
            let mut fields = row.split(' ');
            let at: usize = fields
                .next()
                .expect("a row has no offset")
                .parse()
                .expect("an offset is not a number");
            let _generation = fields.next().expect("a row has no generation");
            if fields.next() == Some("n") {
                out.push((first + i, at));
            }
        }
        (out, count)
    }
}
