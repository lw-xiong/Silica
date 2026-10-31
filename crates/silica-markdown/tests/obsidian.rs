//! Obsidian Flavored Markdown: the extensions on top of CommonMark, checked
//! both as rendered HTML and as tree shape with source spans.

use silica_markdown::{
    parse, render_html, render_html_with, Block, BlockKind, Fold, HtmlOptions, Inline, InlineKind,
    ListKind, Span,
};

fn html(src: &str) -> String {
    render_html(&parse(src))
}

fn plain_html(src: &str) -> String {
    render_html_with(&parse(src), &HtmlOptions::commonmark())
}

fn paragraph_inlines(src: &str) -> Vec<Inline<'_>> {
    let doc = parse(src);
    match doc.blocks.into_iter().next().map(|b| b.kind) {
        Some(BlockKind::Paragraph { content }) => content,
        other => panic!("expected a paragraph, got {other:?}"),
    }
}

#[test]
fn wikilinks_and_embeds() {
    let src = "See [[Note#Head|alias]] and ![[pic.png|200]].";
    let inlines = paragraph_inlines(src);
    let links: Vec<_> = inlines
        .iter()
        .filter_map(|i| match &i.kind {
            InlineKind::Wikilink { body, embed } => Some((body.as_ref(), *embed, i.span)),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        vec![
            ("Note#Head|alias", false, Span::new(4, 23)),
            ("pic.png|200", true, Span::new(28, 44)),
        ]
    );
    assert_eq!(&src[4..23], "[[Note#Head|alias]]");
    assert_eq!(&src[28..44], "![[pic.png|200]]");
    assert_eq!(
        html(src),
        "<p>See <a data-tooltip-position=\"top\" aria-label=\"Note#Head\" data-href=\"Note#Head\" href=\"Note#Head\" class=\"internal-link\" target=\"_blank\" rel=\"noopener nofollow\">alias</a> and <span alt=\"pic.png\" src=\"pic.png\" class=\"internal-embed\"></span>.</p>\n"
    );
}

#[test]
fn wikilinks_inside_code_and_links_without_alias() {
    assert_eq!(
        html("`[[not a link]]` and [[A#B]]"),
        "<p><code>[[not a link]]</code> and <a data-href=\"A#B\" href=\"A#B\" class=\"internal-link\" target=\"_blank\" rel=\"noopener nofollow\">A &gt; B</a></p>\n"
    );
    // Wikilinks are disabled in CommonMark mode and parse as bracketed text.
    assert_eq!(
        render_html_with(
            &silica_markdown::parse_with("[[x]]", &silica_markdown::Options::commonmark()),
            &HtmlOptions::commonmark()
        ),
        "<p>[[x]]</p>\n"
    );
}

#[test]
fn tags() {
    let inlines = paragraph_inlines("a #tag/nested, #1a but not #123 or x#y or # alone");
    let tags: Vec<_> = inlines
        .iter()
        .filter_map(|i| match &i.kind {
            InlineKind::Tag(name) => Some(*name),
            _ => None,
        })
        .collect();
    assert_eq!(tags, vec!["tag/nested", "1a"]);
    assert_eq!(
        html("#tag"),
        "<p><a href=\"#tag\" class=\"tag\" target=\"_blank\" rel=\"noopener nofollow\">#tag</a></p>\n"
    );
    // `#` followed by a space is a heading, not a tag.
    assert_eq!(html("# Title"), "<h1 data-heading=\"Title\">Title</h1>\n");
}

#[test]
fn block_ids() {
    let doc = parse("para one ^abc-1\n\n- item ^li\n\n| a |\n| - |\n| b |\n\n^tbl\n");
    let ids: Vec<_> = doc
        .blocks
        .iter()
        .map(|b| b.id.map(|i| (i.id, i.span)))
        .collect();
    assert_eq!(ids[0], Some(("abc-1", Span::new(9, 15))));
    assert_eq!(ids[1], None);
    assert_eq!(ids[2], Some(("tbl", Span::new(48, 52))));
    let BlockKind::List { items, .. } = &doc.blocks[1].kind else {
        panic!("list")
    };
    assert_eq!(items[0].children[0].id.map(|i| i.id), Some("li"));
    // The id is not part of the paragraph text.
    assert_eq!(html("para one ^abc-1"), "<p>para one</p>\n");
    assert_eq!(doc.blocks.len(), 3);
}

#[test]
fn callouts() {
    let src = "> [!tip]- Fold *me*\n> body line\n>\n> - list\n\n> [!WARNING]\n> only body\n\n> plain quote";
    let doc = parse(src);
    let BlockKind::Callout {
        callout_type,
        fold,
        title,
        children,
    } = &doc.blocks[0].kind
    else {
        panic!("callout, got {:?}", doc.blocks[0].kind)
    };
    assert_eq!(*callout_type, "tip");
    assert_eq!(*fold, Some(Fold::Collapsed));
    // The title is a block tree now; one paragraph, in the ordinary case.
    assert_eq!(
        silica_markdown::plain_text(silica_markdown::inline_runs(title)[0]),
        "Fold me"
    );
    assert_eq!(children.len(), 2);
    assert!(matches!(children[0].kind, BlockKind::Paragraph { .. }));
    assert!(matches!(children[1].kind, BlockKind::List { .. }));
    assert_eq!(doc.blocks[0].span, Span::new(0, 42));
    let BlockKind::Callout {
        callout_type,
        title,
        ..
    } = &doc.blocks[1].kind
    else {
        panic!("callout")
    };
    assert_eq!(*callout_type, "WARNING");
    assert!(title.is_empty());
    assert!(matches!(doc.blocks[2].kind, BlockKind::BlockQuote { .. }));
    assert_eq!(
        html("> [!note] Hi\n> there"),
        "<div data-callout-metadata=\"\" data-callout-fold=\"\" data-callout=\"note\" class=\"callout\"><div class=\"callout-title\"><div class=\"callout-title-inner\">Hi</div></div><div class=\"callout-content\">\n<p>there</p>\n</div></div>\n"
    );
    assert_eq!(
        html("> [!warning]"),
        "<div data-callout-metadata=\"\" data-callout-fold=\"\" data-callout=\"warning\" class=\"callout\"><div class=\"callout-title\"><div class=\"callout-title-inner\">Warning</div></div></div>\n"
    );
}

/// A fence's info string resolves to a highlighter language.
///
/// The language is the first word, lowercased; a word holding anything but
/// `[A-Za-z0-9_-]` does not resolve, and then the `pre` carries
/// `language-none` while the `code` keeps the raw word beside it. Observed
/// against Obsidian 1.14.4 (tools/probe-gen, 2026-10-29).
#[test]
fn a_fence_info_string_resolves_to_a_language() {
    let classes = |src: &str| {
        let out = html(src);
        let pre = out
            .split("<pre")
            .nth(1)
            .and_then(|t| t.split('>').next())
            .unwrap_or("")
            .to_string();
        let code = out
            .split("<code")
            .nth(1)
            .and_then(|t| t.split('>').next())
            .unwrap_or("")
            .to_string();
        (pre, code)
    };
    let fence = |info: &str| format!("```{info}\ncode line\n```");

    // Resolves: lowercased, on both elements.
    assert_eq!(
        classes(&fence("rust")),
        (
            " class=\"language-rust\"".into(),
            " class=\"language-rust\"".into()
        )
    );
    assert_eq!(
        classes(&fence("JS")),
        (
            " class=\"language-js\"".into(),
            " class=\"language-js\"".into()
        )
    );
    // Unregistered but resolvable keeps its own name.
    assert_eq!(
        classes(&fence("silica-made-up")),
        (
            " class=\"language-silica-made-up\"".into(),
            " class=\"language-silica-made-up\"".into()
        )
    );
    // The first word only.
    assert_eq!(classes(&fence("foo bar")).0, " class=\"language-foo\"");

    // Does not resolve: `language-none` on the pre, the raw word kept on the
    // code. These are real fence words, not curiosities.
    for info in ["c#", "f#", "C++", ".net", "{python}", "a~b"] {
        let (pre, code) = classes(&fence(info));
        assert_eq!(pre, " class=\"language-none\"", "{info}");
        assert_eq!(
            code,
            format!(" class=\"language-{info} language-none\""),
            "{info}"
        );
    }

    // No info string, no class at all.
    assert_eq!(classes(&fence("")), (String::new(), String::new()));

    // CommonMark has neither rule: the word goes on the `code` as written, and
    // the `pre` carries nothing.
    let cm = render_html_with(
        &silica_markdown::parse_with(&fence("C#"), &silica_markdown::Options::commonmark()),
        &HtmlOptions::commonmark(),
    );
    assert!(cm.contains("<pre><code class=\"language-C#\">"), "{cm}");
}

/// Obsidian's underscore emphasis is looser than CommonMark's.
///
/// An underscore run may *open* inside a word, which CommonMark forbids, and a
/// run of two or more may close there as well. A single `_` still cannot close
/// before an alphanumeric, and that is what keeps ordinary identifiers out of
/// it. Observed against Obsidian 1.14.4 (tools/probe-gen, 2026-10-29).
#[test]
fn underscore_emphasis_opens_inside_a_word() {
    // Emphasised, where CommonMark leaves all four literal.
    assert_eq!(html("a_b_"), "<p>a<em>b</em></p>\n");
    assert_eq!(html("__a__b"), "<p><strong>a</strong>b</p>\n");
    assert_eq!(html("a__b__"), "<p>a<strong>b</strong></p>\n");
    assert_eq!(html("x__y__z"), "<p>x<strong>y</strong>z</p>\n");

    // A single `_` cannot close before an alphanumeric, so identifiers and
    // ordinary prose are untouched -- the whole reason this is safe.
    for src in [
        "snake_case_word",
        "a_b_c",
        "_a_b",
        "mid_dle",
        "CONST_NAME_HERE",
        "a_b_c_d_e",
        "file_name.txt",
        "a_1_b",
        "1_2_3",
    ] {
        assert_eq!(html(src), format!("<p>{src}</p>\n"), "{src}");
    }

    // The ordinary forms still work.
    assert_eq!(html("_both_"), "<p><em>both</em></p>\n");
    assert_eq!(html("__dunder__"), "<p><strong>dunder</strong></p>\n");

    // CommonMark mode keeps CommonMark's rule: this is a parse-level quirk, so
    // it needs `Options::commonmark`, not just the plain renderer.
    let cm = |src: &str| {
        render_html_with(
            &silica_markdown::parse_with(src, &silica_markdown::Options::commonmark()),
            &HtmlOptions::commonmark(),
        )
    };
    assert_eq!(cm("a_b_"), "<p>a_b_</p>\n");
    assert_eq!(cm("x__y__z"), "<p>x__y__z</p>\n");
    assert_eq!(cm("_both_"), "<p><em>both</em></p>\n");
}

/// A link ends at the first child that is a link of its own.
///
/// Obsidian never nests `<a>`: it closes the outer link there and does not
/// reopen it, so the text after that child is not part of the link. A tag
/// counts, being an anchor too; an embed or an image does not, being a
/// `<span>`. Observed against Obsidian 1.14.4 (tools/probe-gen, 2026-10-29).
#[test]
fn a_link_ends_at_a_link_inside_it() {
    // `md ` keeps its trailing space inside the anchor, the wikilink is a
    // sibling anchor, and ` link` is outside both.
    let rendered = html("[md [[Links]] link](https://e.com)");
    assert!(
        rendered.contains(r#">md </a><a data-href="Links""#),
        "{rendered}"
    );
    assert!(rendered.ends_with("Links</a> link</p>\n"), "{rendered}");
    // Never two anchors deep.
    assert_eq!(rendered.matches("<a ").count(), 2, "{rendered}");

    // Once closed it stays closed: the text between two wikilinks is bare too.
    let two = html("[md [[Links]] a [[Other]] b](https://e.com)");
    assert!(two.ends_with("Other</a> b</p>\n"), "{two}");
    assert!(two.contains("</a> a <a"), "{two}");

    // A tag is an anchor, so it ends the link as well.
    let tag = html("[md #tag link](https://e.com)");
    // `r##` because the string itself contains `"#`.
    assert!(tag.contains(r##">md </a><a href="#tag""##), "{tag}");

    // An embed and an image are spans, so the link continues through them.
    for src in [
        "[md ![[pic.png]] link](https://e.com)",
        "[md ![alt](a.png) link](https://e.com)",
    ] {
        let out = html(src);
        assert!(out.contains(" link</a>"), "{src}: {out}");
    }

    // Nothing to split: the ordinary link is untouched.
    assert_eq!(html("[plain](https://e.com)").matches("</a>").count(), 1);
    // The split is Obsidian's rule, so CommonMark output is left as it was and
    // still nests. Wikilinks do not occur in the CommonMark suite.
    assert_eq!(
        plain_html("[md [[Links]] link](https://e.com)")
            .matches("</a>")
            .count(),
        2
    );
}

/// A callout's title is block-parsed, not inline-parsed.
///
/// Obsidian runs the title through the block parser, so a heading, a list or a
/// quote written as a title renders as one inside `.callout-title-inner` --
/// which is also why `[!note] -` is not a fold marker: the space ends the
/// header and the lone `-` becomes an empty list. The ordinary one-paragraph
/// title still renders as bare text, with no `<p>`. Observed against Obsidian
/// 1.14.4 (tools/probe-gen, 2026-10-29).
#[test]
fn a_callout_title_is_block_parsed() {
    fn title_blocks(src: &str) -> Vec<BlockKind<'_>> {
        let doc = parse(src);
        let BlockKind::Callout { title, .. } = doc.blocks.into_iter().next().unwrap().kind else {
            panic!("expected a callout for {src:?}")
        };
        title.into_iter().map(|b| b.kind).collect()
    }

    assert!(matches!(
        title_blocks("> [!note] # heading\n> body").as_slice(),
        [BlockKind::Heading { level: 1, .. }]
    ));
    assert!(matches!(
        title_blocks("> [!note] 1. item\n> body").as_slice(),
        [BlockKind::List { .. }]
    ));
    assert!(matches!(
        title_blocks("> [!note] > quote\n> body").as_slice(),
        [BlockKind::BlockQuote { .. }]
    ));
    // A spaced `-` or `+` is title content, not a fold marker, and a lone list
    // marker makes a list with one empty item.
    for src in ["> [!note] -\n> body", "> [!note] +\n> body"] {
        let doc = parse(src);
        let BlockKind::Callout { fold, title, .. } = &doc.blocks[0].kind else {
            panic!("callout")
        };
        assert_eq!(*fold, None, "{src:?}");
        assert!(matches!(title[0].kind, BlockKind::List { .. }), "{src:?}");
    }
    // The fold markers that *are* markers still are.
    let doc = parse("> [!note]- t\n> body");
    let BlockKind::Callout { fold, .. } = &doc.blocks[0].kind else {
        panic!("callout")
    };
    assert_eq!(*fold, Some(Fold::Collapsed));

    // Rendering: an ordinary title stays bare, a heading title becomes an h1,
    // and the spans stay absolute in the source despite the nested parse.
    let rendered = html("> [!note] Hi\n> there");
    assert!(
        rendered.contains("callout-title-inner\">Hi</div>"),
        "{rendered}"
    );
    assert!(!rendered.contains("<p>Hi"), "{rendered}");
    assert!(html("> [!note] # h\n> b").contains("<h1 data-heading=\"h\">"));

    let src = "> [!note] # h\n> b";
    let BlockKind::Callout { title, .. } = &parse(src).blocks[0].kind else {
        panic!("callout")
    };
    let BlockKind::Heading { content, .. } = &title[0].kind else {
        panic!("heading")
    };
    // `h` sits at byte 12 of the whole document, not byte 2 of the title.
    assert_eq!(&src[content[0].span.start..content[0].span.end], "h");
    assert_eq!(content[0].span.start, 12);
}

/// A quote kept inside a list item keeps *all* of its lines there.
///
/// The quirk that puts `> quote` directly after a list item's text inside that
/// item only ever matched the first such line; the rest carried no more of the
/// item's indentation than the first did, so they restarted the quote as a
/// sibling of the whole list. Nothing to do with callouts -- it shows up with a
/// plain quote -- but it is why a callout nested in a list item of a callout
/// lost its body. Observed against Obsidian 1.14.4 (tools/probe-gen,
/// 2026-10-29).
#[test]
fn a_quote_in_a_list_item_keeps_its_later_lines() {
    fn item_children(src: &str) -> Vec<BlockKind<'_>> {
        let doc = parse(src);
        let BlockKind::List { items, .. } = doc.blocks.into_iter().next().unwrap().kind else {
            panic!("expected a list for {src:?}")
        };
        items
            .into_iter()
            .next()
            .expect("an item")
            .children
            .into_iter()
            .map(|b| b.kind)
            .collect()
    }

    // One quote inside the item, holding both lines -- not a second quote
    // outside the list.
    let kinds = item_children("- item\n> quote\n> more");
    assert!(
        matches!(
            kinds.as_slice(),
            [BlockKind::Paragraph { .. }, BlockKind::BlockQuote { .. }]
        ),
        "{kinds:?}"
    );
    let [_, BlockKind::BlockQuote { children }] = kinds.as_slice() else {
        unreachable!()
    };
    assert_eq!(
        children.len(),
        1,
        "both lines in one paragraph: {children:?}"
    );
    assert_eq!(
        html("- item\n> quote\n> more")
            .matches("<blockquote")
            .count(),
        1
    );

    // The same for a callout, whose body used to be left behind.
    let kinds = item_children("- item\n> [!tip] t\n> a\n> b");
    let [_, BlockKind::Callout { children, .. }] = kinds.as_slice() else {
        panic!("expected a callout in the item: {kinds:?}")
    };
    assert_eq!(children.len(), 1, "{children:?}");

    // And nested one level further: a callout in a list item of a callout.
    let doc = parse("> [!note]\n> - item\n> > [!tip] nested\n> > body");
    let BlockKind::Callout { children, .. } = doc.blocks.into_iter().next().unwrap().kind else {
        panic!("outer callout")
    };
    // Just the list -- no stray quote trailing it inside the outer callout.
    assert_eq!(children.len(), 1, "{children:?}");
    assert!(matches!(children[0].kind, BlockKind::List { .. }));
    let rendered = html("> [!note]\n> - item\n> > [!tip] nested\n> > body");
    assert!(!rendered.contains("<blockquote"), "{rendered}");
    assert_eq!(rendered.matches("callout-content").count(), 2, "{rendered}");
}

/// What counts as a callout type, and the title derived from one.
///
/// Observed against Obsidian 1.14.4 (tools/probe-gen, 2026-10-29): the type
/// must have *something* between the brackets, which is then trimmed, and the
/// default title turns `-` into a space but leaves `_` alone.
#[test]
fn callout_type_and_its_default_title() {
    fn title(src: &str) -> String {
        let doc = parse(src);
        let BlockKind::Callout { .. } = doc.blocks[0].kind else {
            panic!("expected a callout for {src:?}")
        };
        let rendered = html(src);
        let inner = rendered
            .split("callout-title-inner\">")
            .nth(1)
            .and_then(|t| t.split("</div>").next())
            .unwrap_or_default();
        inner.to_string()
    }

    // Whitespace between the brackets is a callout with an empty type; a bare
    // `[!]` is not a callout at all.
    for src in ["> [! ]\n> body", "> [!   ]\n> body"] {
        let BlockKind::Callout { callout_type, .. } = parse(src).blocks.remove(0).kind else {
            panic!("expected a callout for {src:?}")
        };
        assert_eq!(callout_type, "", "{src:?}");
        // An empty type has no default title, and shows an empty `data-callout`.
        assert_eq!(title(src), "");
        assert!(html(src).contains("data-callout=\"\""), "{src:?}");
    }
    assert!(matches!(
        parse("> [!]\n> body").blocks[0].kind,
        BlockKind::BlockQuote { .. }
    ));
    // The type is trimmed, so this is a `bad` callout.
    let BlockKind::Callout { callout_type, .. } = parse("> [! bad]\n> b").blocks.remove(0).kind
    else {
        panic!("callout")
    };
    assert_eq!(callout_type, "bad");

    // `-` becomes a space, the whole thing is lowercased, the first character
    // is upper-cased, and `_` is left alone.
    assert_eq!(title("> [!note]\n> b"), "Note");
    assert_eq!(title("> [!TIP]\n> b"), "Tip");
    assert_eq!(title("> [!bad-type-two]\n> b"), "Bad type two");
    assert_eq!(title("> [!NOTE-X]\n> b"), "Note x");
    assert_eq!(title("> [!bad_type]\n> b"), "Bad_type");
    // A title that is written out is used as written, not derived.
    assert_eq!(title("> [!bad-type] Kept-as-is\n> b"), "Kept-as-is");

    // The derivation is shared, so the helper and the renderer cannot drift.
    assert_eq!(
        silica_markdown::callout_default_title("bad-type-two"),
        "Bad type two"
    );
    assert_eq!(silica_markdown::callout_default_title(""), "");
}

/// A callout's first body line gets the same block rules as any other line.
///
/// The header is taken off the quote while parsing, not lifted out of the body
/// paragraph afterwards. It matters because the table and setext rules both ask
/// how many lines the open paragraph holds, and a header still sitting in it
/// made the answer one too many -- so a table or a `---` underline on the line
/// after the header used to come out as literal text. Observed against
/// Obsidian 1.14.4 (tools/probe-gen, 2026-10-29).
#[test]
fn callout_body_starts_a_block_on_its_first_line() {
    fn first_child(src: &str) -> BlockKind<'_> {
        let doc = parse(src);
        let BlockKind::Callout { children, .. } = doc.blocks.into_iter().next().unwrap().kind
        else {
            panic!("expected a callout for {src:?}")
        };
        children.into_iter().next().expect("a body block").kind
    }

    // A table, with and without a title, a fold marker, or a space after `>`.
    for src in [
        "> [!note]\n> | a | b |\n> |---|---|",
        "> [!note] Title\n> | a | b |\n> |---|---|",
        "> [!tip]- Folded\n> | a | b |\n> |---|---|",
        ">[!note]\n>| a | b |\n>|---|---|",
    ] {
        let BlockKind::Table { header, .. } = first_child(src) else {
            panic!("expected a table for {src:?}, got {:?}", first_child(src))
        };
        assert_eq!(header.len(), 2, "{src:?}");
    }

    // A setext underline, which reads the same paragraph-line count.
    for (src, level) in [
        ("> [!note]\n> text\n> ---", 2u8),
        ("> [!note]\n> text\n> ===", 1),
    ] {
        let BlockKind::Heading {
            level: got, setext, ..
        } = first_child(src)
        else {
            panic!("expected a heading for {src:?}")
        };
        assert_eq!((got, setext), (level, true), "{src:?}");
    }

    // And a list, which asks whether it may interrupt a paragraph.
    assert!(matches!(
        first_child("> [!note]\n> - a\n> - b"),
        BlockKind::List { .. }
    ));

    // Taking the header early must not cost the body its lazy continuation:
    // an unprefixed line still belongs to the callout.
    let BlockKind::Callout { children, .. } = parse("> [!note]\ncontent").blocks.remove(0).kind
    else {
        panic!("callout")
    };
    assert_eq!(children.len(), 1);
    assert!(matches!(children[0].kind, BlockKind::Paragraph { .. }));

    // A header with nothing after it still has no body, so no content div.
    assert!(!html("> [!note]").contains("callout-content"));

    // The header belongs to the quote's first block and nowhere else.
    assert!(matches!(
        parse("> text\n> [!note]").blocks[0].kind,
        BlockKind::BlockQuote { .. }
    ));
}

#[test]
fn highlight_strikethrough_and_math() {
    assert_eq!(
        plain_html("==hi== ~~gone~~ $x^2$ $$a+b$$ not $5 and $6"),
        "<p><mark>hi</mark> <del>gone</del> <span class=\"math math-inline\">x^2</span> <span class=\"math math-block\">a+b</span> not $5 and $6</p>\n"
    );
    assert_eq!(
        plain_html("$$\n\\int_0^1 x\n$$\n"),
        "<div class=\"math math-block\">\\int_0^1 x</div>\n"
    );
    let doc = parse("$$\nx\n$$");
    assert_eq!(doc.blocks[0].span, Span::new(0, 7));
    assert!(matches!(&doc.blocks[0].kind, BlockKind::Math { content } if content == "x\n"));
}

/// `editor:toggle-highlight-red` writes `==🔴word==`; the reader takes
/// the emoji as the colour and does not show it (Specification.md,
/// "Observed editing command behavior").
#[test]
fn a_colour_emoji_inside_a_highlight_is_the_colour() {
    assert_eq!(
        html("==\u{1f534}word== and ==\u{1f7e3}two words=="),
        "<p><mark data-highlight=\"red\">word</mark> and <mark data-highlight=\"purple\">two words</mark></p>\n"
    );
    assert_eq!(
        html("==plain=="),
        "<p><mark data-highlight=\"\">plain</mark></p>\n"
    );
    // Anywhere but directly after the opening pair the emoji is text.
    assert_eq!(
        html("==a \u{1f534}b=="),
        "<p><mark data-highlight=\"\">a \u{1f534}b</mark></p>\n"
    );
    let doc = parse("==\u{1f7e1}hi==");
    let BlockKind::Paragraph { content } = &doc.blocks[0].kind else {
        panic!("a paragraph");
    };
    let InlineKind::Highlight { color, children } = &content[0].kind else {
        panic!("a highlight");
    };
    assert_eq!(*color, Some(silica_markdown::HighlightColor::Yellow));
    // The text node's span starts after the emoji.
    assert_eq!(children[0].span, Span::new(6, 8));
    assert!(matches!(&children[0].kind, InlineKind::Text(t) if t == "hi"));
}

#[test]
fn comments() {
    assert_eq!(plain_html("a %%hidden%% b"), "<p>a  b</p>\n");
    let doc = parse("%%\nblock\ncomment\n%%\nafter");
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Comment { content } if *content == "\nblock\ncomment\n")
    );
    assert_eq!(doc.blocks[0].span, Span::new(0, 19));
    assert!(matches!(doc.blocks[1].kind, BlockKind::Paragraph { .. }));
    assert_eq!(plain_html("%%\nblock\n%%\nafter"), "<p>after</p>\n");
}

