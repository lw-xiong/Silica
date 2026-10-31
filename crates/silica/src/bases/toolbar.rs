//! The Bases toolbar: the view switcher and the five panels its controls
//! open.
//!
//! Observed (`tools/obsidian-probe/results/2026-10-25-bases.json`; the prose
//! is in
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]):
//! the toolbar does **not** open menus of rows. Each control opens a
//! `.menu.bases-toolbar-menu` whose body is a panel —
//!
//! - **sort** and **group**: `.combobox-button` pairs (property, direction)
//!   with a `lucide-trash-2` button;
//! - **filter**: a `<select class="conjunction">` (`and`/`or`/`not`) per
//!   group, then a property combobox, an operator combobox, a value input and
//!   a `filer-expression-mode-toggle` (`lucide-code-xml`) per rule, in two
//!   sections, "All views" and "This view";
//! - **properties**: a checkbox list of every property with "Add *name*
//!   property", "Add formula" and "Hide all";
//! - **results**: a "Limit number of results" number input and the rows
//!   *Show all (n)*, *Copy to clipboard* and *Export CSV...*.
//!
//! The views menu lists one row per view, and the row's own icon opens that
//! view's settings (the Configure view page), whose Layout combobox offers
//! exactly `table`, `cards`, `list` and `kanban`.
//!
//! What the panels write is [`BasesView::edit_view`] and
//! [`BasesView::edit_base`]'s business: *Sort Z → A* **prepends**
//! `{property, direction: DESC}` to the view's `sort`, *Hide column* removes
//! the property from `order`, and the limit input writes `limit` as a bare
//! number.
//!
//! Every write below goes through one of those two closures and through
//! `silica_bases::layout`'s setters, so the bytes are `Base::to_yaml`'s and
//! nothing here serializes anything. The free functions at the bottom of the
//! module are where each rule lives, which is why they take a `&mut View` or
//! a `&mut Option<Filter>` rather than the element: the tests at the end
//! apply one and assert the written bytes.
//!
//! **Unprobed, and therefore not drawn or not written.** Three observed
//! controls are deliberately missing, because what each writes cannot be
//! derived from the run:
//!
//! - the properties panel's **"Hide all"**. `order` is exactly the shown
//!   columns, so hiding every column wants an empty `order` — but
//!   `silica_formats::base::View` never writes an empty `order` (the key is
//!   omitted), and a view with no `order` shows the *implied* columns, which
//!   is the opposite of hiding all. So the key "Hide all" writes is unknown.
//! - the properties panel's **"Add formula"**, whose dialog was never opened,
//!   so neither the formula name it writes nor its statement is observed.
//! - the results panel's ***Export CSV...***, whose file, location and
//!   dialect were never observed.
//!
//! Also unprobed and so not written: the *Summarize...* submenu and what a
//! summary chosen from the UI writes (`summaries` is read and round-tripped,
//! never written by a control here); the key the list layout's "Indent
//! properties" toggle writes (`layout::INDENT_PROPERTIES_KEY` is the plain
//! spelling and nothing writes it, so the toggle is not drawn); renaming a
//! view through the Configure view page's name field, so that page shows the
//! name as text; and whether `groupOrder` is ever written by the UI, so
//! nothing here writes or clears one.
//!
//! **Unprobed metrics.** No toolbar rect was measured in the run: the bar's
//! height, its padding, the gap between controls, and which `lucide-*` icon
//! each control carries are all this module's own. The height is the 30 px of
//! the table's header row, which is the one measured row of the view.

use crate::assets::icon;
use crate::bases::cells::table_tsv;
use crate::bases::view::BasesView;
use crate::bases::writes::{
    add_property, parse_limit, prepend_sort, remove_sort, same_property, set_group_direction,
    set_group_property, set_sort_direction, set_sort_property, toggle_property,
};
use crate::canvas::menu::tooltip;
use crate::search::menu;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{Theme, FONT_UI_SMALL, FONT_UI_SMALLER, ICON_SIZE};
use gpui::{
    anchored, deferred, div, prelude::*, px, Anchor, AnyElement, App, ClickEvent, ClipboardItem,
    Context, Entity, Focusable, Pixels, Point, SharedString, Subscription, Window,
};
use silica_bases::layout::{
    self, ImageFit, Layout, Markers, RowHeight, ASPECT_RANGE, CARD_SIZE_RANGE, COLUMN_WIDTH_RANGE,
    LAYOUTS,
};
use silica_formats::base::{Base, Filter, View, DEFAULT_VIEW_NAME, DEFAULT_VIEW_TYPE};
use serde_json::{json, Value as Json};

/// The bar's own height. **Unprobed** — no toolbar rect was measured; this is
/// the 30 px of the table header row, the one row of the view that was.
pub const TOOLBAR_HEIGHT: f32 = 30.;
/// Padding inside the bar and gap between its controls. **Unprobed**, as
/// above; 8 and 4 are the shell's own.
const PAD: f32 = 8.;
const GAP: f32 = 4.;

/// The `lucide-*` icon each toolbar control carries.
///
/// **Unprobed**: the run read the panels, never the buttons that open them,
/// so neither the icons nor their order is observed. Each name here is the
/// nearest drawing `crates/silica/assets/icons` bundles.
const CONTROLS: &[(Panel, &str)] = &[
    (Panel::Filter, "sliders-horizontal"),
    (Panel::Properties, "square-check"),
    (Panel::Sort, "arrow-up-narrow-wide"),
    (Panel::Group, "layers"),
    (Panel::Results, "list"),
];

/// The twelve `file.*` properties the properties panel offers, in the order
/// it lists them.
///
/// Eleven are observed, under the labels
/// `silica_bases::table::column` gives them: `file backlinks`, `file base
/// name`, `created time`, `file embeds`, `folder`, `file full name`,
/// `file links`, `modified time`, `file path`, `file size`, `file tags` —
/// which is alphabetical by the property's own name. `file.name` is the
/// twelfth and is a **reading**: the observed sentence does not list it, but
/// it is the one `file.*` column the run ever rendered and the panel is a
/// checkbox list of every property, so it is offered in its alphabetical
/// slot.
const FILE_PROPERTIES: &[&str] = &[
    "file.backlinks",
    "file.basename",
    "file.ctime",
    "file.embeds",
    "file.folder",
    "file.fullname",
    "file.links",
    "file.mtime",
    "file.name",
    "file.path",
    "file.size",
    "file.tags",
];

/// `ASC` and `DESC`, which Obsidian labels "A → Z" and "Z → A" — the two
/// labels its own column header menu uses (*Sort A → Z*, *Sort Z → A*).
const DIRECTIONS: &[(&str, &str)] = &[("ASC", "A → Z"), ("DESC", "Z → A")];

/// The conjunctions a `<select class="conjunction">` offers.
const CONJUNCTIONS: &[&str] = &["and", "or", "not"];

/// Which panel, if any, is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    None,
    Views,
    Filter,
    Properties,
    Sort,
    Group,
    Results,
}

impl Panel {
    /// The name the control socket reports, which is also the name the
    /// toolbar's own buttons go by.
    pub fn as_str(self) -> &'static str {
        match self {
            Panel::None => "none",
            Panel::Views => "views",
            Panel::Filter => "filter",
            Panel::Properties => "properties",
            Panel::Sort => "sort",
            Panel::Group => "group",
            Panel::Results => "results",
        }
    }
}

/// Which of the filter panel's two sections a group or rule belongs to: the
/// base's own `filters` ("All views") or the showing view's ("This view").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    AllViews,
    ThisView,
}

impl Section {
    /// Obsidian's own heading for the section.
    fn heading(self) -> &'static str {
        match self {
            Section::AllViews => "All views",
            Section::ThisView => "This view",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Section::AllViews => "all",
            Section::ThisView => "view",
        }
    }
}

/// Which text control the one shared [`TextInput`] is standing in for.
///
/// One field serves the whole toolbar, the way `SettingsView` keeps one for
/// every row of a settings tab: only one panel is open at a time, and inside
/// a panel only one field is being typed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The results panel's "Limit number of results" number input.
    Limit,
    /// The list layout's `separator` text field.
    Separator,
    /// The properties panel's own field, which both narrows the checkbox list
    /// and names the property "Add *name* property" adds.
    Property,
    /// One filter rule's statement, by index into [`BasesView::filter_nodes`].
    /// A rule past the end of a group is the group's own new-rule field.
    Rule(usize),
}

impl Field {
    fn as_str(self) -> &'static str {
        match self {
            Field::Limit => "limit",
            Field::Separator => "separator",
            Field::Property => "property",
            Field::Rule(_) => "rule",
        }
    }
}

