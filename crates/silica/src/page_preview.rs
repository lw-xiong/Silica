//! Page preview popover on hover (Reading View) and Ctrl+hover (Editor).
//!
//! Renders a floating preview card with the target note's title and excerpt,
//! or an image/media preview, or "Note not created yet" indicator.

use crate::reading::model::LinkTarget;
use crate::theme::Theme;
use crate::workspace::VaultState;
use gpui::{
    div, img, px, AnyElement, Context, Div, FontWeight, IntoElement, ObjectFit, ParentElement,
    Pixels, Point, Render, SharedString, Styled, StyledImage, Window,
};
use silica_formats::FileKind;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagePreviewData {
    pub title: SharedString,
    pub subpath: Option<String>,
    pub excerpt: Option<String>,
    pub image_path: Option<PathBuf>,
    pub is_unresolved: bool,
    pub is_external: bool,
    pub external_url: Option<String>,
}

pub struct PagePreviewView {
    pub data: PagePreviewData,
}

impl PagePreviewView {
    pub fn new(data: PagePreviewData) -> Self {
        Self { data }
    }
}

impl Render for PagePreviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        render_preview_card(&self.data, &theme)
    }
}

pub fn render_preview_card(data: &PagePreviewData, theme: &Theme) -> Div {
    let base = theme.base_font_size;
    let mut card = div()
        .w(px(420.))
        .max_w(px(500.))
        .max_h(px(320.))
        .overflow_hidden()
        .p_3()
        .rounded_lg()
        .bg(theme.bg)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .gap_2();

    // Header
    let mut header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .pb_2()
        .border_b_1()
        .border_color(theme.border);

    let title_text = div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_size(px(base * 0.9))
                .text_color(theme.text)
                .child(data.title.clone()),
        );

    let title_row = if let Some(sub) = &data.subpath {
        title_text.child(
            div()
                .text_size(px(base * 0.8))
                .text_color(theme.text_muted)
                .child(SharedString::from(format!(" > #{sub}"))),
        )
    } else {
        title_text
    };

    header = header.child(title_row);

    if data.is_unresolved {
        header = header.child(
            div()
                .text_size(px(base * 0.75))
                .text_color(theme.accent)
                .child("Uncreated"),
        );
    } else if data.is_external {
        header = header.child(
            div()
                .text_size(px(base * 0.75))
                .text_color(theme.text_muted)
                .child("External"),
        );
    }

    card = card.child(header);

    // Body
    if let Some(img_path) = &data.image_path {
        card = card.child(
            div()
                .w_full()
                .max_h(px(240.))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .child(
                    img(img_path.clone())
                        .max_w_full()
                        .max_h(px(230.))
                        .object_fit(ObjectFit::Contain),
                ),
        );
    } else if data.is_unresolved {
        card = card.child(
            div()
                .py_2()
                .text_size(px(base * 0.85))
                .text_color(theme.text_muted)
                .child("This note does not exist yet. Click link to create it."),
        );
    } else if let Some(url) = &data.external_url {
        card = card.child(
            div()
                .py_2()
                .text_size(px(base * 0.85))
                .text_color(theme.link)
                .child(SharedString::from(url.clone())),
        );
    } else if let Some(excerpt) = &data.excerpt {
        let lines: Vec<_> = excerpt
            .lines()
            .take(12)
            .map(|l| {
                if l.trim().is_empty() {
                    div().h(px(6.))
                } else {
                    div()
                        .text_size(px(base * 0.825))
                        .line_height(px(base * 1.35))
                        .text_color(theme.text)
                        .child(SharedString::from(l.to_string()))
                }
            })
            .collect();
        card = card.child(
            div()
                .flex()
                .flex_col()
                .overflow_hidden()
                .children(lines),
        );
    }

    card
}

pub fn render_floating(
    data: &PagePreviewData,
    pos: Point<Pixels>,
    theme: &Theme,
) -> AnyElement {
    let card = render_preview_card(data, theme);
    div()
        .absolute()
        .left(pos.x + px(12.))
        .top(pos.y + px(18.))
        .child(card)
        .into_any_element()
}

fn subpath_to_string(subpath: &silica_markdown::Subpath<'_>) -> String {
    match subpath {
        silica_markdown::Subpath::Headings(h) => format!("#{}", h.join("#")),
        silica_markdown::Subpath::Block(b) => format!("#^{b}"),
    }
}