#[test]
fn tables_with_escaped_pipes() {
    let src = "| a | b |\n| :- | -: |\n| [[x\\|y]] | `c\\|d` |\n";
    let doc = parse(src);
    let BlockKind::Table { header, rows, .. } = &doc.blocks[0].kind else {
        panic!("table")
    };
    assert_eq!(header.len(), 2);
    assert_eq!(rows.len(), 1);
    // The body keeps `\|`; the resolvable target drops the backslash.
    assert!(
        matches!(&rows[0][0].content[0].kind, InlineKind::Wikilink { body, .. } if body == r"x\|y")
    );
    assert_eq!(silica_markdown::link_target(r"x\|y"), "x");
    assert_eq!(silica_markdown::parse_linktext(r"x\|y").alias, Some("y"));
    assert!(matches!(&rows[0][1].content[0].kind, InlineKind::Code(c) if c == "c\\|d"));
    assert_eq!(rows[0][0].span, Span::new(24, 32));
    assert_eq!(&src[24..32], "[[x\\|y]]");
    assert_eq!(
        plain_html("| a |\n| - |\n| 1 |\n"),
        "<table>\n<thead>\n<tr>\n<th>a</th>\n</tr>\n</thead>\n<tbody>\n<tr>\n<td>1</td>\n</tr>\n</tbody>\n</table>\n"
    );
}

