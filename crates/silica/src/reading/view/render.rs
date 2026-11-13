//! The reading view's paint code: building the rendered blocks, widgets,
//! images and tables into the scrollable frame — the methods of the
//! split-out render impl, unchanged apart from living in their own module.
//! Split from `reading/view.rs` per the audit's module-size finding.

use super::*;

impl Render for ReadingView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let base = theme.base_font_size;
        let view = cx.entity().downgrade();
        div()
            .key_context("ReadingView")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::field_key_down))
            .on_key_down(cx.listener(Self::inline_title_key_down))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::scroll_line_down))
            .on_action(cx.listener(Self::scroll_line_up))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .size_full()
            .flex()
            .flex_col()
            .font_family(theme.text_font.clone())
            .text_size(px(base))
            .line_height(px(base * LINE_HEIGHT))
            .text_color(theme.text)
            .relative()
            .children(
                self.inline_title
                    .then(|| self.render_inline_title(base, cx)),
            )
            .child(
                div().flex_1().min_h_0().child(
                    list(self.list.clone(), move |ix, window, cx| {
                        view.update(cx, |this, cx| this.render_block(ix, window, cx))
                            .unwrap_or_else(|_| div().into_any_element())
                    })
                    .size_full(),
                ),
            )
            .children(
                (self.backlinks_in_document && !self.document_backlinks.is_empty())
                    .then(|| self.render_document_backlinks(base, cx)),
            )
            .child(
                // * Records the painted width; a change lays the blocks out again.
                canvas(
                    {
                        let view = cx.entity();
                        move |bounds, _, cx| {
                            view.update(cx, |this, cx| {
                                if this.width != bounds.size.width {
                                    this.width = bounds.size.width;
                                    cx.notify();
                                }
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }


}

/// Builds GPUI elements for model blocks; the reading view and Live
/// Preview's widgets share it, each with its own click handlers.
pub(crate) struct Renderer<'a> {
    theme: &'a Theme,
    toggled: &'a HashSet<usize>,
    /// The vault's `types.json`, so a property row can draw the type its key
    /// is assigned rather than only the one its value infers.
    property_types: &'a PropertyTypes,
    /// The vault-relative path of the note being rendered, so the reading
    // * view can write property changes back to the vault file. Empty for
    //   Live Preview, which edits through its own buffer.
    path: String,
    /// Weak handle to the reading view, so property changes can be emitted
    // * as events for the workspace to write to the vault.
    view: WeakEntity<ReadingView>,
    /// Set by Live Preview to make the properties block editable.
    properties_host: Option<&'a PropertiesHost>,
    /// Set by Live Preview to make a table's cells editable.
    table_host: Option<&'a TableHost>,
    /// Set by the reading view for the affordances only it draws.
    reading_host: Option<&'a ReadingHost>,
    on_link: LinkHandler,
    on_toggle: ToggleHandler,
    /// Play a local video; `None` where nothing can, so Live Preview's
    /// widget keeps the one click path it has.
    video_play: Option<VideoHandler>,
    /// What the player is showing, shared with the view that owns it.
    video: Option<Rc<RefCell<Option<VideoState>>>>,
    /// Where a canvas embed asks for its element; `None` where nothing can
    /// answer, so Live Preview's widget keeps the card.
    on_canvas: Option<CanvasHandler>,
    text_system: Arc<WindowTextSystem>,
    /// Inner width of a top-level block, in pixels.
    available: f32,
    /// The reading view's current selection, for highlighting selected text.
    selection: Option<&'a [(usize, usize, usize, usize)]>,
}

impl<'a> Renderer<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        theme: &'a Theme,
        text_system: Arc<WindowTextSystem>,
        available: f32,
        toggled: &'a HashSet<usize>,
        property_types: &'a PropertyTypes,
        properties_host: Option<&'a PropertiesHost>,
        table_host: Option<&'a TableHost>,
        on_link: LinkHandler,
        on_toggle: ToggleHandler,
        path: String,
        view: WeakEntity<ReadingView>,
        selection: Option<&'a [(usize, usize, usize, usize)]>,
    ) -> Renderer<'a> {
        Renderer {
            theme,
            toggled,
            property_types,
            properties_host,
            table_host,
            reading_host: None,
            on_link,
            on_toggle,
            video_play: None,
            on_canvas: None,
            video: None,
            text_system,
            available: available.max(100.),
            path,
            view,
            selection,
        }
    }

    /// The reading view's own settings and handlers. Live Preview leaves
    /// them unset, so its widgets keep the one click path they have.
    pub(crate) fn with_reading_host(mut self, host: &'a ReadingHost) -> Renderer<'a> {
        self.reading_host = Some(host);
        self.video_play = Some(host.on_video.clone());
        self.video = Some(host.video.clone());
        self.on_canvas = Some(host.on_canvas.clone());
        self
    }

    /// Width of `text` set in the note's font, in pixels.
    fn measure(&self, text: &str, font_size: f32, bold: bool, mono: bool) -> f32 {
        let t = self.theme;
        let font = Font {
            family: if mono {
                t.mono_font.clone()
            } else {
                t.text_font.clone()
            },
            features: Default::default(),
            fallbacks: None,
            weight: if bold {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            },
            style: FontStyle::Normal,
        };
        let run = TextRun {
            len: text.len(),
            font,
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = self.text_system.shape_line(
            SharedString::from(text.to_string()),
            px(font_size),
            &[run],
            None,
        );
        f32::from(line.width)
    }

    /// Column widths the way an HTML table's automatic layout sizes them: a
    /// column wants its longest cell (max-content) and needs at least its
    /// longest word (min-content); when the note is too narrow for every
    /// column's wish, the slack is shared in proportion to each column's
    /// range, and below the minimum the table overflows.
    /// `editing` is the column a cell is being typed into and the text in
    /// hand, so the column grows with what is typed rather than clipping it.
    fn column_widths(
        &self,
        header: &[RichText],
        rows: &[Vec<RichText>],
        font_size: f32,
        editing: Option<(usize, &str)>,
    ) -> Vec<f32> {
        const PADDING: f32 = 17.;
        let count = header
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0));
        let mut max = vec![0f32; count];
        let mut min = vec![0f32; count];
        let mut fold = |ix: usize, cell: &RichText, bold: bool| {
            let mono = cell.runs.iter().any(|(_, s)| s.code);
            let longest_line = cell
                .text
                .lines()
                .max_by_key(|l| l.chars().count())
                .unwrap_or("");
            let longest_word = cell
                .text
                .split_whitespace()
                .max_by_key(|w| w.chars().count())
                .unwrap_or("");
            max[ix] = max[ix].max(self.measure(longest_line, font_size, bold, mono));
            min[ix] = min[ix].max(self.measure(longest_word, font_size, bold, mono));
        };
        for (ix, cell) in header.iter().enumerate() {
            fold(ix, cell, true);
        }
        for row in rows {
            for (ix, cell) in row.iter().enumerate() {
                fold(ix, cell, false);
            }
        }
        if let Some((ix, text)) = editing.filter(|(ix, _)| *ix < count) {
            let width = self.measure(text, font_size, false, false);
            max[ix] = max[ix].max(width);
            min[ix] = min[ix].max(width);
        }
        // * Two pixels of slack: the wrapper measures advances a little
        //   differently from the shaper and must not break a line that fits.
        let max: Vec<f32> = max.iter().map(|m| m.ceil() + 2. + PADDING).collect();
        let min: Vec<f32> = min
            .iter()
            .map(|m| m.max(8.).ceil() + 2. + PADDING)
            .collect();
        let sum_max: f32 = max.iter().sum();
        let sum_min: f32 = min.iter().sum();
        if sum_max <= self.available {
            max
        } else if sum_min >= self.available {
            min
        } else {
            let share = (self.available - sum_min) / (sum_max - sum_min);
            max.iter()
                .zip(&min)
                .map(|(mx, mn)| (mn + (mx - mn) * share).floor())
                .collect()
        }
    }

    fn blocks(&self, blocks: &[RBlock], tight: bool) -> Vec<AnyElement> {
        blocks.iter().map(|b| self.block(b, tight)).collect()
    }

    pub(crate) fn block(&self, block: &RBlock, tight: bool) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let gap = if tight { px(0.) } else { px(base * 0.9) };
        match block {
            RBlock::Properties(entries) => self.properties(entries),
            RBlock::Paragraph(segments) => div()
                .mb(gap)
                .flex()
                .flex_col()
                .children(segments.iter().map(|s| self.segment(s)))
                .into_any_element(),
            RBlock::Heading { level, text, .. } => {
                // * Obsidian's scale: h1 1.618em at 700, down to h6 1em at
                //   600, with 1.2 line heights at the top; a 40 px gap above
                //   an h2 and 16 px below every heading, the body margin.
                let (size, weight, line_height) = crate::theme::heading_style(*level);
                let top = match level {
                    1 => 1.0,
                    2 => 2.5,
                    _ => 1.5,
                };
                div()
                    .mt(px(base * top))
                    .mb(px(base))
                    .text_size(px(base * size))
                    .line_height(px(base * size * line_height))
                    .font_weight(FontWeight(weight))
                    .child(self.rich_at(text, base * size))
                    .into_any_element()
            }
            RBlock::ThematicBreak => div()
                .my(px(base * 1.5))
                .h(px(1.))
                .w_full()
                .bg(t.border)
                .into_any_element(),
            RBlock::Code { id, text, copy, .. } => {
                let id = *id;
                let payload = copy.clone();
                let on_copy = self.reading_host.map(|h| h.on_copy.clone());
                let copied = self.reading_host.is_some_and(|h| h.copied == Some(id));
                div()
                    .mb(gap)
                    .relative()
                    .group("code-block")
                    .bg(t.code_bg)
                    .rounded_md()
                    .py_2()
                    .font_family(t.mono_font.clone())
                    .text_size(px(base * 0.875))
                    .line_height(px(base * 0.875 * 1.5))
                    .child(
                        // * Code keeps its columns: long lines scroll sideways.
                        div().id(("code", id)).overflow_x_scroll().px_3().child(
                            div()
                                .whitespace_nowrap()
                                .child(SharedString::from(text.clone())),
                        ),
                    )
                    .child(
                        // * Obsidian draws no language name on a rendered
                        //   block; the only thing on it is a copy button,
                        //   and only while the block is hovered.
                        div()
                            .id(("copy-code", id))
                            .absolute()
                            .top_0()
                            .right_0()
                            .m(px(6.))
                            .px_2()
                            .py(px(6.))
                            .rounded_sm()
                            .invisible()
                            .group_hover("code-block", |s| s.visible())
                            .cursor_pointer()
                            .hover(|s| s.bg(t.bg_secondary))
                            // * `lucide-copy`, and `lucide-check` for as long
                            //   as the check shows. A 16 px icon in this box
                            //   is the 31.98 x 27.97 px button Obsidian paints.
                            .child(icon(
                                if copied { "check" } else { "copy" },
                                16.,
                                t.text_muted,
                            ))
                            .on_click(move |_, _, cx| {
                                // * An empty fence's button writes nothing at
                                //   all -- a click on one in Obsidian left the
                                //   clipboard holding what it already had, so
                                //   an empty body is not copied as an empty
                                //   string. Whether its check still flashes is
                                //   unobserved; flashing one for a copy that
                                //   did not happen would be a lie, so it does
                                //   not.
                                if payload.is_empty() {
                                    return;
                                }
                                cx.write_to_clipboard(ClipboardItem::new_string(payload.clone()));
                                if let Some(on_copy) = &on_copy {
                                    on_copy(id, cx);
                                }
                            }),
                    )
                    .into_any_element()
            }
            RBlock::Html { text, kind } => {
                // * Run every registered markdown post-processors so a plugin
                //   can inspect the rendered output. The hook is in place;
                //   the DOM element itself is not yet available to a native
                //   renderer, so the callback receives no element.
                let _ = live::run_all_post_processors();
                // * Source blocks (comments, scripts, unknown tags) keep
                //   their raw text: its `html` run paints it monospace and
                //   muted. `div` is drawn as a real block container; `pre`
                //   keeps its newlines and stays monospace at the code-block
                //   size, the way a `<pre>` block reads.
                let mut block = div().mb(gap);
                if matches!(kind, HtmlBlockKind::Pre) {
                    block = block
                        .font_family(t.mono_font.clone())
                        .text_size(px(base * 0.875));
                }
                block.child(self.rich(text)).into_any_element()
            }
            RBlock::Quote(children) => div()
                .mb(gap)
                .border_l_2()
                .border_color(t.accent)
                .pl_4()
                .flex()
                .flex_col()
                .children(self.blocks(children, false))
                .into_any_element(),
            RBlock::Callout {
                kind,
                fold,
                title,
                children,
            } => {
                let color = t.callout_color(kind);
                let foldable = fold.is_some();
                let flipped = self.toggled.contains(&title.id);
                let collapsed = fold.unwrap_or(false) != flipped;
                let id = title.id;
                let on_toggle = self.on_toggle.clone();
                let mut bg = color;
                bg.a = if t.dark { 0.12 } else { 0.08 };
                div()
                    .mb(gap)
                    .bg(bg)
                    .border_l_4()
                    .border_color(color)
                    .rounded_md()
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(("callout", id))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            // * Mouse-down rather than click, and blocking the
                            //   mouse rather than letting it through: in Live
                            //   Preview the editor's own window listener runs
                            //   ahead of this element and would put the caret
                            //   on the title line, which reveals the source
                            //   and takes the widget — chevron and all — away
                            //   before the button-up that a click needs. Scroll
                            //   still falls through to the editor behind.
                            .when(foldable, |d| {
                                d.cursor_pointer()
                                    .block_mouse_except_scroll()
                                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                        on_toggle(id, cx)
                                    })
                            })
                            // * The type's Lucide icon, in the callout's own
                            //   colour, which is where Obsidian puts it and
                            //   what Obsidian paints it (see [`callout_icon`]).
                            .child(icon(callout_icon(kind), 16., color))
                            .child(
                                div().flex_1().child(
                                    // * Where the title holds a link the link
                                    //   wins: Obsidian navigates and leaves the
                                    //   fold alone, so the text keeps the
                                    //   mouse-down to itself. The chevron and the
                                    //   rest of the row still fold.
                                    match foldable && !title.links.is_empty() {
                                        true => div()
                                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .child(self.rich(title))
                                            .into_any_element(),
                                        false => self.rich(title).into_any_element(),
                                    },
                                ),
                            )
                            .when(foldable, |d| {
                                // * Obsidian's `.callout-fold` holds
                                //   `lucide-chevron-down` collapsed and
                                //   expanded alike, turned a quarter turn when
                                //   collapsed by something that is not a
                                //   transform on that element;
                                //   `chevron-right` is the same drawing
                                //   already turned.
                                d.child(icon(
                                    if collapsed {
                                        "chevron-right"
                                    } else {
                                        "chevron-down"
                                    },
                                    16.,
                                    t.text_muted,
                                ))
                            }),
                    )
                    .when(!collapsed && !children.is_empty(), |d| {
                        d.child(
                            div()
                                .mt_2()
                                .flex()
                                .flex_col()
                                .children(self.blocks(children, false)),
                        )
                    })
                    .into_any_element()
            }
            RBlock::List { tight, items } => div()
                .mb(gap)
                .flex()
                .flex_col()
                .children(items.iter().map(|item| self.item(item, *tight)))
                .into_any_element(),
            RBlock::Table {
                alignments,
                header,
                rows,
                spans,
            } => {
                let font_size = base * 0.9375;
                let host = self.table_host;
                // * The column holding the open cell measures the text in
                //   hand, so it grows as the cell is typed into.
                let editing = host.and_then(|h| h.cell.as_ref()).and_then(|(at, f)| {
                    spans
                        .iter()
                        .find_map(|row| row.iter().position(|s| s == at))
                        .map(|ix| (ix, f.text.as_str()))
                });
                let columns = self.column_widths(header, rows, font_size, editing);
                let table_width: f32 = columns.iter().sum();
                // * `row` is 0 for the header, so the span of every cell is
                //   where the model put it.
                let cell = |text: &RichText, row: usize, ix: usize, head: bool| -> AnyElement {
                    let align = alignments.get(ix).copied().unwrap_or(Alignment::None);
                    let width = columns.get(ix).copied().unwrap_or(80.);
                    let span = spans.get(row).and_then(|r| r.get(ix)).cloned();
                    // * The cell being typed into shows the text in hand with
                    //   a caret, the way a property row's field does.
                    let editing = host
                        .and_then(|h| h.cell.as_ref())
                        .filter(|(at, _)| Some(at) == span.as_ref())
                        .map(|(_, f)| f);
                    let drawn = div()
                        .w(px(width))
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .border_r_1()
                        .border_color(t.border)
                        .when(head, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .map(|d| match align {
                            Alignment::Center => d.text_center(),
                            Alignment::Right => d.text_right(),
                            _ => d,
                        })
                        .map(|d| match editing {
                            Some(field) => d.child(self.property_field(field, false)),
                            None => d.child(self.rich(text)),
                        });
                    // * A cell is clicked into, the way a property row is.
                    match (host, span) {
                        (Some(host), Some(span)) => {
                            let on_action = host.on_action.clone();
                            drawn
                                .id(("table-cell", span.start))
                                .cursor_text()
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    on_action(TableAction::Edit { span: span.clone() }, window, cx);
                                })
                                .into_any_element()
                        }
                        _ => drawn.into_any_element(),
                    }
                };
                // * The table is as wide as its columns, like an HTML table.
                div()
                    .mb(gap)
                    .flex()
                    .flex_row()
                    .child(
                        div()
                            .w(px(table_width))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .border_1()
                            .border_color(t.border)
                            .rounded_sm()
                            .text_size(px(font_size))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .bg(t.bg_secondary)
                                    .border_b_1()
                                    .border_color(t.border)
                                    .children(
                                        header.iter().enumerate().map(|(i, c)| cell(c, 0, i, true)),
                                    ),
                            )
                            .children(rows.iter().enumerate().map(|(ri, row)| {
                                div()
                                    .flex()
                                    .flex_row()
                                    .when(ri + 1 < rows.len(), |d| {
                                        d.border_b_1().border_color(t.border)
                                    })
                                    .children(
                                        row.iter()
                                            .enumerate()
                                            .map(|(i, c)| cell(c, ri + 1, i, false)),
                                    )
                            })),
                    )
                    .into_any_element()
            }
            RBlock::Math(text) => self.math_block(text, gap),
            RBlock::Mermaid { id, text } => self.mermaid_block(*id, text, gap),
            RBlock::Footnotes(notes) => div()
                .mt(px(base * 2.))
                .pt(px(base))
                .border_t_1()
                .border_color(t.border)
                .text_size(px(base * 0.875))
                .flex()
                .flex_col()
                .children(notes.iter().map(|(n, blocks)| {
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(
                            div()
                                .w(px(28.))
                                .flex_shrink_0()
                                .text_color(t.text_muted)
                                .child(SharedString::from(format!("{n}."))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .children(self.blocks(blocks, true)),
                        )
                }))
                .into_any_element(),
        }
    }

    fn item(&self, item: &RItem, tight: bool) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let marker: AnyElement = match (item.task, item.number) {
            (Some(c), _) => {
                let checked = c != ' ';
                let check = div()
                    .w(px(16.))
                    .h(px(16.))
                    .mt(px(base * 0.3))
                    .rounded_sm()
                    .border_1()
                    .border_color(if checked { t.accent } else { t.text_muted })
                    .when(checked, |d| d.bg(t.accent))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .text_color(t.bg)
                    .when(checked, |d| d.child("✓"));
                // * Clicking the box rewrites its line, the way it does in
                //   Live Preview. An item that came out of an embed has no
                //   marker and stays inert: those bytes are another file's.
                match (self.reading_host, item.marker) {
                    (Some(host), Some(marker)) => {
                        let on_task = host.on_task.clone();
                        check
                            .id(("task", marker.line))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| on_task(marker, cx))
                            .into_any_element()
                    }
                    _ => check.into_any_element(),
                }
            }
            (None, Some(n)) => div()
                .text_color(t.text_muted)
                .child(SharedString::from(format!("{n}.")))
                .into_any_element(),
            (None, None) => div().text_color(t.text_muted).child("•").into_any_element(),
        };
        let done = item.task.is_some_and(|c| c != ' ');
        div()
            .flex()
            .flex_row()
            .gap_2()
            .when(!tight, |d| d.mb(px(base * 0.4)))
            .child(
                div()
                    .min_w(px(22.))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .child(marker),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    // * Obsidian's default theme dims a done task and
                    //   strikes it through (`--checklist-done-decoration`).
                    .when(done, |d| d.text_color(t.text_muted).line_through())
                    .children(self.blocks(&item.children, tight)),
            )
            .into_any_element()
    }

    fn segment(&self, segment: &Segment) -> AnyElement {
        match segment {
            Segment::Text(text) => self.rich(text),
            Segment::Embed(embed) => match &embed.kind {
                // * Inline images (local or remote) render as an inline img
                //   so `![[img]]` and `![url]` work mid-sentence, in lists,
                //   blockquotes and callout titles without breaking the line.
                EmbedKind::Image(_) | EmbedKind::RemoteImage(_) => self.inline_embed(embed),
                _ => self.embed(embed),
            },
            // * Display math inside a paragraph is a block of its own; the
            //   paragraph's own gap follows the paragraph, not this.
            Segment::Math(tex) => self.math_block(tex, px(0.)),
        }
    }

    /// `$$...$$`: the formula typeset in display style, centred, at the
    /// body size, and wide formulas scroll sideways rather than wrap, as
    /// MathJax's display container does. A formula that does not parse
    /// shows its source in the error red, the way MathJax's error output
    /// does.
    fn math_block(&self, tex: &str, gap: Pixels) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let tex = tex.trim();
        match math::typeset(tex, true) {
            Ok(formula) => div()
                .id(ElementId::Name(format!("math-block:{tex}").into()))
                .mb(gap)
                .py(px(base * 0.5))
                .w_full()
                .overflow_x_scroll()
                .flex()
                .flex_row()
                .child(div().mx_auto().flex_shrink_0().child(FormulaElement::new(
                    formula,
                    px(base),
                    t.text,
                )))
                .into_any_element(),
            Err(_) => div()
                .mb(gap)
                .py(px(base * 0.5))
                .text_color(math::error_color())
                .text_center()
                .child(SharedString::from(tex.to_string()))
                .into_any_element(),
        }
    }

    fn mermaid_block(&self, id: usize, text: &str, gap: Pixels) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let text = text.trim();
        match crate::mermaid::render_mermaid(text, t.dark) {
            Ok(image) => div()
                .id(ElementId::Name(format!("mermaid-block:{id}").into()))
                .mb(gap)
                .py(px(base * 0.5))
                .w_full()
                .overflow_x_scroll()
                .flex()
                .flex_row()
                .child(
                    div()
                        .mx_auto()
                        .flex_shrink_0()
                        .child(img(image).max_w_full().object_fit(ObjectFit::Contain)),
                )
                .into_any_element(),
            Err(err) => div()
                .id(ElementId::Name(format!("mermaid-err:{id}").into()))
                .mb(gap)
                .p_3()
                .rounded_md()
                .bg(t.code_bg)
                .border_1()
                .border_color(t.border)
                .child(
                    div()
                        .text_size(px(base * 0.875))
                        .text_color(crate::math::error_color())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Mermaid syntax error"),
                )
                .child(
                    div()
                        .my_1()
                        .text_size(px(base * 0.75))
                        .text_color(t.text_muted)
                        .child(SharedString::from(err)),
                )
                .child(
                    div()
                        .mt_2()
                        .p_2()
                        .rounded_sm()
                        .bg(t.bg)
                        .font_family(t.mono_font.clone())
                        .text_size(px(base * 0.75))
                        .text_color(t.text)
                        .child(SharedString::from(text.to_string())),
                )
                .into_any_element(),
        }
    }

    /// The video embed: the frame, a play/pause button, a clock and a
    /// position bar. Pure view — the host's handler plays, so the same
    /// drawing serves the reading view and Live Preview's widgets.
    fn video_embed(
        &self,
        path: &std::path::Path,
        label: &str,
        t: &Theme,
        handler: VideoHandler,
        video: Rc<RefCell<Option<VideoState>>>,
    ) -> AnyElement {
        let playing_state = {
            let video = video.borrow();
            video
                .as_ref()
                .filter(|v| v.path == path)
                .map(|v| (v.playing, v.at, v.info, v.frame.clone()))
        };
        let info = match playing_state.as_ref() {
            Some((_, _, info, _)) => *info,
            None => match crate::video::probe(path) {
                Some(info) => info,
                None => return self.video_card(path, label, t),
            },
        };
        let (playing, at, duration) = match playing_state.as_ref() {
            Some((playing, at, info, _)) => (*playing, *at, info.duration),
            None => (false, 0.0, info.duration),
        };
        let (w, h) = info.display_size(self.available);
        // * The frame: the player's, or the file's first, so the embed is not
        //   a black box before it is ever clicked.
        let frame = match playing_state.and_then(|(_, _, _, frame)| frame) {
            Some(frame) => Some(frame),
            None => crate::video::still(path, &info, 0.0),
        };
        let on_frame = handler.clone();
        let on_toggle = handler.clone();
        let on_toggle_skip = on_toggle.clone();
        let _on_bar = handler;
        let frame_path = path.to_path_buf();
        let toggle_path = path.to_path_buf();
        let toggle_path_skip = toggle_path.clone();
        div()
            .my_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(t.border)
                    .bg(gpui::black())
                    .id(("video-frame", label.len()))
                    .on_click(move |_, window, cx| on_frame(&frame_path, None, window, cx))
                    .children(frame.map(|f| {
                        img(crate::video::frame_source(f))
                            .size_full()
                            .object_fit(ObjectFit::Fill)
                    }))
                    .when(!playing, |el| {
                        el.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .size(px(52.))
                                        .rounded_full()
                                        .bg(gpui::rgba(0x000000b0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_color(gpui::white())
                                        .text_size(px(22.))
                                        .child(SharedString::from("▶")),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(w))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(
                        div()
                            .w(px(14.))
                            .cursor_pointer()
                            .id(("video-toggle", label.len()))
                            .on_click(move |_, window, cx| {
                                on_toggle(&toggle_path, None, window, cx)
                            })
                            .child(SharedString::from(if playing { "⏸" } else { "▶" })),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .cursor_pointer()
                            .id(("video-skip-back", label.len()))
                            .on_click({
                                let on_toggle = on_toggle_skip.clone();
                                let toggle_path = toggle_path_skip.clone();
                                move |_, window, cx| {
                                    let seek = (at - 10.0).max(0.0);
                                    on_toggle(&toggle_path, Some(seek / duration), window, cx)
                                }
                            })
                            .child(SharedString::from("⏮")),
                    )
                    .child(SharedString::from(clock(at)))
                    .child(
                        // * The position bar: the share played so far.
                        div()
                            .flex_1()
                            .h(px(4.))
                            .rounded_full()
                            .bg(t.border)
                            .relative()
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .h(px(4.))
                                    .rounded_full()
                                    .bg(t.accent)
                                    .w(gpui::relative(if duration > 0.0 {
                                        (at / duration).clamp(0.0, 1.0)
                                    } else {
                                        0.0
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .cursor_pointer()
                            .id(("video-skip-forward", label.len()))
                            .on_click({
                                let on_toggle = on_toggle_skip.clone();
                                let toggle_path = toggle_path_skip.clone();
                                move |_, window, cx| {
                                    let seek = (at + 10.0).min(duration);
                                    on_toggle(&toggle_path, Some(seek / duration), window, cx)
                                }
                            })
                            .child(SharedString::from("⏭")),
                    )
                    .child(SharedString::from(clock(duration))),
            )
            .into_any_element()
    }

    /// A video nothing here can play: the card that opens the system player,
    /// which is what every video embed did before the in-app one.
    fn video_card(&self, path: &std::path::Path, label: &str, t: &Theme) -> AnyElement {
        let target = format!("file://{}", path.display());
        div()
            .my_1()
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .px_4()
            .py_3()
            .cursor_pointer()
            .hover(|s| s.bg(t.bg_alt))
            .id(("video-play", label.len()))
            .on_click(move |_, _, cx| {
                cx.open_url(&target);
            })
            .child(SharedString::from("▶ "))
            .child(SharedString::from(label.to_string()))
            .into_any_element()
    }

    /// Renders an image embed inline (mid-sentence, in lists, blockquotes,
    /// callout titles) without a block wrapper, so text flows around it.
    fn inline_embed(&self, embed: &Embed) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let image = match &embed.kind {
            EmbedKind::Image(path) => {
                let lost = path.clone();
                img(path.clone())
                    .max_w_full()
                    .object_fit(ObjectFit::Contain)
                    // * A file the vault holds no image for paints nothing at
                    //   all: the fallback names it instead.
                    .with_fallback(move || missing_image(&lost))
            }
            EmbedKind::RemoteImage(url) => {
                let err_url = url.clone();
                let bg_sec = t.bg_secondary;
                let fg_muted = t.text_muted;
                img(url.clone())
                    .max_w_full()
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || {
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(bg_sec)
                            .text_color(fg_muted)
                            .text_size(px(base * 0.85))
                            .child(format!("Image failed to load: {}", err_url))
                            .into_any_element()
                    })
            }
            _ => unreachable!("inline_embed only called for image kinds"),
        };
        let mut image = image;
        if let Some(w) = embed.width {
            image = image.w(px(w));
        }
        if let Some(h) = embed.height {
            image = image.h(px(h));
        }
        image.into_any_element()
    }

    fn embed(&self, embed: &Embed) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        match &embed.kind {
            EmbedKind::Image(path) => {
                let lost = path.clone();
                let mut image = img(path.clone())
                    .max_w_full()
                    .object_fit(ObjectFit::Contain)
                    // * The same fallback the inline image carries: a file the
                    //   vault holds no image for names itself instead of
                    //   painting nothing.
                    .with_fallback(move || missing_image(&lost));
                if let Some(w) = embed.width {
                    image = image.w(px(w));
                }
                if let Some(h) = embed.height {
                    image = image.h(px(h));
                }
                div().my_1().child(image).into_any_element()
            }
            EmbedKind::RemoteImage(url) => {
                let err_url = url.clone();
                let bg_sec = t.bg_secondary;
                let fg_muted = t.text_muted;
                let mut image = img(url.clone())
                    .max_w_full()
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || {
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(bg_sec)
                            .text_color(fg_muted)
                            .text_size(px(base * 0.85))
                            .child(format!("Image failed to load: {}", err_url))
                            .into_any_element()
                    });
                if let Some(w) = embed.width {
                    image = image.w(px(w));
                }
                if let Some(h) = embed.height {
                    image = image.h(px(h));
                }
                div().my_1().child(image).into_any_element()
            }
            EmbedKind::Note {
                path,
                title,
                blocks,
            } => {
                let on_link = self.on_link.clone();
                let link = embed.link.clone();
                div()
                    .my_2()
                    .border_1()
                    .border_color(t.border)
                    .rounded_md()
                    .px_4()
                    .py_3()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(("embed-title", path.len() + title.len()))
                            .mb_2()
                            .text_size(px(base * 0.75))
                            .text_color(t.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.text_color(t.link))
                            .on_click(move |_, window, cx| {
                                on_link(LinkTarget::Internal(link.clone()), window, cx)
                            })
                            .child(SharedString::from(format!("↗ {title}"))),
                    )
                    .children(self.blocks(blocks, false))
                    .into_any_element()
            }
            // * `![[x.base]]`: the base's own drawing is `bases::embed`, which
            //   takes the view resolved by the same engine the open view uses.
            //   Every click inside it is an internal link by vault path — the
            //   title opens the `.base`, a row opens the file that row came
            //   from — which is how every other embed opens its target.
            EmbedKind::Base(base) => {
                let on_link = self.on_link.clone();
                let open: crate::bases::embed::OpenPath =
                    Rc::new(move |path: &str, window: &mut Window, cx: &mut App| {
                        on_link(LinkTarget::Internal(path.to_string()), window, cx)
                    });
                crate::bases::embed::render(base, t, open)
            }
            // * `![[x.pdf]]`: the pages are drawn by `pdf::embed`, which asks
            //   the document cache for them when it is painted; the model
            //   carried only the path and the subpath. A click opens the file
            //   by vault path, as every other embed's target is opened.
            EmbedKind::Pdf(pdf) => {
                let on_link = self.on_link.clone();
                let open: crate::pdf::embed::OpenPath =
                    Rc::new(move |path: &str, window: &mut Window, cx: &mut App| {
                        on_link(LinkTarget::Internal(path.to_string()), window, cx)
                    });
                crate::pdf::embed::render(pdf, t, self.available, open)
            }
            // * `![[x.mp3]]` local audio plays in Silica's own player (rodio).
            //   Local video and remote media open in the system's default player.
            //   `![[x.mp4]]` plays in Silica's own player: ffmpeg decodes,
            //   gpui draws. With no handler (Live Preview's widget) or no
            //   ffmpeg on the machine, the card opens the system player.
            EmbedKind::Canvas(path) => {
                let vault_path = path.to_string_lossy().into_owned();
                let width = embed.width.unwrap_or(0.0);
                let height = embed.height.unwrap_or(0.0);
                match self.on_canvas.as_ref() {
                    Some(host) => host(&vault_path, width, height),
                    None => missing_canvas(&vault_path, width, height),
                }
            }
            EmbedKind::Video(path) => {
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&embed.link)
                    .to_string();
                match (&self.video_play, &self.video) {
                    (Some(handler), Some(video)) => {
                        self.video_embed(path, &label, t, handler.clone(), video.clone())
                    }
                    _ => self.video_card(path, &label, t),
                }
            }
            EmbedKind::Audio(path) => {
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&embed.link)
                    .to_string();
                let is_playing = AUDIO_PLAYER
                    .with(|p| p.borrow().as_ref().is_some_and(|player| player.is_playing));
                let path_clone = path.clone();
                div()
                    .my_1()
                    .border_1()
                    .border_color(t.border)
                    .rounded_md()
                    .px_4()
                    .py_3()
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bg_alt))
                    .id(("audio-player", embed.link.len()))
                    .on_click(move |_, _, _cx| {
                        AUDIO_PLAYER.with(|slot| {
                            let mut slot = slot.borrow_mut();
                            if let Some(player) = slot.as_mut() {
                                if player.is_playing {
                                    player.sink.pause();
                                    player.is_playing = false;
                                } else {
                                    player.sink.play();
                                    player.is_playing = true;
                                }
                                return;
                            }
                            let Ok((stream, handle)) = rodio::OutputStream::try_default() else {
                                return;
                            };
                            let Ok(file) = std::fs::File::open(&path_clone) else {
                                return;
                            };
                            let Ok(source) = rodio::Decoder::new(file) else {
                                return;
                            };
                            let Ok(sink) = rodio::Sink::try_new(&handle) else {
                                return;
                            };
                            sink.append(source);
                            sink.play();
                            *slot = Some(AudioPlayer {
                                _stream: stream,
                                sink,
                                is_playing: true,
                            });
                        });
                    })
                    .child(SharedString::from(if is_playing { "⏸ " } else { "▶ " }))
                    .child(SharedString::from(label))
                    .into_any_element()
            }
            EmbedKind::RemoteAudio(url) | EmbedKind::RemoteVideo(url) => {
                let label = url.rsplit('/').next().unwrap_or(url).to_string();
                let url = url.clone();
                div()
                    .my_1()
                    .border_1()
                    .border_color(t.border)
                    .rounded_md()
                    .px_4()
                    .py_3()
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bg_alt))
                    .id(("media-play-remote", url.len()))
                    .on_click(move |_, _, cx| {
                        cx.open_url(&url);
                    })
                    .child(SharedString::from("▶ "))
                    .child(SharedString::from(label))
                    .into_any_element()
            }
            // * A missing attachment is Obsidian's `file-embed
            //   mod-empty-attachment` box (observed on `![[missing.pdf]]`,
            //   `tools/obsidian-probe/results/2026-10-26-pdf.json` s08): a
            //   block padded 8 px with an 8 px radius on the alternate
            //   ground, 14 px muted text saying `“missing.pdf” could not be
            //   found.` A missing note keeps the shell's own line.
            EmbedKind::Missing
                if FileKind::from_path(silica_markdown::parse_linktext(&embed.link).path)
                    != FileKind::Markdown =>
            {
                div()
                    .my_1()
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(t.bg_alt)
                    .text_size(px(14.))
                    .text_color(t.text_muted)
                    .child(SharedString::from(format!(
                        "\u{201c}{}\u{201d} could not be found.",
                        embed.link
                    )))
                    .into_any_element()
            }
            EmbedKind::Missing => div()
                .my_1()
                .text_color(t.text_muted)
                .italic()
                .child(SharedString::from(format!("{}: file not found", embed.alt)))
                .into_any_element(),
            EmbedKind::Unsupported(kind) => {
                let on_link = self.on_link.clone();
                let link = embed.link.clone();
                div()
                    .id(("embed-file", embed.link.len()))
                    .my_1()
                    .text_color(t.link)
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        on_link(LinkTarget::Internal(link.clone()), window, cx)
                    })
                    .child(SharedString::from(format!("{} ({kind:?})", embed.alt)))
                    .into_any_element()
            }
            EmbedKind::TooDeep => div()
                .my_1()
                .text_color(t.text_muted)
                .italic()
                .child(SharedString::from(format!(
                    "{}: embed nested too deep",
                    embed.alt
                )))
                .into_any_element(),
        }
    }

    /// The properties block: a foldable "Properties" heading and one row per
    /// property, each with its type glyph, its key and its value. Links and
    /// tags inside a value are clickable; nothing here is editable, which is
    /// the one way the reading view differs from Obsidian's (it edits
    /// properties in reading view too).
    fn properties(&self, entries: &[Property]) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let host = self.properties_host;
        let hidden = host.is_some_and(|h| h.mode == PropertiesInDocument::Hidden)
            || self
                .reading_host
                .is_some_and(|h| h.properties == PropertiesInDocument::Hidden);
        if hidden {
            // * "Hide" in `propertiesInDocument`: the YAML stays in the file,
            //   and neither the editor nor the rendered text shows it.
            return div().into_any_element();
        }
        let folded = self.toggled.contains(&PROPERTIES_FOLD_ID);
        let mut next_id = PROPERTY_ID_BASE;
        let rows = crate::properties::rows(
            entries,
            |key| silica_formats::properties::type_of(key, entries, self.property_types),
            &mut next_id,
        );
        let on_toggle = self.on_toggle.clone();
        let add = host.map(|h| h.on_action.clone());
        div()
            .mb(px(base * 1.2))
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(t.border)
            .pb(px(base * 0.6))
            .text_size(px(base * 0.875))
            .child(
                div()
                    .id("properties-heading")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .py_1()
                    .cursor_pointer()
                    .text_color(t.text_muted)
                    .child(SharedString::from(if folded {
                        "\u{25b8}"
                    } else {
                        "\u{25be}"
                    }))
                    .child(SharedString::from("Properties"))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        on_toggle(PROPERTIES_FOLD_ID, cx);
                    }),
            )
            .when(!folded, |d| {
                d.children(rows.into_iter().map(|row| self.property_row(row)))
            })
            .when_some(
                host.and_then(|h| h.field.clone())
                    .filter(|f| f.target == Target::New && !folded),
                |d, field| {
                    // * The row "Add file property" opened: a key field with
                    //   nothing written to the note behind it yet.
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .py_1()
                            .child(
                                div()
                                    .w(px(base))
                                    .flex_shrink_0()
                                    .text_color(t.text_faint)
                                    .child(SharedString::from("T")),
                            )
                            .child(self.property_field(&field, false)),
                    )
                },
            )
            .when_some(
                add.filter(|_| {
                    !folded
                        && !host.is_some_and(|h| {
                            h.field.as_ref().is_some_and(|f| f.target == Target::New)
                        })
                }),
                |d, on_action| {
                    // * The row Obsidian's "Add file property" opens. Nothing is
                    //   written until it is named.
                    d.child(
                        div()
                            .id("properties-add")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .py_1()
                            .cursor_pointer()
                            .text_color(t.text_faint)
                            .child(SharedString::from("+"))
                            .child(SharedString::from("Add property"))
                            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                on_action(PropertyAction::AddRow, window, cx);
                            }),
                    )
                },
            )
            .into_any_element()
    }

    /// The caret-bearing field a row shows while it is being typed into.
    fn property_field(&self, field: &Field, wide: bool) -> AnyElement {
        let t = self.theme;
        let before = &field.text[..field.cursor];
        let after = &field.text[field.cursor..];
        div()
            .flex()
            .flex_row()
            .items_center()
            .when(wide, |d| d.flex_1())
            .min_w_0()
            .px_1()
            .rounded_sm()
            .bg(t.bg_active)
            .child(SharedString::from(before.to_string()))
            .child(
                div()
                    .w(px(1.))
                    .h(px(t.base_font_size))
                    .bg(t.accent)
                    .flex_shrink_0(),
            )
            .child(SharedString::from(after.to_string()))
            .into_any_element()
    }

    fn property_row(&self, row: crate::properties::Row) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let host = self.properties_host;
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap_2()
            .py_1()
            .child(
                // * The type glyph and the key share the left column, as the
                //   properties UI lays them out.
                div()
                    .w(px(160.))
                    .flex_shrink_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .text_color(t.text_muted)
                    .child({
                        let glyph = div()
                            .w(px(base))
                            .flex_shrink_0()
                            .text_color(t.text_faint)
                            .child(SharedString::from(crate::properties::type_glyph(row.ty)));
                        match host.map(|h| h.on_action.clone()) {
                            Some(on_action) => {
                                let key = row.key.clone();
                                glyph
                                    .id(("property-type", row.id))
                                    .cursor_pointer()
                                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                        on_action(
                                            PropertyAction::OpenTypeMenu { key: key.clone() },
                                            window,
                                            cx,
                                        );
                                    })
                                    .into_any_element()
                            }
                            None => glyph.into_any_element(),
                        }
                    })
                    .child(match self.open_field(&row, Part::Key) {
                        Some(field) => self.property_field(field, true),
                        None => {
                            let cell = div()
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .overflow_hidden()
                                .child(SharedString::from(row.key.clone()));
                            match host.map(|h| h.on_action.clone()) {
                                Some(on_action) => {
                                    let key = row.key.clone();
                                    cell.id(("property-key", row.id))
                                        .cursor_pointer()
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            move |_, window, cx| {
                                                on_action(
                                                    PropertyAction::Edit {
                                                        target: Target::Existing(key.clone()),
                                                        part: Part::Key,
                                                    },
                                                    window,
                                                    cx,
                                                );
                                            },
                                        )
                                        .into_any_element()
                                }
                                None => cell.into_any_element(),
                            }
                        }
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(match self.open_field(&row, Part::Value) {
                        Some(field) => self.property_field(field, true),
                        None => self.property_value_cell(&row, host),
                    })
                    .when(row.mismatched, |d| {
                        // * Obsidian marks a value that does not fit its type
                        //   and keeps the text as written.
                        d.child(
                            div()
                                .flex_shrink_0()
                                .text_color(t.text_faint)
                                .child(SharedString::from("\u{26a0}")),
                        )
                    })
                    .when_some(host.map(|h| h.on_action.clone()), |d, on_action| {
                        let key = row.key.clone();
                        d.child(
                            div()
                                .id(("property-remove", row.id))
                                .flex_shrink_0()
                                .px_1()
                                .cursor_pointer()
                                .text_color(t.text_faint)
                                .child(SharedString::from("\u{00d7}"))
                                .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                    on_action(
                                        PropertyAction::Remove { key: key.clone() },
                                        window,
                                        cx,
                                    );
                                }),
                        )
                    }),
            )
            .when_some(
                host.filter(|h| h.menu.as_deref() == Some(row.key.as_str())),
                |d, h| d.child(self.property_type_menu(&row, h)),
            )
            .into_any_element()
    }

    /// The type menu Obsidian opens from a row's icon. `tags` and `aliases`
    /// are reserved for their keys and are not offered.
    fn property_type_menu(
        &self,
        row: &crate::properties::Row,
        host: &PropertiesHost,
    ) -> AnyElement {
        let t = self.theme;
        div()
            .ml(px(24.))
            .mb_1()
            .flex()
            .flex_col()
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .bg(t.bg_secondary)
            .py_1()
            .children(PropertyType::selectable().iter().copied().map(|ty| {
                let on_action = host.on_action.clone();
                let key = row.key.clone();
                div()
                    .id(("property-type-option", row.id * 16 + ty as usize))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_0p5()
                    .cursor_pointer()
                    .when(ty == row.ty, |d| d.text_color(t.accent))
                    .hover(|d| d.bg(t.bg_active))
                    .child(SharedString::from(crate::properties::type_glyph(ty)))
                    .child(SharedString::from(ty.label()))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                        on_action(
                            PropertyAction::Retype {
                                key: key.clone(),
                                ty,
                            },
                            window,
                            cx,
                        );
                    })
            }))
            .into_any_element()
    }

    /// The open field when it belongs to this row and part. A field holding
    /// one item of a list is not one of these: the value cell draws it among
    /// the pills, in the place of the item it stands for.
    fn open_field(&self, row: &crate::properties::Row, part: Part) -> Option<&Field> {
        let field = self.properties_host?.field.as_ref()?;
        (field.part == part
            && field.key() == Some(row.key.as_str())
            && field.item_index().is_none())
        .then_some(field)
    }

    /// Which item of this row's list the open field holds, if it holds one.
    /// An index past the last item is the empty input Obsidian keeps after
    /// the pills, where a new item is typed.
    fn open_item(&self, row: &crate::properties::Row) -> Option<(usize, &Field)> {
        let field = self.properties_host?.field.as_ref()?;
        Some((field.item_of(row)?, field))
    }

    /// The value cell when no field is open: read-only in the reading view,
    /// clickable in Live Preview (a checkbox flips, anything else opens a
    /// field).
    fn property_value_cell(
        &self,
        row: &crate::properties::Row,
        host: Option<&PropertiesHost>,
    ) -> AnyElement {
        let Some(on_action) = host.map(|h| h.on_action.clone()) else {
            return self.property_value(row);
        };
        if let crate::properties::Display::Checkbox(checked) = row.value {
            let key = row.key.clone();
            let path = self.path.clone();
            let view = self.view.clone();
            return div()
                .id(("property-checkbox", row.id))
                .flex_1()
                .min_w_0()
                .cursor_pointer()
                .child(self.property_value(row))
                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                    let path = path.clone();
                    let key = key.clone();
                    let _ = view.update(cx, move |_, cx| {
                        cx.emit(ReadingEvent::PropertyChanged {
                            path,
                            changes: vec![Change::Set {
                                key,
                                value: silica_formats::properties::Value::Checkbox(!checked),
                            }],
                        });
                    });
                })
                .into_any_element();
        }
        let key = row.key.clone();
        // * A list value is never opened whole: a click in the cell lands in
        //   the empty input after the pills, so what is typed becomes one
        //   more item however many commas it holds. Only a scalar opens on
        //   its whole text.
        let append = row.is_list_valued().then_some(row.items.len());
        div()
            .id(("property-value", row.id))
            .flex_1()
            .min_w_0()
            .cursor_text()
            .child(self.property_value(row))
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                let action = match append {
                    Some(index) => PropertyAction::EditItem {
                        key: key.clone(),
                        index,
                    },
                    None => PropertyAction::Edit {
                        target: Target::Existing(key.clone()),
                        part: Part::Value,
                    },
                };
                on_action(action, window, cx);
            })
            .into_any_element()
    }

    fn property_value(&self, row: &crate::properties::Row) -> AnyElement {
        let t = self.theme;
        let open = self.open_item(row);
        match &row.value {
            crate::properties::Display::Scalar(text) => div()
                .flex_1()
                .min_w_0()
                .child(self.rich(text))
                .into_any_element(),
            crate::properties::Display::Pills(items) if !row.is_blank() => {
                self.property_pills(row, items, open)
            }
            // * A value with no items still takes a new one, and then the
            //   field for it is the only thing the cell has to draw.
            crate::properties::Display::Empty | crate::properties::Display::Pills(_)
                if open.is_some() =>
            {
                self.property_pills(row, &[], open)
            }
            // * The box keeps its own size; the cell around it takes the width,
            //   so the row's affordances still sit at the right edge.
            crate::properties::Display::Checkbox(checked) => div()
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .w(px(t.base_font_size))
                        .h(px(t.base_font_size))
                        .border_1()
                        .border_color(t.border)
                        .rounded_sm()
                        .when(*checked, |d| {
                            d.bg(t.accent)
                                .text_color(t.bg)
                                .child(SharedString::from("\u{2713}"))
                        }),
                )
                .into_any_element(),
            // * `aliases: []` lands here along with a key that has no value:
            //   both are blank, and a blank cell that drew nothing would have
            //   no height and so take no click at all.
            crate::properties::Display::Empty | crate::properties::Display::Pills(_) => div()
                .text_color(t.text_faint)
                .child(SharedString::from("Empty"))
                .into_any_element(),
        }
    }

    /// A list value: one pill per item, with the open field standing in for
    /// the item it holds, so editing one item never touches the others. A
    /// field past the last item draws last, as the empty input Obsidian
    /// keeps after the pills.
    ///
    /// In Live Preview each pill is its own affordance — clicking it opens
    /// that item alone, and its remove button drops that item — which is
    /// the markup a real Obsidian was observed to draw
    /// (`div.multi-select-pill` with its own
    /// `div.multi-select-pill-remove-button` holding a `lucide-x`). The
    /// reading view passes no host and draws the same pills read-only,
    /// without that button: Obsidian draws one there as well — its
    /// reading view edits properties too, which is the gap
    /// [`Renderer::properties`] records — and in Silica nothing in the
    /// reading view can write the note, so the affordance waits on that
    /// path rather than being drawn dead.
    fn property_pills(
        &self,
        row: &crate::properties::Row,
        items: &[RichText],
        open: Option<(usize, &Field)>,
    ) -> AnyElement {
        let t = self.theme;
        let on_action = self.properties_host.map(|h| h.on_action.clone());
        // * The pill a Backspace selected, drawn focused until the second
        //   press removes it. It belongs to the open field's row, so it only
        //   means anything while a field of this row is open.
        let selected = open.and_then(|_| self.properties_host.and_then(|h| h.selected_pill));
        // * A click inside the open field is not a click on the cell behind
        //   it, which would commit this item and open a fresh one.
        let item_field = |field: &Field| {
            div()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(self.property_field(field, false))
                .into_any_element()
        };
        let mut cells: Vec<AnyElement> = Vec::new();
        for (ix, item) in items.iter().enumerate() {
            if let Some((at, field)) = open {
                if at == ix {
                    cells.push(item_field(field));
                    continue;
                }
            }
            cells.push(
                div()
                    .id(("property-pill", item.id))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_1p5()
                    .rounded_sm()
                    .bg(t.code_bg)
                    // * The border is the pill's own colour until the pill is
                    //   the selected one, so taking focus moves nothing.
                    .border_1()
                    .border_color(if selected == Some(ix) {
                        t.border_focus
                    } else {
                        t.code_bg
                    })
                    .child(self.rich(item))
                    .when_some(on_action.clone(), |d, on_action| {
                        let key = row.key.clone();
                        let remove = on_action.clone();
                        let remove_key = row.key.clone();
                        d.cursor_text()
                            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                // * The cell behind opens the append input;
                                //   this pill opens itself instead.
                                cx.stop_propagation();
                                on_action(
                                    PropertyAction::EditItem {
                                        key: key.clone(),
                                        index: ix,
                                    },
                                    window,
                                    cx,
                                );
                            })
                            .child(
                                div()
                                    .id(("property-pill-remove", item.id))
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    // * Obsidian's `.multi-select-pill-remove-button`
                                    //   holds a `lucide-x`, 14 px wide, and it
                                    //   is always drawn rather than revealed on
                                    //   hover.
                                    .child(crate::assets::icon(
                                        "x",
                                        t.base_font_size * 0.875,
                                        t.text_faint,
                                    ))
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        move |_, window, cx| {
                                            cx.stop_propagation();
                                            remove(
                                                PropertyAction::RemoveItem {
                                                    key: remove_key.clone(),
                                                    index: ix,
                                                },
                                                window,
                                                cx,
                                            );
                                        },
                                    ),
                            )
                    })
                    .into_any_element(),
            );
        }
        if let Some((_, field)) = open.filter(|(at, _)| *at >= items.len()) {
            // * A selected pill has the focus, so the input does not: it is
            //   empty by then (the gesture only starts from an empty one), so
            //   dropping its caret leaves nothing of it to draw.
            if selected.is_none() {
                cells.push(item_field(field));
            }
        }
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_1()
            .children(cells)
            .into_any_element()
    }

    /// Inline content at the body size.
    fn rich(&self, text: &RichText) -> AnyElement {
        self.rich_at(text, self.theme.base_font_size)
    }

    /// Filter `InlineStyle` runs to those falling within `[start, end)`.
    fn filter_runs(
        runs: &[(Range<usize>, InlineStyle)],
        start: usize,
        end: usize,
    ) -> Vec<(Range<usize>, InlineStyle)> {
        runs.iter()
            .filter_map(|(r, s)| {
                if r.end <= start || r.start >= end {
                    None
                } else {
                    Some((r.start.max(start).min(end)..r.end.min(end), *s))
                }
            })
            .collect()
    }

    /// Filter `LinkTarget` ranges to those falling within `[start, end)`.
    fn filter_links(
        links: &[(Range<usize>, LinkTarget)],
        start: usize,
        end: usize,
    ) -> Vec<(Range<usize>, LinkTarget)> {
        links
            .iter()
            .filter_map(|(r, l)| {
                if r.end <= start || r.start >= end {
                    None
                } else {
                    Some((r.start.max(start).min(end)..r.end.min(end), l.clone()))
                }
            })
            .collect()
    }

    /// Inline content set at `size` pixels: one `InteractiveText`, with
    /// inline formulas typeset over placeholders when the text has any.
    pub(crate) fn rich_at(&self, text: &RichText, size: f32) -> AnyElement {
        // * Each `$x$` run, typeset in text style. A formula that does not
        //   parse stays as its source, in the error red (`highlight`).
        let formulas: Vec<(Range<usize>, Arc<silica_math::Formula>)> = text
            .runs
            .iter()
            .filter(|(_, s)| s.math)
            .filter_map(|(r, _)| {
                let tex = text.text.get(r.clone())?;
                math::typeset(tex, false).ok().map(|f| (r.clone(), f))
            })
            .collect();
        // * `<sup>` and `<sub>` runs: raised/lowered above or below the
        //   baseline with reduced size. GPUI's `HighlightStyle` has no
        //   vertical-offset attribute, so these are rendered as separate
        //   positioned elements interleaved with the surrounding text.
        let vertical: Vec<(Range<usize>, bool)> = text
            .runs
            .iter()
            .filter_map(|(r, s)| {
                if s.sup {
                    Some((r.clone(), true))
                } else if s.sub {
                    Some((r.clone(), false))
                } else {
                    None
                }
            })
            .collect();
        if formulas.is_empty() && vertical.is_empty() {
            return self
                .interactive(text, &text.text, &text.runs, &text.links)
                .into_any_element();
        }
        // * Build a list of segments alternating between regular text and
        //   sup/sub text. Each regular segment is an `InteractiveText`;
        //   each sup/sub segment is a positioned `Div` with reduced size
        //   and vertical offset.
        let mut segments: Vec<AnyElement> = Vec::new();
        let mut at = 0;
        let mut all_special: Vec<(Range<usize>, bool, f32)> = Vec::new();
        for (range, _formula) in &formulas {
            all_special.push((range.clone(), false, 0.0));
        }
        for (range, is_sup) in &vertical {
            all_special.push((range.clone(), *is_sup, if *is_sup { -4.0 } else { 2.0 }));
        }
        all_special.sort_by_key(|(r, _, _)| r.start);
        for (range, _is_sup, offset) in &all_special {
            if at < range.start {
                segments.push(
                    self.interactive(
                        text,
                        &text.text[at..range.start],
                        &Self::filter_runs(&text.runs, at, range.start),
                        &Self::filter_links(&text.links, at, range.start),
                    )
                    .into_any_element(),
                );
            }
            let segment_text = &text.text[range.clone()];
            let sup_sub_size = px(size * 0.6);
            segments.push(
                div()
                    .text_size(sup_sub_size)
                    .mt(px(*offset))
                    .child(SharedString::from(segment_text))
                    .into_any_element(),
            );
            at = range.end;
        }
        if at < text.text.len() {
            segments.push(
                self.interactive(
                    text,
                    &text.text[at..],
                    &Self::filter_runs(&text.runs, at, text.text.len()),
                    &Self::filter_links(&text.links, at, text.text.len()),
                )
                .into_any_element(),
            );
        }
        let result = div()
            .flex()
            .flex_row()
            .items_start()
            .gap_0()
            .children(segments)
            .into_any_element();
        if formulas.is_empty() {
            return result;
        }
        // * Formulas still need the Overlay treatment on top of the
        //   segmented text.
        let t = self.theme;
        let em = px(size);
        let font = Font {
            family: t.text_font.clone(),
            features: Default::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let spaces = SpaceWidths::measure(&self.text_system, &font, em);
        let mut edits: Vec<(Range<usize>, usize)> = Vec::new();
        let mut shaped = String::with_capacity(text.text.len());
        let mut inlines = Vec::new();
        let mut at = 0;
        for (range, formula) in &formulas {
            shaped.push_str(&text.text[at..range.start]);
            let (placeholder, slot) = spaces.placeholder(math::width_px(formula, em));
            let start = shaped.len();
            shaped.push_str(&placeholder);
            inlines.push(Inline {
                range: start..shaped.len(),
                formula: formula.clone(),
                slot,
            });
            edits.push((range.clone(), placeholder.len()));
            at = range.end;
        }
        shaped.push_str(&text.text[at..]);
        let remap = |offset: usize| -> usize {
            let mut shift = 0isize;
            for (range, len) in &edits {
                if offset <= range.start {
                    break;
                }
                if offset < range.end {
                    return (range.start as isize + shift) as usize;
                }
                shift += *len as isize - range.len() as isize;
            }
            (offset as isize + shift) as usize
        };
        let runs: Vec<(Range<usize>, InlineStyle)> = text
            .runs
            .iter()
            .filter(|(r, s)| !(s.math && formulas.iter().any(|(f, _)| f == r)))
            .map(|(r, s)| (remap(r.start)..remap(r.end), *s))
            .collect();
        let links: Vec<(Range<usize>, LinkTarget)> = text
            .links
            .iter()
            .map(|(r, l)| (remap(r.start)..remap(r.end), l.clone()))
            .collect();
        let (interactive, layout) = self.interactive_with_layout(text, &shaped, &runs, &links);
        Overlay::new(interactive.into_any_element(), layout, inlines, em, t.text).into_any_element()
    }

    fn interactive(
        &self,
        text: &RichText,
        string: &str,
        runs: &[(Range<usize>, InlineStyle)],
        links: &[(Range<usize>, LinkTarget)],
    ) -> InteractiveText {
        self.interactive_with_layout(text, string, runs, links).0
    }

    /// The `InteractiveText` of `string` styled by `runs`, and its layout
    /// handle, which reports where each byte landed once the text has been
    /// laid out.
    fn interactive_with_layout(
        &self,
        text: &RichText,
        string: &str,
        runs: &[(Range<usize>, InlineStyle)],
        links: &[(Range<usize>, LinkTarget)],
    ) -> (InteractiveText, gpui::TextLayout) {
        let t = self.theme;
        let mut highlights: Vec<_> = runs
            .iter()
            .map(|(range, style)| (range.clone(), self.highlight(*style)))
            .collect();
        // * Selection highlight: this RichText's id is unique in the model, so
        //   matching it against the selection finds the selected byte range.
        if let Some((start, end)) = self.selection.and_then(|ranges| {
            ranges
                .iter()
                .find(|(_, rid, _, _)| *rid == text.id)
                .map(|(_, _, s, e)| (*s, *e))
        }) {
            highlights.push((start..end, self.selection_highlight()));
        }
        let mono: Vec<_> = runs
            .iter()
            .filter(|(_, s)| s.code || s.html || s.kbd)
            .map(|(range, _)| (range.clone(), t.mono_font.clone()))
            .collect();
        let mut styled =
            StyledText::new(SharedString::from(string.to_string())).with_highlights(highlights);
        if !mono.is_empty() {
            styled = styled.with_font_family_overrides(mono);
        }
        let layout = styled.layout().clone();
        let ranges: Vec<_> = links.iter().map(|(r, _)| r.clone()).collect();
        let targets: Vec<LinkTarget> = links.iter().map(|(_, l)| l.clone()).collect();
        let on_link = self.on_link.clone();
        let mut interactive = InteractiveText::new(("rich", text.id), styled).on_click(
            ranges,
            move |ix, window, cx| {
                if let Some(target) = targets.get(ix).cloned() {
                    on_link(target, window, cx);
                }
            },
        );
        if let Some(preview_fn) = self.reading_host.and_then(|h| h.preview_resolver.clone()) {
            let targets: Vec<LinkTarget> = links.iter().map(|(_, l)| l.clone()).collect();
            interactive = interactive.tooltip(move |ix, window, cx| {
                if window.modifiers().control || window.modifiers().platform {
                    if let Some(target) = targets.get(ix) {
                        return preview_fn(target, window, cx);
                    }
                }
                None
            });
        }
        (interactive, layout)
    }

    fn highlight(&self, s: InlineStyle) -> HighlightStyle {
        let t = self.theme;
        let mut h = HighlightStyle::default();
        if s.bold {
            h.font_weight = Some(FontWeight::BOLD);
        }
        if s.italic {
            h.font_style = Some(FontStyle::Italic);
        }
        if s.underline {
            h.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            });
        }
        if s.strike {
            h.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(t.text_muted),
            });
            h.color = Some(t.text_muted);
        }
        if s.highlight {
            h.background_color = Some(match s.highlight_color {
                Some(c) => t.highlight_color(c),
                None => t.highlight_bg,
            });
        }
        if s.code {
            h.background_color = Some(t.code_bg);
        }
        if s.kbd {
            // * `<kbd>` paints as a key box: monospace over a filled wash.
            //   gpui pins no per-run border, so the box has no outline — a
            //   gpui limit reported with the feature.
            h.background_color = Some(t.code_bg);
        }
        if s.math {
            // * Only a formula that failed to typeset is still text here.
            h.color = Some(math::error_color());
        }
        if s.html {
            h.color = Some(t.text_muted);
        }
        // * `<sup>` / `<sub>`: raised or lowered text. This gpui's
        //   `HighlightStyle` carries no per-run font size or vertical
        //   offset, so they paint at base size until gpui can; the offset is
        //   a reported gpui limit, and `InlineStyle::sup`/`::sub` keep the
        //   intent on each run.
        if s.tag {
            h.color = Some(t.tag);
        }
        if s.footnote {
            h.color = Some(t.link);
        }
        if s.link {
            h.color = Some(t.link);
            h.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(t.link),
                wavy: false,
            });
        }
        h
    }

    /// Selection background color, drawn behind a selected byte range.
    fn selection_highlight(&self) -> HighlightStyle {
        let t = self.theme;
        HighlightStyle {
            background_color: Some(t.accent),
            color: Some(t.bg),
            ..Default::default()
        }
    }
}