pub fn resolve_preview(
    target: &LinkTarget,
    source: Option<&str>,
    state: &VaultState,
) -> Option<PagePreviewData> {
    match target {
        LinkTarget::Internal(body) => {
            let parsed = silica_markdown::parse_linktext(body);
            let resolved = state
                .cache()
                .resolve(body, source.unwrap_or_default())
                .map(str::to_string);
            match resolved {
                Some(path) => {
                    let kind = FileKind::from_path(&path);
                    match kind {
                        FileKind::Image => Some(PagePreviewData {
                            title: parsed.path.to_string().into(),
                            subpath: parsed.subpath.as_ref().map(subpath_to_string),
                            excerpt: None,
                            image_path: Some(state.vault().absolute(&path)),
                            is_unresolved: false,
                            is_external: false,
                            external_url: None,
                        }),
                        FileKind::Markdown => {
                            let abs = state.vault().absolute(&path);
                            let text = state
                                .texts()
                                .get(&path)
                                .map(|s| s.to_string())
                                .or_else(|| std::fs::read_to_string(&abs).ok())
                                .unwrap_or_default();
                            let (fm_title, excerpt) = extract_excerpt(&text, 15);
                            let title = fm_title.unwrap_or_else(|| {
                                path.rsplit('/')
                                    .next()
                                    .and_then(|s| s.strip_suffix(".md"))
                                    .unwrap_or(&path)
                                    .to_string()
                            });
                            Some(PagePreviewData {
                                title: title.into(),
                                subpath: parsed.subpath.as_ref().map(subpath_to_string),
                                excerpt: Some(excerpt),
                                image_path: None,
                                is_unresolved: false,
                                is_external: false,
                                external_url: None,
                            })
                        }
                        FileKind::Pdf => Some(PagePreviewData {
                            title: parsed.path.to_string().into(),
                            subpath: parsed.subpath.as_ref().map(subpath_to_string),
                            excerpt: Some("PDF Document".to_string()),
                            image_path: None,
                            is_unresolved: false,
                            is_external: false,
                            external_url: None,
                        }),
                        _ => Some(PagePreviewData {
                            title: parsed.path.to_string().into(),
                            subpath: parsed.subpath.as_ref().map(subpath_to_string),
                            excerpt: Some(format!("File: {path}")),
                            image_path: None,
                            is_unresolved: false,
                            is_external: false,
                            external_url: None,
                        }),
                    }
                }
                None => Some(PagePreviewData {
                    title: parsed.path.to_string().into(),
                    subpath: parsed.subpath.as_ref().map(subpath_to_string),
                    excerpt: None,
                    image_path: None,
                    is_unresolved: true,
                    is_external: false,
                    external_url: None,
                }),
            }
        }
        LinkTarget::External(url) => Some(PagePreviewData {
            title: "External Link".into(),
            subpath: None,
            excerpt: None,
            image_path: None,
            is_unresolved: false,
            is_external: true,
            external_url: Some(url.clone()),
        }),
        LinkTarget::Tag(tag) => Some(PagePreviewData {
            title: format!("#{tag}").into(),
            subpath: None,
            excerpt: Some(format!("Tag: #{tag}")),
            image_path: None,
            is_unresolved: false,
            is_external: false,
            external_url: None,
        }),
        LinkTarget::Footnote(n) => Some(PagePreviewData {
            title: format!("Footnote [{n}]").into(),
            subpath: None,
            excerpt: Some(format!("Footnote reference {n}")),
            image_path: None,
            is_unresolved: false,
            is_external: false,
            external_url: None,
        }),
    }
}

pub fn extract_excerpt(text: &str, max_lines: usize) -> (Option<String>, String) {
    let mut title = None;
    let mut content_lines = Vec::new();
    let mut lines = text.lines().peekable();

    if let Some(&first) = lines.peek() {
        if first.trim() == "---" {
            lines.next();
            for line in lines.by_ref() {
                if line.trim() == "---" {
                    break;
                }
                if let Some(t) = line.strip_prefix("title:") {
                    let cleaned = t.trim().trim_matches('"').trim_matches('\'');
                    if !cleaned.is_empty() {
                        title = Some(cleaned.to_string());
                    }
                }
            }
        }
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !content_lines.is_empty() && content_lines.len() < max_lines {
                content_lines.push("");
            }
            continue;
        }
        if title.is_none() && trimmed.starts_with("# ") {
            title = Some(trimmed[2..].trim().to_string());
            continue;
        }
        content_lines.push(trimmed);
        if content_lines.len() >= max_lines {
            break;
        }
    }

    (title, content_lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_frontmatter_title_and_excerpt() {
        let text = "---\ntitle: Sample Note\n---\n# Sample Note\nFirst paragraph.\n\nSecond paragraph.";
        let (title, excerpt) = extract_excerpt(text, 5);
        assert_eq!(title.as_deref(), Some("Sample Note"));
        assert!(excerpt.contains("First paragraph."));
        assert!(excerpt.contains("Second paragraph."));
    }

    #[test]
    fn extracts_heading_when_no_frontmatter() {
        let text = "# Heading One\nContent here.";
        let (title, excerpt) = extract_excerpt(text, 5);
        assert_eq!(title.as_deref(), Some("Heading One"));
        assert_eq!(excerpt, "Content here.");
    }
}