#[test]
fn task_lists() {
    let doc = parse("- [ ] open\n- [x] done\n- [?] custom\n- [ ]\n- plain");
    let BlockKind::List { items, kind, .. } = &doc.blocks[0].kind else {
        panic!("list")
    };
    assert_eq!(*kind, ListKind::Bullet { marker: b'-' });
    let tasks: Vec<_> = items.iter().map(|i| i.task).collect();
    // `- [ ]` with nothing after it is plain text in Obsidian.
    assert_eq!(tasks, vec![Some(' '), Some('x'), Some('?'), None, None]);
    assert_eq!(items[1].marker, Span::new(11, 12));
    assert_eq!(
        plain_html("- [x] done"),
        "<ul>\n<li><input type=\"checkbox\" checked=\"\" disabled=\"\" /> done</li>\n</ul>\n"
    );
    assert_eq!(
        html("- [ ] a"),
        "<ul class=\"contains-task-list\">\n<li data-task=\"\" class=\"task-list-item\"><input type=\"checkbox\" class=\"task-list-item-checkbox\">a</li>\n</ul>\n"
    );
}

#[test]
fn footnotes() {
    let src = "Text[^1] and inline^[note here].\n\n[^1]: The *def*.\n";
    assert_eq!(
        plain_html(src),
        "<p>Text<sup data-footnote-id=\"fnref-1\" class=\"footnote-ref\" id=\"fnref-1\"><a data-footref=\"1\" href=\"#fn-1\" class=\"footnote-link\" target=\"_blank\" rel=\"noopener nofollow\">[1]</a></sup> and inline<sup data-footnote-id=\"fnref-2\" class=\"footnote-ref\" id=\"fnref-2\"><a data-footref=\"inline-1\" href=\"#fn-2\" class=\"footnote-link\" target=\"_blank\" rel=\"noopener nofollow\">[2]</a></sup>.</p>\n<section class=\"footnotes\"><hr><ol>\n<li data-footnote-id=\"fn-1\" id=\"fn-1\">\n<p>The <em>def</em>.<a href=\"#fnref-1\" class=\"footnote-backref footnote-link\" target=\"_blank\" rel=\"noopener nofollow\">\u{21a9}\u{fe0e}</a></p>\n</li>\n<li data-footnote-id=\"fn-2\" id=\"fn-2\">note here<a href=\"#fnref-2\" class=\"footnote-backref footnote-link\" target=\"_blank\" rel=\"noopener nofollow\">\u{21a9}\u{fe0e}</a></li>\n</ol></section>\n"
    );
    let doc = parse(src);
    assert_eq!(doc.footnote_labels, vec!["1"]);
    assert!(matches!(
        &doc.blocks[1].kind,
        BlockKind::FootnoteDefinition { label: "1", .. }
    ));
    // An undefined reference renders as its label, as Obsidian does.
    assert_eq!(plain_html("x[^nope]"), "<p>xnope</p>\n");
}