/// What the toolbar keeps between renders.
#[derive(Default)]
pub struct ToolbarState {
    pub panel: Panel,
    /// Which view's Configure view page is showing inside the views panel.
    pub configuring: Option<usize>,
    /// The combobox whose options are showing: its name and the entry it
    /// belongs to (0 where there is only one of it).
    pub combo: Option<(&'static str, usize)>,
    /// Where the panel is anchored, which is where the control was clicked.
    /// **Unprobed**: no toolbar rect was measured, so there is no button box
    /// to hang the panel under.
    at: Point<Pixels>,
    /// The panel a press outside it just closed, with the position of that
    /// press. A control's own button sits outside its panel, so pressing it
    /// closes the panel on the way down and the click that follows would
    /// open it again; the position says whether the two are one gesture.
    just_closed: Option<(Panel, Point<Pixels>)>,
    /// The shared text field, made the first time a panel wants one.
    field: Option<Entity<TextInput>>,
    /// What [`ToolbarState::field`] is standing in for, if anything.
    pub editing: Option<Field>,
    /// What the field last held, so the control socket can read it without
    /// the app context a [`TextInput`] needs.
    field_value: String,
    /// How many rows the showing view resolves to with its `limit` dropped,
    /// which is the *n* of *Show all (n)*. Counted when the results panel
    /// opens, because the resolved table holds only the limited rows.
    total: Option<usize>,
    /// What the open panel drew, for the control socket. Filled during
    /// render, so `bases.toolbar` reports the panel's contents rather than
    /// only its name.
    rows: Vec<Json>,
    subscriptions: Vec<Subscription>,
}

impl ToolbarState {
    pub fn state_json(&self) -> Json {
        json!({
            "panel": self.panel.as_str(),
            "configuring": self.configuring,
            "combo": self.combo.map(|(name, ix)| json!({ "name": name, "ix": ix })),
            "field": self.editing.map(Field::as_str),
            "field_value": self.field_value,
            "total": self.total,
            "rows": self.rows,
        })
    }
}

// ---------------------------------------------------------------------------
// The bar
// ---------------------------------------------------------------------------

impl BasesView {
    /// The toolbar above the layout: the view switcher, the five controls,
    /// and whichever panel is open.
    pub(super) fn render_toolbar(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let base = self.base()?.clone();
        let view = self.showing_view(&base);
        let open = self.toolbar.panel;
        let icon_name = layout::kind_icon(&view.kind).unwrap_or("file-question");
        let bar = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .w_full()
            .h(px(TOOLBAR_HEIGHT))
            .px(px(PAD))
            .gap(px(GAP))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("bases-toolbar-views")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .h(px(22.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.text)
                    .when(open == Panel::Views, |d| d.bg(theme.bg_alt))
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        this.toggle_panel(Panel::Views, event.position(), window, cx)
                    }))
                    .child(icon(icon_name, 14., theme.text_muted))
                    .child(
                        div()
                            .whitespace_nowrap()
                            .child(SharedString::from(view.name.clone())),
                    )
                    .child(icon("chevron-down", 12., theme.text_muted)),
            )
            .child(div().flex_1())
            .children(
                CONTROLS
                    .iter()
                    .map(|(panel, name)| self.render_control(*panel, name, theme, cx)),
            );
        let panel = self.render_panel(&base, &view, theme, cx);
        Some(
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .w_full()
                .child(bar)
                .children(panel)
                .into_any_element(),
        )
    }

    /// One toolbar control: an icon that opens its panel.
    fn render_control(
        &self,
        panel: Panel,
        name: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.toolbar.panel == panel;
        div()
            .id(("bases-toolbar-control", panel as usize))
            .size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .when(open, |d| d.bg(theme.bg_alt))
            .hover(|s| s.bg(theme.bg_alt))
            .tooltip(tooltip(panel.as_str()))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.toggle_panel(panel, event.position(), window, cx)
            }))
            .child(icon(name, ICON_SIZE, theme.text_muted))
            .into_any_element()
    }

    /// Opens `panel`, or closes it when it was already open.
    fn toggle_panel(
        &mut self,
        panel: Panel,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        // The press that began this click lands outside the open panel —
        // a control's button is not inside its own panel — so the panel has
        // already closed itself by the time the click arrives. A second click
        // on the same control therefore closes rather than reopens.
        let reopening = self.toolbar.just_closed.take() == Some((panel, at));
        if self.toolbar.panel == panel || reopening {
            self.close_panel(cx);
            return;
        }
        self.toolbar.panel = panel;
        self.toolbar.at = at;
        self.toolbar.combo = None;
        self.toolbar.configuring = None;
        self.toolbar.editing = None;
        self.toolbar.total = None;
        self.toolbar.rows.clear();
        // Two panels open with a field already live, so the first keystroke
        // lands in it rather than opening it.
        match panel {
            Panel::Results => {
                let total = self.unlimited_rows();
                self.toolbar.total = Some(total);
                let text = self
                    .view()
                    .and_then(|v| v.limit)
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                self.open_field(Field::Limit, text, window, cx);
            }
            Panel::Properties => self.open_field(Field::Property, String::new(), window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Closes the panel because something outside it was pressed, keeping
    /// which panel it was and where the press landed; see
    /// [`ToolbarState::just_closed`].
    fn close_panel_out(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let was = self.toolbar.panel;
        self.close_panel(cx);
        self.toolbar.just_closed = Some((was, at));
    }

    fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.toolbar.panel = Panel::None;
        self.toolbar.combo = None;
        self.toolbar.configuring = None;
        self.toolbar.editing = None;
        self.toolbar.field_value.clear();
        self.toolbar.rows.clear();
        cx.notify();
    }

    /// The view whose keys the toolbar shows. An empty base has none, and
    /// what it shows is the default view `edit_view` would materialize.
    fn showing_view(&self, base: &Base) -> View {
        base.views
            .get(self.view_ix())
            .cloned()
            .unwrap_or_else(default_view)
    }

    /// How many rows the showing view resolves to with no `limit`, which is
    /// the *n* of *Show all (n)*: the resolved table carries only the limited
    /// rows, so the whole count has to be built for it.
    fn unlimited_rows(&self) -> usize {
        let Some(base) = self.base() else {
            return 0;
        };
        if self.table().is_some_and(|t| !t.limited) {
            return self.table().map_or(0, |t| t.rows.len());
        }
        let (Some(state), Some(path)) = (self.vault(), self.path()) else {
            return self.table().map_or(0, |t| t.rows.len());
        };
        let mut probe = base.clone();
        let ix = self.view_ix();
        if let Some(view) = probe.views.get_mut(ix) {
            view.limit = None;
        }
        let env = super::rows::env_for(state, path);
        silica_bases::table::build(&probe, ix, &env).rows.len()
    }

    // ----- the shared text field ---------------------------------------------

    /// Puts the shared field on `which`, with `text` in it, and focuses it.
    ///
    /// Nothing is written by installing the text: [`BasesView::commit_field`]
    /// compares before it writes, so the change event `set_text` raises for
    /// the value already in the file is a no-op. Obsidian writes nothing on an
    /// open, and neither does this.
    fn open_field(
        &mut self,
        which: Field,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.toolbar.field.is_none() {
            let input = cx.new(|cx| TextInput::new("", cx));
            self.toolbar.subscriptions =
                vec![cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
                    let text = input.read(cx).text().to_string();
                    match event {
                        InputEvent::Changed => this.commit_field(&text, cx),
                        InputEvent::Submit => {
                            this.commit_field(&text, cx);
                            if matches!(this.toolbar.editing, Some(Field::Rule(_))) {
                                this.toolbar.editing = None;
                            }
                            cx.notify();
                        }
                        InputEvent::Cancel => {
                            this.toolbar.editing = None;
                            cx.notify();
                        }
                        InputEvent::Up | InputEvent::Down => {}
                    }
                })];
            self.toolbar.field = Some(input);
        }
        self.toolbar.editing = Some(which);
        self.toolbar.field_value = text.clone();
        if let Some(input) = self.toolbar.field.clone() {
            input.update(cx, |input, cx| {
                input.set_text(text, cx);
                input.select_all(cx);
            });
            let handle = input.focus_handle(cx);
            window.focus(&handle, cx);
        }
    }

    /// What the shared field writes, by what it is standing in for.
    ///
    /// Every arm compares before it writes: the field raises a change event
    /// for the text it is opened with too, and a base must not be written
    /// because a panel was opened.
    fn commit_field(&mut self, text: &str, cx: &mut Context<Self>) {
        self.toolbar.field_value = text.to_string();
        match self.toolbar.editing {
            // Observed: the limit input writes `limit` as a bare number, and
            // an empty field is no limit at all.
            Some(Field::Limit) => {
                let limit = parse_limit(text);
                if self.view().and_then(|v| v.limit) == limit {
                    return;
                }
                self.edit_view(move |view| layout::set_limit(view, limit), cx);
            }
            Some(Field::Separator) => {
                let now = self
                    .base()
                    .map(|base| self.showing_view(base))
                    .and_then(|v| match Layout::of(&v) {
                        Layout::List(list) => Some(list.separator),
                        _ => None,
                    });
                if now.as_deref() == Some(text) {
                    return;
                }
                let separator = text.to_string();
                self.edit_view(move |view| layout::set_separator(view, &separator), cx);
            }
            // The property field only narrows the list; "Add *name* property"
            // is the row that writes.
            Some(Field::Property) => cx.notify(),
            Some(Field::Rule(ix)) => {
                let Some(node) = self.filter_nodes().get(ix).cloned() else {
                    return;
                };
                match &node.kind {
                    NodeKind::Rule(now) if now == text => return,
                    NodeKind::Group(_) if text.is_empty() => return,
                    _ => {}
                }
                let text = text.to_string();
                let new_rule = matches!(node.kind, NodeKind::Group(_));
                let path = node.path.clone();
                let kind = node.kind.clone();
                self.edit_filter(
                    node.section,
                    move |filter| {
                        let mut tree = group_tree(filter.as_ref());
                        match kind {
                            NodeKind::Rule(_) => set_rule(&mut tree, &path, &text),
                            NodeKind::Group(_) => add_rule(&mut tree, &path, &text),
                        }
                        *filter = written(tree);
                    },
                    cx,
                );
                // A group's new-rule field has become that rule's own field,
                // which the next render draws on the node the rule now is.
                if new_rule {
                    self.toolbar.editing = None;
                }
                cx.notify();
            }
            None => {}
        }
    }

    fn field_text(&self, cx: &Context<Self>) -> String {
        self.toolbar
            .field
            .as_ref()
            .map(|f| f.read(cx).text().to_string())
            .unwrap_or_default()
    }

    // ----- the panels ---------------------------------------------------------

    /// The open panel, anchored where its control was clicked.
    fn render_panel(
        &mut self,
        base: &Base,
        view: &View,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.toolbar.rows.clear();
        let body = match self.toolbar.panel {
            Panel::None => return None,
            Panel::Views => self.render_views_panel(base, theme, cx),
            Panel::Sort => self.render_sort_panel(base, view, theme, cx),
            Panel::Group => self.render_group_panel(base, view, theme, cx),
            Panel::Properties => self.render_properties_panel(base, view, theme, cx),
            Panel::Filter => self.render_filter_panel(base, view, theme, cx),
            Panel::Results => self.render_results_panel(view, theme, cx),
        };
        let at = self.toolbar.at;
        Some(
            deferred(
                anchored()
                    .position(at)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(body),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The `.menu.bases-toolbar-menu` box every panel sits in.
    fn panel_box(
        &self,
        id: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        menu(id, theme)
            .min_w(px(260.))
            .max_w(px(420.))
            .gap(px(2.))
            .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                this.close_panel_out(event.position, cx)
            }))
    }

    /// Opens the views panel on `ix`'s **Configure view** page, which is
    /// what `bases:add-view` does after appending a view: "appends a view
    /// named `View 2` and opens its Configure view page" is the observed
    /// effect, and this is its second half.
    pub(super) fn configure_view(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.toolbar.panel = Panel::Views;
        self.toolbar.configuring = Some(ix);
        self.toolbar.combo = None;
        cx.notify();
    }

    // ----- views + Configure view --------------------------------------------

    /// The views panel: one row per view, the row's own icon opening that
    /// view's Configure view page.
    fn render_views_panel(
        &mut self,
        base: &Base,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(ix) = self.toolbar.configuring {
            return self.render_configure(base, ix, theme, cx);
        }
        let views: Vec<View> = if base.views.is_empty() {
            vec![default_view()]
        } else {
            base.views.clone()
        };
        self.toolbar.rows = views
            .iter()
            .map(|v| json!({ "name": v.name, "type": v.kind }))
            .collect();
        let showing = self.view_ix();
        let mut body = self.panel_box("bases-toolbar-views-menu", theme, cx);
        for (ix, view) in views.iter().enumerate() {
            let icon_name = layout::kind_icon(&view.kind).unwrap_or("file-question");
            body = body.child(
                div()
                    .id(("bases-view-row", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(ix == showing, |d| d.bg(theme.bg_active))
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_view(ix, cx);
                        this.close_panel(cx);
                    }))
                    .child(
                        // The row's own icon is the Configure view page's
                        // handle, which is observed.
                        div()
                            .id(("bases-view-configure", ix))
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .hover(|s| s.bg(theme.bg_active))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.show_view(ix, cx);
                                this.toolbar.configuring = Some(ix);
                                cx.notify();
                            }))
                            .child(icon(icon_name, 14., theme.text_muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(SharedString::from(view.name.clone())),
                    ),
            );
        }
        body.into_any_element()
    }

    /// The Configure view page: the Layout combobox and that layout's own
    /// controls.
    ///
    /// Observed: the combobox offers exactly Table, Cards, List and Kanban,
    /// and picking one **rewrites `type` and leaves the previous layout's
    /// keys in place**. Obsidian 1.14.5 marks *Table* as the selected row
    /// whatever the view's real layout is; that is a bug the run recorded as
    /// one, so this marks the real layout instead.
    ///
    /// The page's name field is **not** drawn: what renaming a view through
    /// it writes was never observed, so the name shows as text.
    fn render_configure(
        &mut self,
        base: &Base,
        ix: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = base.views.get(ix).cloned().unwrap_or_else(default_view);
        let kind = view.kind.clone();
        let labels: Vec<SharedString> = LAYOUTS
            .iter()
            .map(|(_, label, _)| SharedString::from(*label))
            .collect();
        let selected = LAYOUTS.iter().position(|(k, ..)| *k == kind);
        let shown = layout::kind_label(&kind)
            .map(SharedString::from)
            .unwrap_or_else(|| SharedString::from(kind.clone()));
        let mut body = self
            .panel_box("bases-toolbar-configure", theme, cx)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(4.))
                    .pb(px(4.))
                    .child(
                        div()
                            .id("bases-configure-back")
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.bg_alt))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.toolbar.configuring = None;
                                this.toolbar.combo = None;
                                this.toolbar.editing = None;
                                cx.notify();
                            }))
                            .child(icon("arrow-left", 14., theme.text_muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(theme.text)
                            .child(SharedString::from(view.name.clone())),
                    ),
            )
            .child(self.setting_row(
                "Layout",
                self.combobox("layout", 0, Some(shown), &labels, selected, theme, cx),
                theme,
            ));
        self.toolbar.rows = vec![json!({ "control": "Layout", "value": kind })];
        // Each layout's own keys, from the observed layout-key table. The
        // label of every control but Row height is **unprobed** — the
        // Configure view page's own wording was never read — so the rest show
        // the key they write.
        match Layout::of(&view) {
            Layout::Table(table) => {
                let steps: Vec<SharedString> = RowHeight::all()
                    .iter()
                    .map(|h| SharedString::from(h.label()))
                    .collect();
                let now = RowHeight::all().iter().position(|h| *h == table.row_height);
                let shown = SharedString::from(table.row_height.label());
                self.record_control("rowHeight", table.row_height.key());
                body = body.child(self.setting_row(
                    "Row height",
                    self.combobox("rowHeight", 0, Some(shown), &steps, now, theme, cx),
                    theme,
                ));
            }
            Layout::Cards(cards) => {
                self.record_control("cardSize", &cards.card_size.to_string());
                body = body.child(self.setting_row(
                    "cardSize",
                    self.slider("cardSize", cards.card_size, CARD_SIZE_RANGE, theme, cx),
                    theme,
                ));
                body = body.child(self.image_row(base, &view, cards.image.as_deref(), theme, cx));
                body = body.child(self.image_fit_row(cards.image_fit, theme, cx));
                self.record_control("imageAspectRatio", &cards.image_aspect_ratio.to_string());
                body = body.child(self.setting_row(
                    "imageAspectRatio",
                    self.slider(
                        "imageAspectRatio",
                        cards.image_aspect_ratio,
                        ASPECT_RANGE,
                        theme,
                        cx,
                    ),
                    theme,
                ));
            }
            Layout::Kanban(kanban) => {
                self.record_control("columnWidth", &kanban.column_width.to_string());
                body = body.child(self.setting_row(
                    "columnWidth",
                    self.slider(
                        "columnWidth",
                        kanban.column_width,
                        COLUMN_WIDTH_RANGE,
                        theme,
                        cx,
                    ),
                    theme,
                ));
                body = body.child(self.image_row(base, &view, kanban.image.as_deref(), theme, cx));
                body = body.child(self.image_fit_row(kanban.image_fit, theme, cx));
                self.record_control("imageAspectRatio", &kanban.image_aspect_ratio.to_string());
                body = body.child(self.setting_row(
                    "imageAspectRatio",
                    self.slider(
                        "imageAspectRatio",
                        kanban.image_aspect_ratio,
                        ASPECT_RANGE,
                        theme,
                        cx,
                    ),
                    theme,
                ));
            }
            Layout::List(list) => {
                let marks: Vec<SharedString> = Markers::all()
                    .iter()
                    .map(|m| SharedString::from(m.label()))
                    .collect();
                let now = Markers::all().iter().position(|m| *m == list.markers);
                let shown = SharedString::from(list.markers.label());
                self.record_control("markers", list.markers.key());
                body = body.child(self.setting_row(
                    "markers",
                    self.combobox("markers", 0, Some(shown), &marks, now, theme, cx),
                    theme,
                ));
                self.record_control("separator", &list.separator);
                body = body.child(self.setting_row(
                    "separator",
                    self.text_field(Field::Separator, &list.separator, theme, cx),
                    theme,
                ));
                // "Indent properties" is not drawn: the key it writes is
                // unprobed, and nothing should write `indentProperties`.
            }
            // An unknown type never reaches the toolbar; the shell draws the
            // error block instead.
            Layout::Unknown(_) => {}
        }
        body.into_any_element()
    }

    /// The `image` property picker, which writes a **qualified** id.
    fn image_row(
        &self,
        base: &Base,
        view: &View,
        current: Option<&str>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let properties = self.properties_of(base, view);
        // The picker's own "no property" row was never read; this is the one
        // label in the module that is not observed, and it only clears a key.
        let mut labels = vec![SharedString::from("None")];
        labels.extend(properties.iter().map(|p| SharedString::from(p.1.clone())));
        let selected = match current {
            None => Some(0),
            Some(id) => properties
                .iter()
                .position(|p| same_property(&p.0, id))
                .map(|ix| ix + 1),
        };
        let shown = selected
            .and_then(|ix| labels.get(ix).cloned())
            .unwrap_or_else(|| SharedString::from(current.unwrap_or("None").to_string()));
        self.setting_row(
            "image",
            self.combobox("image", 0, Some(shown), &labels, selected, theme, cx),
            theme,
        )
    }

    fn image_fit_row(&self, fit: ImageFit, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let labels = [SharedString::from("Cover"), SharedString::from("Contain")];
        let now = match fit {
            ImageFit::Cover => 0,
            ImageFit::Contain => 1,
        };
        self.setting_row(
            "imageFit",
            self.combobox(
                "imageFit",
                0,
                Some(labels[now].clone()),
                &labels,
                Some(now),
                theme,
                cx,
            ),
            theme,
        )
    }

    fn record_control(&mut self, key: &str, value: &str) {
        self.toolbar
            .rows
            .push(json!({ "control": key, "value": value }));
    }

    // ----- results ------------------------------------------------------------

    /// The results panel: the limit input, *Show all (n)* and *Copy to
    /// clipboard*.
    ///
    /// Observed: the input writes `limit` as a bare number and *Show all*
    /// drops it; *Copy to clipboard* produces the same TSV `bases:copy-table`
    /// does. ***Export CSV...*** is not drawn — see the module note.
    fn render_results_panel(
        &mut self,
        view: &View,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let total = self.toolbar.total.unwrap_or(0);
        let limit = view.limit;
        self.toolbar.rows = vec![
            json!({ "control": "Limit number of results", "value": limit }),
            json!({ "row": format!("Show all ({total})") }),
            json!({ "row": "Copy to clipboard" }),
        ];
        let field = self.text_field(
            Field::Limit,
            &limit.map(|n| n.to_string()).unwrap_or_default(),
            theme,
            cx,
        );
        self.panel_box("bases-toolbar-results", theme, cx)
            .child(self.setting_row("Limit number of results", field, theme))
            .child(self.panel_row(
                "bases-results-show-all",
                0,
                None,
                SharedString::from(format!("Show all ({total})")),
                theme,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.edit_view(|view| layout::set_limit(view, None), cx);
                    let total = this.unlimited_rows();
                    this.toolbar.total = Some(total);
                    // The field has to follow the key it writes; the change
                    // event that raises is a no-op, because `limit` is
                    // already gone.
                    if let Some(input) = this.toolbar.field.clone() {
                        input.update(cx, |input, cx| input.set_text("", cx));
                    }
                    cx.notify();
                }),
            ))
            .child(self.panel_row(
                "bases-results-copy",
                0,
                Some("copy"),
                SharedString::from("Copy to clipboard"),
                theme,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.copy_table(cx);
                }),
            ))
            .into_any_element()
    }

    /// The visible columns as TSV, on the clipboard, with Obsidian's own
    /// notice.
    ///
    /// Observed for `bases:copy-table`: one header row of display names, tab
    /// separators, no trailing newline, and a "Copied to your clipboard"
    /// notice. The results panel's own row produces the same TSV.
    fn copy_table(&mut self, cx: &mut Context<Self>) {
        let Some(table) = self.table() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(table_tsv(table)));
        self.report("Copied to your clipboard", cx);
        self.close_panel(cx);
    }

    // ----- sort ---------------------------------------------------------------

    /// The sort panel: a (property, direction) pair per entry with a
    /// `lucide-trash-2`, over the row that adds one.
    ///
    /// Observed: an insertion into `sort` **prepends** — *Sort Z → A* puts
    /// `{property, direction: DESC}` at the head of the list. The panel's own
    /// add affordance was never read, so it is drawn at the head and
    /// prepends too, which is the only insertion the run pins down.
    fn render_sort_panel(
        &mut self,
        base: &Base,
        view: &View,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let properties = self.properties_of(base, view);
        let labels: Vec<SharedString> = properties
            .iter()
            .map(|p| SharedString::from(p.1.clone()))
            .collect();
        let directions: Vec<SharedString> = DIRECTIONS
            .iter()
            .map(|(_, label)| SharedString::from(*label))
            .collect();
        self.toolbar.rows = view
            .sort
            .iter()
            .map(|s| json!({ "property": s.property, "direction": s.direction }))
            .collect();
        let mut body = self
            .panel_box("bases-toolbar-sort", theme, cx)
            .child(self.add_row("sortAdd", &labels, theme, cx));
        for (ix, entry) in view.sort.iter().enumerate() {
            let selected = properties
                .iter()
                .position(|p| same_property(&p.0, &entry.property));
            let shown = selected
                .and_then(|i| labels.get(i).cloned())
                .unwrap_or_else(|| SharedString::from(entry.property.clone()));
            let down = direction_ix(entry.direction.as_deref());
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .px(px(4.))
                    .child(div().flex_1().min_w(px(0.)).child(self.combobox(
                        "sortProperty",
                        ix,
                        Some(shown),
                        &labels,
                        selected,
                        theme,
                        cx,
                    )))
                    .child(self.combobox(
                        "sortDirection",
                        ix,
                        Some(directions[down].clone()),
                        &directions,
                        Some(down),
                        theme,
                        cx,
                    ))
                    .child(self.trash(
                        "bases-sort-trash",
                        ix,
                        theme,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.edit_view(move |view| remove_sort(view, ix), cx);
                        }),
                    )),
            );
        }
        body.into_any_element()
    }

    // ----- group --------------------------------------------------------------

    /// The group panel: the same (property, direction) pair for `groupBy`,
    /// which is one entry rather than a list.
    ///
    /// Nothing here writes or clears `groupOrder`: whether the UI ever writes
    /// one is unprobed, so a `groupOrder` the file carries is left exactly as
    /// it was found.
    fn render_group_panel(
        &mut self,
        base: &Base,
        view: &View,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let properties = self.properties_of(base, view);
        let labels: Vec<SharedString> = properties
            .iter()
            .map(|p| SharedString::from(p.1.clone()))
            .collect();
        let directions: Vec<SharedString> = DIRECTIONS
            .iter()
            .map(|(_, label)| SharedString::from(*label))
            .collect();
        self.toolbar.rows = view
            .group_by
            .iter()
            .map(|g| json!({ "property": g.property, "direction": g.direction }))
            .collect();
        let mut body = self.panel_box("bases-toolbar-group", theme, cx);
        match &view.group_by {
            None => body = body.child(self.add_row("groupAdd", &labels, theme, cx)),
            Some(group) => {
                let selected = properties
                    .iter()
                    .position(|p| same_property(&p.0, &group.property));
                let shown = selected
                    .and_then(|i| labels.get(i).cloned())
                    .unwrap_or_else(|| SharedString::from(group.property.clone()));
                let down = direction_ix(group.direction.as_deref());
                body = body.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.))
                        .px(px(4.))
                        .child(div().flex_1().min_w(px(0.)).child(self.combobox(
                            "groupProperty",
                            0,
                            Some(shown),
                            &labels,
                            selected,
                            theme,
                            cx,
                        )))
                        .child(self.combobox(
                            "groupDirection",
                            0,
                            Some(directions[down].clone()),
                            &directions,
                            Some(down),
                            theme,
                            cx,
                        ))
                        .child(self.trash(
                            "bases-group-trash",
                            0,
                            theme,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.edit_view(|view| view.group_by = None, cx);
                            }),
                        )),
                );
            }
        }
        body.into_any_element()
    }

    // ----- properties ---------------------------------------------------------

    /// The properties panel: a checkbox per property, over the field that
    /// narrows the list and the "Add *name* property" row.
    ///
    /// Observed: `order` is exactly the shown columns, so checking a property
    /// appends its id and unchecking removes it. A box is checked when the
    /// resolved table shows that column, which is also what makes unchecking
    /// work on a view with no `order` at all: the implied columns are written
    /// out minus the one unchecked. That materialization is a **reading** —
    /// no fixture reached the renderer without an `order`.
    ///
    /// "Hide all" and "Add formula" are not drawn; see the module note.
    fn render_properties_panel(
        &mut self,
        base: &Base,
        view: &View,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let properties = self.properties_of(base, view);
        let shown = self.shown_columns();
        let typed = match self.toolbar.editing {
            Some(Field::Property) => self.field_text(cx),
            _ => String::new(),
        };
        let needle = typed.trim().to_ascii_lowercase();
        let matching: Vec<(usize, (String, String))> = properties
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, (id, label))| {
                needle.is_empty()
                    || label.to_ascii_lowercase().contains(&needle)
                    || id.to_ascii_lowercase().contains(&needle)
            })
            .collect();
        self.toolbar.rows = matching
            .iter()
            .map(|(_, (id, label))| {
                json!({
                    "id": id,
                    "label": label,
                    "checked": shown.iter().any(|s| same_property(s, id)),
                })
            })
            .collect();
        let mut body = self.panel_box("bases-toolbar-properties", theme, cx).child(
            div()
                .px(px(4.))
                .pb(px(4.))
                .child(self.text_field(Field::Property, &typed, theme, cx)),
        );
        for (ix, (id, label)) in matching {
            let checked = shown.iter().any(|s| same_property(s, &id));
            let write = id.clone();
            let shown_now = shown.clone();
            body = body.child(
                div()
                    .id(("bases-property-row", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let id = write.clone();
                        let shown = shown_now.clone();
                        this.edit_view(move |view| toggle_property(view, &shown, &id), cx);
                    }))
                    .child(check_box(checked, theme))
                    .child(
                        div()
                            .flex_1()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(SharedString::from(label)),
                    ),
            );
        }
        // "Add *name* property": the italic *name* is the field's own text,
        // which is how the row names a property the vault does not carry yet.
        let name = typed.trim().to_string();
        if !name.is_empty() && !properties.iter().any(|(_, label)| *label == name) {
            let id = layout::qualify(&name);
            body = body.child(self.panel_row(
                "bases-property-add",
                0,
                Some("plus"),
                SharedString::from(format!("Add {name} property")),
                theme,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let id = id.clone();
                    this.edit_view(move |view| add_property(view, &layout::qualify(&id)), cx);
                }),
            ));
        }
        body.into_any_element()
    }

    /// The ids of the columns the showing view resolves to, which is what the
    /// checkbox list reflects.
    fn shown_columns(&self) -> Vec<String> {
        self.table()
            .map(|t| t.columns.iter().map(|c| c.id.clone()).collect())
            .unwrap_or_default()
    }

    // ----- filter -------------------------------------------------------------

    /// The filter panel: the base's own `filters` under "All views" and the
    /// view's under "This view", a conjunction select per group and a
    /// statement field per rule.
    ///
    /// The **operator combobox is not drawn**, and with it the structured
    /// (property, operator, value) triple: the run read the rule's shape but
    /// never its operator list, so building one would invent the statement
    /// text a rule writes. The panel is therefore the expression mode the
    /// `filer-expression-mode-toggle` switches into — the toggle is drawn on
    /// each rule, lit, and says so — and a rule is its statement verbatim,
    /// which is the one form `silica_formats` round-trips byte for byte.
    ///
    /// An absent `filters` and a bare top-level statement are both shown as
    /// an `and` group, so every section has one group at its root; the first
    /// edit of a bare statement therefore writes it as `and: [..]`. What
    /// Obsidian writes for a single rule is unobserved.
    fn render_filter_panel(
        &mut self,
        _base: &Base,
        _view: &View,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let nodes = self.filter_nodes();
        self.toolbar.rows = nodes
            .iter()
            .map(|n| match &n.kind {
                NodeKind::Group(conj) => json!({
                    "section": n.section.as_str(),
                    "depth": n.depth,
                    "conjunction": conj,
                }),
                NodeKind::Rule(text) => json!({
                    "section": n.section.as_str(),
                    "depth": n.depth,
                    "rule": text,
                }),
            })
            .collect();
        let conjunctions: Vec<SharedString> = CONJUNCTIONS
            .iter()
            .map(|c| SharedString::from(*c))
            .collect();
        let mut body = self.panel_box("bases-toolbar-filter", theme, cx);
        let mut section = None;
        for (ix, node) in nodes.iter().enumerate() {
            if section != Some(node.section) {
                section = Some(node.section);
                body = body.child(
                    div()
                        .px(px(8.))
                        .pt(px(6.))
                        .pb(px(2.))
                        .text_size(px(FONT_UI_SMALLER))
                        .text_color(theme.text_faint)
                        .child(SharedString::from(node.section.heading())),
                );
            }
            let indent = px(4. + 12. * node.depth as f32);
            match &node.kind {
                NodeKind::Group(conj) => {
                    let now = CONJUNCTIONS.iter().position(|c| c == conj);
                    let shown = SharedString::from(*conj);
                    body = body.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.))
                            .pl(indent)
                            .pr(px(4.))
                            .child(self.combobox(
                                "conjunction",
                                ix,
                                Some(shown),
                                &conjunctions,
                                now,
                                theme,
                                cx,
                            )),
                    );
                    // The group's own new-rule field: typing in it appends a
                    // rule to this group.
                    body = body.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.))
                            .pl(px(4. + 12. * (node.depth + 1) as f32))
                            .pr(px(4.))
                            .child(div().flex_1().min_w(px(0.)).child(self.text_field(
                                Field::Rule(ix),
                                "",
                                theme,
                                cx,
                            ))),
                    );
                }
                NodeKind::Rule(text) => {
                    body = body.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.))
                            .pl(indent)
                            .pr(px(4.))
                            .child(div().flex_1().min_w(px(0.)).child(self.text_field(
                                Field::Rule(ix),
                                text,
                                theme,
                                cx,
                            )))
                            .child(
                                // `filer-expression-mode-toggle`, spelled as
                                // Obsidian spells it. It is lit and inert:
                                // expression mode is the only mode this panel
                                // has, because the operator list is unprobed.
                                div()
                                    .id(("bases-rule-mode", ix))
                                    .size(px(22.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .bg(theme.bg_active)
                                    .tooltip(tooltip(EXPRESSION_MODE_TOGGLE))
                                    .child(icon("code-xml", 14., theme.accent)),
                            )
                            .child(self.trash(
                                "bases-rule-trash",
                                ix,
                                theme,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.remove_rule_at(ix, cx);
                                }),
                            )),
                    );
                }
            }
        }
        body.into_any_element()
    }

    /// Every group and rule of both sections, flattened, which is what the
    /// filter panel's combobox and field keys index into.
    fn filter_nodes(&self) -> Vec<Node> {
        let Some(base) = self.base() else {
            return Vec::new();
        };
        let view = self.showing_view(base);
        let mut out = Vec::new();
        flatten(
            Section::AllViews,
            &group_tree(base.filters.as_ref()),
            Vec::new(),
            0,
            &mut out,
        );
        flatten(
            Section::ThisView,
            &group_tree(view.filters.as_ref()),
            Vec::new(),
            0,
            &mut out,
        );
        out
    }

    fn remove_rule_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(node) = self.filter_nodes().get(ix).cloned() else {
            return;
        };
        self.toolbar.editing = None;
        let path = node.path.clone();
        self.edit_filter(
            node.section,
            move |filter| {
                let mut tree = group_tree(filter.as_ref());
                remove_at(&mut tree, &path);
                *filter = written(tree);
            },
            cx,
        );
        cx.notify();
    }

    /// Routes a filter write to the section it belongs to: the base's own
    /// `filters` ("All views") or the showing view's ("This view").
    fn edit_filter<F>(&mut self, section: Section, edit: F, cx: &mut Context<Self>)
    where
        F: FnOnce(&mut Option<Filter>),
    {
        match section {
            Section::AllViews => self.edit_base(move |base| edit(&mut base.filters), cx),
            Section::ThisView => self.edit_view(move |view| edit(&mut view.filters), cx),
        }
    }

    // ----- the widgets --------------------------------------------------------

    /// A labelled row of the Configure view and results panels.
    fn setting_row(&self, label: &str, control: AnyElement, theme: &Theme) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(4.))
            .py(px(3.))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(label.to_string())),
            )
            .child(control)
            .into_any_element()
    }

    /// A row of a panel: an icon slot, a label, and what a click does.
    fn panel_row(
        &self,
        prefix: &'static str,
        ix: usize,
        icon_name: Option<&'static str>,
        label: SharedString,
        theme: &Theme,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        div()
            .id((prefix, ix))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_alt))
            .on_click(on_click)
            .child(
                div()
                    .size(px(16.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(icon_name.map(|n| icon(n, ICON_SIZE, theme.text_muted))),
            )
            .child(div().whitespace_nowrap().child(label))
            .into_any_element()
    }

    /// The `lucide-trash-2` button an entry of the sort, group and filter
    /// panels carries.
    fn trash(
        &self,
        prefix: &'static str,
        ix: usize,
        theme: &Theme,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        div()
            .id((prefix, ix))
            .size(px(22.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_alt))
            .on_click(on_click)
            .child(icon("trash-2", 14., theme.text_muted))
            .into_any_element()
    }

    /// The head of the sort and group panels: a `plus` that opens the
    /// property list and, on a pick, adds the entry.
    fn add_row(
        &self,
        name: &'static str,
        labels: &[SharedString],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(4.))
            .child(self.combobox(name, 0, None, labels, None, theme, cx))
            .into_any_element()
    }

    /// A `.combobox-button`: the shown value, and its options while it is
    /// open. `shown` of `None` draws the `plus` trigger the add rows use.
    #[allow(clippy::too_many_arguments)]
    fn combobox(
        &self,
        name: &'static str,
        ix: usize,
        shown: Option<SharedString>,
        options: &[SharedString],
        selected: Option<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.toolbar.combo == Some((name, ix));
        let trigger = match shown {
            Some(text) => div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(4.))
                .px(px(6.))
                .h(px(22.))
                .min_w(px(0.))
                .rounded(px(4.))
                .bg(theme.bg_alt)
                .border_1()
                .border_color(theme.border)
                .text_size(px(FONT_UI_SMALLER))
                .text_color(theme.text)
                .child(
                    div()
                        .min_w(px(0.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(text),
                )
                .child(icon("chevron-down", 12., theme.text_muted)),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.))
                .rounded(px(4.))
                .bg(theme.bg_alt)
                .border_1()
                .border_color(theme.border)
                .child(icon("plus", 14., theme.text_muted)),
        };
        let mut wrap = div().relative().min_w(px(0.)).child(
            div()
                .id(("bases-combobox", hash_key(name, ix)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toolbar.combo =
                        (this.toolbar.combo != Some((name, ix))).then_some((name, ix));
                    cx.notify();
                }))
                .child(trigger),
        );
        if open {
            let options: Vec<SharedString> = options.to_vec();
            wrap = wrap.child(
                deferred(
                    div()
                        .id(("bases-combobox-list", hash_key(name, ix)))
                        .absolute()
                        .top(px(24.))
                        .left(px(0.))
                        .min_w(px(180.))
                        .max_h(px(320.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .p(px(4.))
                        .bg(theme.bg_secondary)
                        .border_1()
                        .border_color(theme.tab_divider)
                        .rounded(px(8.))
                        .shadow_lg()
                        .occlude()
                        .text_size(px(FONT_UI_SMALLER))
                        .text_color(theme.text)
                        .children(options.into_iter().enumerate().map(|(opt, label)| {
                            div()
                                .id(("bases-combobox-option", hash_key(name, ix) * 512 + opt))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.))
                                .px(px(6.))
                                .py(px(3.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .when(selected == Some(opt), |d| d.bg(theme.bg_active))
                                .hover(|s| s.bg(theme.bg_alt))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.combo_pick(name, ix, opt, cx);
                                }))
                                .child(
                                    div()
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(label),
                                )
                        })),
                )
                .with_priority(2),
            );
        }
        wrap.into_any_element()
    }

    /// A slider over one of the layout keys' observed `(min, max, step)`
    /// ranges: one cell per step, the way the settings sliders are drawn, so
    /// a click lands on a value without measuring the track.
    fn slider(
        &self,
        name: &'static str,
        value: f64,
        range: (f64, f64, f64),
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (min, max, step) = range;
        let count = (((max - min) / step).round() as usize) + 1;
        let now = (((layout::snap(value, range) - min) / step).round().max(0.) as usize).min(count);
        let width = (120. / count as f32).max(2.);
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(14.))
                    .children((0..count).map(|i| {
                        div()
                            .id(("bases-slider", hash_key(name, 0) * 1024 + i))
                            .w(px(width))
                            .h(px(4.))
                            .cursor_pointer()
                            .bg(if i <= now {
                                theme.accent
                            } else {
                                theme.bg_active
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.slider_pick(name, min + i as f64 * step, cx);
                            }))
                    })),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(FONT_UI_SMALLER))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(number_text(value))),
            )
            .into_any_element()
    }

    /// A text field: the shared [`TextInput`] when it is standing in for
    /// `which`, and the value as a box that opens it otherwise.
    fn text_field(
        &self,
        which: Field,
        value: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.toolbar.editing == Some(which) {
            if let Some(input) = self.toolbar.field.clone() {
                return div()
                    .id("bases-toolbar-field")
                    .min_w(px(120.))
                    // A click inside the open field is not a click away from
                    // the panel, which is what the root would read it as.
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(input)
                    .into_any_element();
            }
        }
        let text = value.to_string();
        let key = match which {
            Field::Rule(ix) => ix + 1,
            _ => 0,
        };
        div()
            .id(("bases-toolbar-field-shut", hash_key(which.as_str(), key)))
            .min_w(px(120.))
            .px(px(6.))
            .h(px(22.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .bg(theme.bg_alt)
            .border_1()
            .border_color(theme.border)
            .cursor_text()
            .text_size(px(FONT_UI_SMALLER))
            .text_color(if text.is_empty() {
                theme.text_faint
            } else {
                theme.text
            })
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .child(SharedString::from(text.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.open_field(which, text.clone(), window, cx);
            }))
            .into_any_element()
    }

    // ----- what the comboboxes and sliders write -----------------------------

    /// Every combobox's write, by the name the box goes by.
    fn combo_pick(&mut self, name: &'static str, ix: usize, opt: usize, cx: &mut Context<Self>) {
        self.toolbar.combo = None;
        match name {
            // Observed: picking a layout rewrites `type` and leaves the
            // previous layout's keys in place.
            "layout" => {
                if let Some((kind, ..)) = LAYOUTS.get(opt) {
                    let kind = kind.to_string();
                    self.edit_view(move |view| layout::set_kind(view, &kind), cx);
                }
            }
            "rowHeight" => {
                if let Some(h) = RowHeight::all().get(opt).copied() {
                    self.edit_view(move |view| layout::set_row_height(view, h), cx);
                }
            }
            "markers" => {
                if let Some(m) = Markers::all().get(opt).copied() {
                    self.edit_view(move |view| layout::set_markers(view, m), cx);
                }
            }
            "imageFit" => {
                let fit = if opt == 1 {
                    ImageFit::Contain
                } else {
                    ImageFit::Cover
                };
                self.edit_view(move |view| layout::set_image_fit(view, fit), cx);
            }
            "image" => {
                let id = opt.checked_sub(1).and_then(|i| self.property_id(i));
                self.edit_view(move |view| layout::set_image(view, id.as_deref()), cx);
            }
            "sortAdd" => {
                if let Some(id) = self.property_id(opt) {
                    self.edit_view(
                        move |view| prepend_sort(view, &layout::qualify(&id), "ASC"),
                        cx,
                    );
                }
            }
            "sortProperty" => {
                if let Some(id) = self.property_id(opt) {
                    self.edit_view(
                        move |view| set_sort_property(view, ix, &layout::qualify(&id)),
                        cx,
                    );
                }
            }
            "sortDirection" => {
                let dir = DIRECTIONS.get(opt).map(|(d, _)| d.to_string());
                if let Some(dir) = dir {
                    self.edit_view(move |view| set_sort_direction(view, ix, &dir), cx);
                }
            }
            "groupAdd" | "groupProperty" => {
                if let Some(id) = self.property_id(opt) {
                    self.edit_view(
                        move |view| set_group_property(view, &layout::qualify(&id)),
                        cx,
                    );
                }
            }
            "groupDirection" => {
                let dir = DIRECTIONS.get(opt).map(|(d, _)| d.to_string());
                if let Some(dir) = dir {
                    self.edit_view(move |view| set_group_direction(view, &dir), cx);
                }
            }
            "conjunction" => {
                let Some(conj) = CONJUNCTIONS.get(opt).copied() else {
                    return;
                };
                let Some(node) = self.filter_nodes().get(ix).cloned() else {
                    return;
                };
                let path = node.path.clone();
                self.edit_filter(
                    node.section,
                    move |filter| {
                        let mut tree = group_tree(filter.as_ref());
                        set_conjunction(&mut tree, &path, conj);
                        *filter = written(tree);
                    },
                    cx,
                );
            }
            _ => {}
        }
        cx.notify();
    }

    /// Every slider's write, by the key it writes.
    fn slider_pick(&mut self, name: &'static str, value: f64, cx: &mut Context<Self>) {
        match name {
            "cardSize" => self.edit_view(move |view| layout::set_card_size(view, value), cx),
            "columnWidth" => self.edit_view(move |view| layout::set_column_width(view, value), cx),
            "imageAspectRatio" => {
                self.edit_view(move |view| layout::set_image_aspect_ratio(view, value), cx)
            }
            _ => {}
        }
    }

    /// The id of the `n`th row of the property pickers, which every one of
    /// them is built from. Rebuilt rather than cached: a picker is only open
    /// while its panel is, and the set is a few dozen strings.
    fn property_id(&self, n: usize) -> Option<String> {
        let base = self.base()?;
        let view = self.showing_view(base);
        self.properties_of(base, &view)
            .get(n)
            .map(|(id, _)| id.clone())
    }

    // ----- the property set ---------------------------------------------------

    /// Every property the pickers and the checkbox list offer: the twelve
    /// `file.*` ones, then the base's formulas, then the frontmatter
    /// properties the rows carry, then anything the view's `order` names that
    /// none of those reached.
    ///
    /// The file set is the one the properties panel was observed to name;
    /// the formulas and the `order` tail are **readings** — "a checkbox list
    /// of every property" does not say whether a formula is one, and
    /// `formula.ppu` renders as a column, so it is offered.
    fn properties_of(&self, base: &Base, view: &View) -> Vec<(String, String)> {
        let mut ids: Vec<String> = FILE_PROPERTIES.iter().map(|id| id.to_string()).collect();
        fn push(ids: &mut Vec<String>, id: String) {
            if !ids.iter().any(|seen| same_property(seen, &id)) {
                ids.push(id);
            }
        }
        for name in base.formulas.keys() {
            push(&mut ids, format!("formula.{name}"));
        }
        for row in self.table().iter().flat_map(|t| t.rows.iter()) {
            for name in row.file.properties.keys() {
                push(&mut ids, format!("note.{name}"));
            }
        }
        for id in &view.order {
            push(&mut ids, layout::qualify(id));
        }
        ids.into_iter()
            .map(|id| {
                let display = silica_bases::table::column(base, &id).display;
                (id, display)
            })
            .collect()
    }
}

