//! YAML properties block: `---` on line 1, closed by a `---` line.

/// The raw YAML text of a properties block and the byte range it occupies
/// (both fence lines and the newline after the closing fence included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatter<'a> {
    pub yaml: &'a str,
    pub start: usize,
    pub end: usize,
}

/// Extracts the frontmatter block if the document starts with `---`.
/// A UTF-8 BOM before the opening fence is tolerated, as Obsidian does.
pub fn extract_frontmatter(src: &str) -> Option<FrontMatter<'_>> {
    let start = if src.starts_with('\u{feff}') { 3 } else { 0 };
    let body = &src[start..];
    let first_line_end = body.find('\n')?;
    if body[..first_line_end].trim_end_matches('\r') != "---" {
        return None;
    }
    let mut pos = first_line_end + 1;
    loop {
        let line_end = body[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(body.len());
        let line = body[pos..line_end].trim_end_matches('\r');
        // Only `---` closes the block. YAML's `...` document end does not:
        // Obsidian 1.14.4 reads `---\na: 1\n...\n` as having no properties at
        // all and prepends a second block when one is added (observed with a
        // probe plugin), so `...` must not terminate the block here either.
        if line == "---" {
            let yaml = &body[first_line_end + 1..pos];
            let end = if line_end < body.len() {
                line_end + 1
            } else {
                line_end
            };
            return Some(FrontMatter {
                yaml,
                start,
                end: start + end,
            });
        }
        if line_end >= body.len() {
            return None;
        }
        pos = line_end + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_block() {
        let src = "---\ntitle: Hi\ntags: [a, b]\n---\n# Body\n";
        let fm = extract_frontmatter(src).unwrap();
        assert_eq!(fm.yaml, "title: Hi\ntags: [a, b]\n");
        assert_eq!(&src[fm.end..], "# Body\n");
    }

    #[test]
    fn requires_fence_on_first_line() {
        assert!(extract_frontmatter("\n---\na: 1\n---\n").is_none());
        assert!(extract_frontmatter("# Heading\n---\n").is_none());
    }

    #[test]
    fn tolerates_bom_and_crlf() {
        let src = "\u{feff}---\r\nk: v\r\n---\r\nrest";
        let fm = extract_frontmatter(src).unwrap();
        assert_eq!(fm.yaml, "k: v\r\n");
        assert_eq!(&src[fm.end..], "rest");
    }

    #[test]
    fn yaml_document_end_does_not_close_the_block() {
        // Obsidian 1.14.4 does not accept `...` as the closing fence.
        assert!(extract_frontmatter("---\nk: v\n...\nrest\n").is_none());
        // It still closes on a later `---`.
        let fm = extract_frontmatter("---\nk: v\n...\n---\nrest\n").unwrap();
        assert_eq!(fm.yaml, "k: v\n...\n");
    }

    #[test]
    fn unterminated_block_is_not_frontmatter() {
        assert!(extract_frontmatter("---\nk: v\nno end").is_none());
        assert!(extract_frontmatter("---").is_none());
    }
}