#[test]
fn frontmatter_block() {
    let src = "---\ntitle: T\n---\n# H\n";
    let doc = parse(src);
    assert!(matches!(&doc.blocks[0].kind, BlockKind::Yaml { yaml } if *yaml == "title: T\n"));
    assert_eq!(doc.blocks[0].span, Span::new(0, 16));
    assert_eq!(doc.blocks[1].span, Span::new(17, 20));
    assert_eq!(html(src), "<h1 data-heading=\"H\">H</h1>\n");
}

#[test]
fn bare_urls() {
    assert_eq!(
        plain_html("see https://example.com/a?b=1. or www.x.org/y (http://a.b/c)"),
        "<p>see <a href=\"https://example.com/a?b=1\">https://example.com/a?b=1</a>. or <a href=\"http://www.x.org/y\">www.x.org/y</a> (<a href=\"http://a.b/c\">http://a.b/c</a>)</p>\n"
    );
}

#[test]
fn spans_cover_blocks_and_inlines() {
    let src = "# Head\n\npara *em* text\n\n- a\n- b\n\n```rs\nlet x;\n```\n";
    let doc = parse(src);
    let spans: Vec<_> = doc.blocks.iter().map(|b| b.span.of(src)).collect();
    assert_eq!(
        spans,
        vec!["# Head", "para *em* text", "- a\n- b", "```rs\nlet x;\n```"]
    );
    let BlockKind::Paragraph { content } = &doc.blocks[1].kind else {
        panic!()
    };
    assert_eq!(content[1].span.of(src), "*em*");
    let BlockKind::List { items, .. } = &doc.blocks[2].kind else {
        panic!()
    };
    assert_eq!(items[1].span.of(src), "- b");
    let BlockKind::CodeBlock { info, content, .. } = &doc.blocks[3].kind else {
        panic!()
    };
    assert_eq!(info.as_deref(), Some("rs"));
    assert_eq!(content, "let x;\n");
}