/// The class Obsidian puts on a rule's mode button, spelled as Obsidian
/// spells it — `filer`, not `filter`.
pub const EXPRESSION_MODE_TOGGLE: &str = "filer-expression-mode-toggle";

/// A small checkbox, as the properties panel's rows carry.
fn check_box(checked: bool, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(14.))
        .rounded_sm()
        .border_1()
        .border_color(if checked { theme.accent } else { theme.border })
        .when(checked, |d| {
            d.bg(theme.accent).child(icon("check", 10., theme.bg))
        })
}

/// A stable element-id number for a `(name, index)` key.
fn hash_key(name: &str, ix: usize) -> usize {
    name.bytes().fold(ix.wrapping_mul(31), |h, b| {
        h.wrapping_mul(131).wrapping_add(b as usize)
    }) % 100_000
}

/// A number as the sliders label one: integral values bare, the rest with
/// their decimals, which is the rule the keys are written by.
fn number_text(v: f64) -> String {
    if v.fract() == 0. {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// The view an empty base shows, which is what `edit_view` materializes for
/// a write.
fn default_view() -> View {
    View {
        kind: DEFAULT_VIEW_TYPE.to_string(),
        name: DEFAULT_VIEW_NAME.to_string(),
        ..View::default()
    }
}

/// Which of [`DIRECTIONS`] a direction is; an absent one is ascending.
fn direction_ix(direction: Option<&str>) -> usize {
    match direction {
        Some(d) if d.eq_ignore_ascii_case("desc") => 1,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// What the panels write
// ---------------------------------------------------------------------------

// ----- the filter tree -------------------------------------------------------

/// One group or rule of the filter panel, flattened.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub section: Section,
    /// The indices to descend from the section's root group.
    pub path: Vec<usize>,
    pub depth: usize,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// A conjunction group: `and`, `or` or `not`.
    Group(&'static str),
    /// A rule: its statement, verbatim.
    Rule(String),
}

/// The tree the filter panel edits: an absent `filters` and a bare statement
/// are both read as an `and` group, so every section has one group at its
/// root and a conjunction select to go with it.
pub fn group_tree(filter: Option<&Filter>) -> Filter {
    match filter {
        None => Filter::And(Vec::new()),
        Some(Filter::Statement(s)) => Filter::And(vec![Filter::Statement(s.clone())]),
        Some(other) => other.clone(),
    }
}

/// What a group tree is written back as: an empty group is no `filters` key
/// at all, which is the shape the file had before the panel touched it.
pub fn written(tree: Filter) -> Option<Filter> {
    let empty = match &tree {
        Filter::And(items) | Filter::Or(items) | Filter::Not(items) => items.is_empty(),
        Filter::Statement(_) => false,
    };
    if empty {
        None
    } else {
        Some(tree)
    }
}

fn conjunction_of(filter: &Filter) -> &'static str {
    match filter {
        Filter::And(_) => "and",
        Filter::Or(_) => "or",
        Filter::Not(_) => "not",
        Filter::Statement(_) => "and",
    }
}

fn items_of(filter: &Filter) -> Option<&Vec<Filter>> {
    match filter {
        Filter::And(items) | Filter::Or(items) | Filter::Not(items) => Some(items),
        Filter::Statement(_) => None,
    }
}

fn items_mut(filter: &mut Filter) -> Option<&mut Vec<Filter>> {
    match filter {
        Filter::And(items) | Filter::Or(items) | Filter::Not(items) => Some(items),
        Filter::Statement(_) => None,
    }
}

fn at_mut<'f>(tree: &'f mut Filter, path: &[usize]) -> Option<&'f mut Filter> {
    match path.split_first() {
        None => Some(tree),
        Some((head, rest)) => at_mut(items_mut(tree)?.get_mut(*head)?, rest),
    }
}

/// Flattens a section's tree into the nodes the panel draws.
pub fn flatten(
    section: Section,
    tree: &Filter,
    path: Vec<usize>,
    depth: usize,
    out: &mut Vec<Node>,
) {
    match tree {
        Filter::Statement(s) => out.push(Node {
            section,
            path,
            depth,
            kind: NodeKind::Rule(s.clone()),
        }),
        group => {
            out.push(Node {
                section,
                path: path.clone(),
                depth,
                kind: NodeKind::Group(conjunction_of(group)),
            });
            for (ix, item) in items_of(group).into_iter().flatten().enumerate() {
                let mut child = path.clone();
                child.push(ix);
                flatten(section, item, child, depth + 1, out);
            }
        }
    }
}

/// Rewraps the group at `path` under another conjunction, keeping its rules.
pub fn set_conjunction(tree: &mut Filter, path: &[usize], conjunction: &str) {
    let Some(node) = at_mut(tree, path) else {
        return;
    };
    let items = match items_mut(node) {
        Some(items) => std::mem::take(items),
        None => return,
    };
    *node = match conjunction {
        "or" => Filter::Or(items),
        "not" => Filter::Not(items),
        _ => Filter::And(items),
    };
}

/// Replaces the rule at `path` with `text`, verbatim.
pub fn set_rule(tree: &mut Filter, path: &[usize], text: &str) {
    if let Some(node) = at_mut(tree, path) {
        *node = Filter::Statement(text.to_string());
    }
}

/// Appends a rule to the group at `path`. An empty statement writes nothing:
/// the new-rule field is live before it has anything in it.
pub fn add_rule(tree: &mut Filter, path: &[usize], text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(items) = at_mut(tree, path).and_then(items_mut) {
        items.push(Filter::Statement(text.to_string()));
    }
}

/// Removes whatever is at `path` from its parent group.
pub fn remove_at(tree: &mut Filter, path: &[usize]) {
    let Some((last, parent)) = path.split_last() else {
        return;
    };
    if let Some(items) = at_mut(tree, parent).and_then(items_mut) {
        if *last < items.len() {
            items.remove(*last);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_bases::Table;
    use silica_formats::base::Base;

    fn base_of(yaml: &str) -> Base {
        Base::from_yaml(yaml).expect("parses")
    }

    fn yaml_of(base: &Base) -> String {
        base.to_yaml()
    }

    #[test]
    fn a_sort_added_from_the_panel_goes_to_the_head_of_the_list() {
        // Observed: *Sort Z → A* prepends `{property, direction: DESC}`.
        let mut base = base_of(
            "views:\n  - type: table\n    name: V\n    sort:\n      - property: file.name\n        direction: ASC\n",
        );
        prepend_sort(&mut base.views[0], "note.status", "DESC");
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: table\n    name: V\n    sort:\n      - property: note.status\n        direction: DESC\n      - property: file.name\n        direction: ASC\n"
        );
    }

    #[test]
    fn a_sort_entry_keeps_the_bare_id_the_file_wrote() {
        // The writer "does not qualify ids that are already written bare", so
        // changing a direction must not rewrite the property.
        let mut base =
            base_of("views:\n  - type: table\n    name: V\n    sort:\n      - property: status\n");
        set_sort_direction(&mut base.views[0], 0, "DESC");
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: table\n    name: V\n    sort:\n      - property: status\n        direction: DESC\n"
        );
    }

    #[test]
    fn the_trash_button_drops_one_sort_entry_and_the_key_with_the_last() {
        let mut base = base_of(
            "views:\n  - type: table\n    name: V\n    sort:\n      - property: a\n      - property: b\n",
        );
        remove_sort(&mut base.views[0], 0);
        assert!(yaml_of(&base).contains("property: b"));
        remove_sort(&mut base.views[0], 0);
        assert_eq!(yaml_of(&base), "views:\n  - type: table\n    name: V\n");
    }

    #[test]
    fn a_group_by_added_from_the_panel_is_ascending_and_leaves_group_order_alone() {
        let mut base =
            base_of("views:\n  - type: table\n    name: V\n    groupOrder:\n      - todo\n");
        set_group_property(&mut base.views[0], "note.status");
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: table\n    name: V\n    groupBy:\n      property: note.status\n      direction: ASC\n    groupOrder:\n      - todo\n"
        );
    }

    #[test]
    fn a_limit_is_written_as_a_bare_number_and_show_all_drops_it() {
        let mut base = base_of("views:\n  - type: table\n    name: V\n");
        layout::set_limit(&mut base.views[0], parse_limit(" 103 "));
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: table\n    name: V\n    limit: 103\n"
        );
        layout::set_limit(&mut base.views[0], parse_limit(""));
        assert_eq!(yaml_of(&base), "views:\n  - type: table\n    name: V\n");
        assert_eq!(parse_limit("ten"), None);
    }

    #[test]
    fn picking_a_layout_rewrites_type_and_keeps_the_old_layouts_keys() {
        // Observed: a list view with `markers: number` and `separator: "; "`
        // became `type: kanban` with both keys still there.
        let mut base = base_of(
            "views:\n  - type: list\n    name: V\n    markers: number\n    separator: \"; \"\n",
        );
        layout::set_kind(&mut base.views[0], "kanban");
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: kanban\n    name: V\n    markers: number\n    separator: \"; \"\n"
        );
    }

    #[test]
    fn checking_a_property_appends_its_id_and_unchecking_removes_it() {
        let mut base =
            base_of("views:\n  - type: table\n    name: V\n    order:\n      - file.name\n");
        let shown = vec!["file.name".to_string()];
        toggle_property(&mut base.views[0], &shown, "note.status");
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n      - note.status\n"
        );
        toggle_property(
            &mut base.views[0],
            &["file.name".to_string(), "note.status".to_string()],
            "file.name",
        );
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: table\n    name: V\n    order:\n      - note.status\n"
        );
    }

    #[test]
    fn unchecking_a_column_of_a_view_with_no_order_writes_the_rest_out() {
        // A view with no `order` shows the implied columns, so the panel has
        // to name what is left for the uncheck to mean anything.
        let mut base = base_of("views:\n  - type: table\n    name: V\n");
        let shown = vec!["note.status".to_string(), "note.phase".to_string()];
        toggle_property(&mut base.views[0], &shown, "note.status");
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: table\n    name: V\n    order:\n      - note.phase\n"
        );
    }

    #[test]
    fn a_bare_id_already_in_order_is_the_same_property_as_its_qualified_form() {
        let mut base =
            base_of("views:\n  - type: table\n    name: V\n    order:\n      - status\n");
        // Checking it again must not double it, and unchecking must find it.
        add_property(&mut base.views[0], "note.status");
        assert_eq!(base.views[0].order, vec!["status".to_string()]);
        // Unchecking the last column is refused rather than writing an empty
        // `order`, which the serializer omits and the implied columns come
        // back from: the box would turn into "show every property".
        toggle_property(&mut base.views[0], &["status".to_string()], "note.status");
        assert_eq!(
            yaml_of(&base),
            "views:\n  - type: table\n    name: V\n    order:\n      - status\n"
        );
    }

    #[test]
    fn a_file_property_the_panel_offers_takes_obsidians_own_label() {
        let base = base_of("views:\n  - type: table\n    name: V\n");
        let labels: Vec<String> = FILE_PROPERTIES
            .iter()
            .map(|id| silica_bases::table::column(&base, id).display)
            .collect();
        assert!(labels.contains(&"file base name".to_string()));
        assert!(labels.contains(&"created time".to_string()));
        assert!(labels.contains(&"modified time".to_string()));
        assert!(labels.contains(&"folder".to_string()));
        assert!(labels.contains(&"file full name".to_string()));
        assert_eq!(FILE_PROPERTIES.len(), 12);
    }

    #[test]
    fn an_absent_filter_reads_as_an_empty_and_group_and_writes_back_as_nothing() {
        let tree = group_tree(None);
        assert_eq!(tree, Filter::And(Vec::new()));
        assert_eq!(written(tree), None);
    }

    #[test]
    fn the_conjunction_select_rewraps_a_group_and_keeps_its_rules() {
        let mut base = base_of(
            "filters:\n  and:\n    - status != \"done\"\n    - not:\n        - file.ext == \"md\"\n",
        );
        let mut tree = group_tree(base.filters.as_ref());
        set_conjunction(&mut tree, &[], "or");
        base.filters = written(tree);
        assert_eq!(
            yaml_of(&base),
            "filters:\n  or:\n    - status != \"done\"\n    - not:\n        - file.ext == \"md\"\n"
        );
    }

    #[test]
    fn a_nested_groups_own_select_is_reached_by_its_path() {
        let mut base = base_of("filters:\n  and:\n    - a\n    - not:\n        - b\n");
        let mut nodes = Vec::new();
        flatten(
            Section::AllViews,
            &group_tree(base.filters.as_ref()),
            Vec::new(),
            0,
            &mut nodes,
        );
        // root group, rule a, nested group, rule b
        assert_eq!(nodes.len(), 4);
        assert_eq!(nodes[0].kind, NodeKind::Group("and"));
        assert_eq!(nodes[1].kind, NodeKind::Rule("a".to_string()));
        assert_eq!(nodes[2].kind, NodeKind::Group("not"));
        assert_eq!(nodes[2].path, vec![1]);
        assert_eq!(nodes[3].depth, 2);
        let mut tree = group_tree(base.filters.as_ref());
        set_conjunction(&mut tree, &nodes[2].path, "or");
        base.filters = written(tree);
        assert_eq!(
            yaml_of(&base),
            "filters:\n  and:\n    - a\n    - or:\n        - b\n"
        );
    }

    #[test]
    fn a_rules_statement_is_written_verbatim() {
        // A statement's bytes survive; only their quoting changes.
        let mut base = base_of("filters:\n  and:\n    - a\n");
        let mut tree = group_tree(base.filters.as_ref());
        set_rule(&mut tree, &[0], "type + \" \"  + file.name");
        base.filters = written(tree);
        assert_eq!(
            yaml_of(&base),
            "filters:\n  and:\n    - type + \" \"  + file.name\n"
        );
    }

    #[test]
    fn the_last_rule_of_a_group_takes_the_filters_key_with_it() {
        let mut base = base_of("filters:\n  and:\n    - a\n");
        let mut tree = group_tree(base.filters.as_ref());
        remove_at(&mut tree, &[0]);
        base.filters = written(tree);
        assert_eq!(yaml_of(&base), "");
    }

    #[test]
    fn a_new_rule_lands_at_the_end_of_its_group_and_an_empty_one_writes_nothing() {
        let mut base = base_of("filters:\n  and:\n    - a\n");
        let mut tree = group_tree(base.filters.as_ref());
        add_rule(&mut tree, &[], "");
        add_rule(&mut tree, &[], "status == \"todo\"");
        base.filters = written(tree);
        assert_eq!(
            yaml_of(&base),
            "filters:\n  and:\n    - a\n    - status == \"todo\"\n"
        );
    }

    #[test]
    fn copy_to_clipboard_is_the_tsv_obsidian_put_on_the_clipboard() {
        // Observed for `bases:copy-table`, which the results panel's own row
        // produces too: one header row of display names, tab separators, no
        // trailing newline.
        use silica_bases::{Cell, CellKind, Column, ColumnSource, FileData, Row, Value};
        use std::sync::Arc;
        fn cell(text: &str) -> Cell {
            Cell {
                value: Value::Str(text.to_string()),
                error: None,
                kind: CellKind::Text,
                text: text.to_string(),
            }
        }
        fn row(a: &str, b: &str) -> Row {
            Row {
                file: Arc::new(FileData::default()),
                cells: vec![cell(a), cell(b)],
                image: None,
            }
        }
        let table = Table {
            columns: vec![
                Column {
                    id: "status".to_string(),
                    display: "status".to_string(),
                    source: ColumnSource::Property("status".to_string()),
                },
                Column {
                    id: "formula.ppu".to_string(),
                    display: "Price per unit".to_string(),
                    source: ColumnSource::Formula("ppu".to_string()),
                },
            ],
            rows: vec![row("todo", "0.02"), row("todo", "0.06")],
            groups: Vec::new(),
            limited: false,
            unknown_kind: None,
        };
        assert_eq!(
            table_tsv(&table),
            "status\tPrice per unit\ntodo\t0.02\ntodo\t0.06"
        );
        let heads_only = Table {
            rows: Vec::new(),
            ..table
        };
        assert_eq!(table_tsv(&heads_only), "status\tPrice per unit");
    }

    #[test]
    fn a_this_view_filter_lands_on_the_view_and_an_all_views_one_on_the_base() {
        // The two sections are two keys, and the panel's own writes are the
        // same shape against either.
        let mut base = base_of("views:\n  - type: table\n    name: V\n");
        let mut tree = group_tree(base.filters.as_ref());
        add_rule(&mut tree, &[], "file.ext == \"md\"");
        base.filters = written(tree);
        let mut tree = group_tree(base.views[0].filters.as_ref());
        add_rule(&mut tree, &[], "status != \"done\"");
        base.views[0].filters = written(tree);
        assert_eq!(
            yaml_of(&base),
            "filters:\n  and:\n    - file.ext == \"md\"\nviews:\n  - type: table\n    name: V\n    filters:\n      and:\n        - status != \"done\"\n"
        );
    }

    #[test]
    fn the_expression_toggles_class_is_spelled_the_way_obsidian_spells_it() {
        assert_eq!(EXPRESSION_MODE_TOGGLE, "filer-expression-mode-toggle");
    }
}