#[test]
fn nested_structure_spans() {
    let src = "> quote\n> - item\n>   more\n";
    let doc = parse(src);
    let Block {
        kind: BlockKind::BlockQuote { children },
        span,
        ..
    } = &doc.blocks[0]
    else {
        panic!()
    };
    assert_eq!(span.of(src), "> quote\n> - item\n>   more");
    let BlockKind::List { items, .. } = &children[1].kind else {
        panic!()
    };
    assert_eq!(items[0].span.of(src), "- item\n>   more");
}

#[test]
fn crlf_and_bom_sources() {
    let src = "\u{feff}---\r\ntags: [a]\r\n---\r\n# H\r\n\r\ntext [[L]]\r\n";
    let doc = parse(src);
    assert!(matches!(doc.blocks[0].kind, BlockKind::Yaml { .. }));
    assert!(matches!(
        &doc.blocks[1].kind,
        BlockKind::Heading { level: 1, .. }
    ));
    let BlockKind::Paragraph { content } = &doc.blocks[2].kind else {
        panic!()
    };
    assert_eq!(content[1].span.of(src), "[[L]]");
}

/// Every CommonMark spec input, plus tricky multibyte and edge inputs, must
/// parse and render under the Obsidian options without panicking.
#[test]
fn obsidian_options_never_panic() {
    let spec = include_str!("data/commonmark-0.31.2.json");
    let mut inputs: Vec<String> = Vec::new();
    for chunk in spec.split("\"markdown\":").skip(1) {
        let s = chunk.trim_start();
        let mut out = String::new();
        let mut chars = s[1..].chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(ch) =
                            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            out.push(ch);
                        }
                    }
                    Some(o) => out.push(o),
                    None => {}
                },
                _ => out.push(c),
            }
        }
        inputs.push(out);
    }
    assert!(inputs.len() >= 600);
    inputs.extend(
        [
            "[[",
            "![[",
            "[[]]",
            "[[é|ü]] #tägß ^",
            "$",
            "$$",
            "$$\n",
            "%%",
            "%%\n",
            "==",
            "~~",
            "> [!",
            "> [!]",
            "> [!x",
            "| é |\n|-|\n| \\| |",
            "- [ ]",
            "- [é] x",
            "^[",
            "^[]",
            "[^",
            "[^]",
            "[^x]: ",
            "---\n",
            "---\nk: v",
            "\u{feff}---\n---\n",
            "a\u{0}b",
            "https://",
            "www.",
            "www.x",
            "<https://é>",
            "*é*_ü_",
            "`é`",
            "#\u{1f600}",
        ]
        .map(String::from),
    );
    for input in &inputs {
        let doc = parse(input);
        let _ = render_html(&doc);
        for i in 0..=input.len() {
            if input.is_char_boundary(i) {
                let doc = parse(&input[..i]);
                let _ = render_html(&doc);
            }
        }
    }
}
