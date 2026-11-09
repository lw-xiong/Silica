//! The Kalender pane: a monthly grid in the right dock whose days open their
//! daily note. The month shown and the selected day are view state kept on
//! [`Workspace`]; everything here is pure rendering plus the arrow/Enter
//! navigation that moves the selection and opens it.
//!
//! Klik tanggal membuka (dan jika perlu membuat) daily note untuk tanggal
//! itu, menggunakan format dan folder dari [`DailyNotesConfig`]. Tombol
//! prev/next berbulan, panah gerak pilihan, Enter membuka.

use crate::assets::icon;
use crate::theme::{Theme, ICON_SIZE};
use crate::workspace::Workspace;
use chrono::{Datelike, Duration, Local, Months, NaiveDate};
use gpui::prelude::*;
use gpui::{div, px, rgb, FontWeight, KeyDownEvent, SharedString, Window};
use silica_templates::Stamp;

/// Hari dalam seminggu, dimulai dari Minggu, singkat untuk selang baris.
const WEEKDAYS: [&str; 7] = ["Min", "Sen", "Sel", "Rab", "Kam", "Jum", "Sab"];

/// Nama bulan, bahasa Indonesia.
const MONTHS: [&str; 12] = [
    "Januari",
    "Februari",
    "Maret",
    "April",
    "Mei",
    "Juni",
    "Juli",
    "Agustus",
    "September",
    "Oktober",
    "November",
    "Desember",
];

/// Ukuran sel tanggal dan lebar kolom grid dalam piksel logis.
const CELL: f32 = 26.;

/// Jumlah baris minggu yang selalu digambar (enam minggu menangkap semua
/// bulan dalam grid 7 kolom).
const WEEKS: usize = 6;

/// Tanggal hari ini dalam waktu setempat.
fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Tanggal pertama dan terakhir dari bulan `viewed`.
fn month_bounds(viewed: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first = viewed.with_day(1).expect("first of month");
    let next = (viewed + Months::new(1))
        .with_day(1)
        .expect("first of next month");
    let last = next
        .checked_sub_signed(Duration::days(1))
        .expect("last of month");
    (first, last)
}

/// Tanggal yang akan ditampilkan sebagai terpilih: yang tersimpan kalau
/// masih ada di bulan ini, kalau tidak hari ini kalau ada di bulan ini,
/// kalau tidak tanggal pertama bulan.
fn selected_shown(viewed: NaiveDate, stored: Option<NaiveDate>) -> NaiveDate {
    let (first, last) = month_bounds(viewed);
    let in_month = |day: &NaiveDate| *day >= first && *day <= last;
    stored
        .filter(in_month)
        .unwrap_or_else(|| Some(today()).filter(in_month).unwrap_or(first))
}

/// Grid satu bulan: `offset` sel kosong di depan, kemudian tiap tanggal
/// bulan ini, diakhir dengan sel kosong pelengkap, selalu `WEEKS * 7` sel.
fn month_cells(viewed: NaiveDate) -> Vec<Option<NaiveDate>> {
    let (first, last) = month_bounds(viewed);
    let offset = first.weekday().num_days_from_sunday() as usize;
    let days = last.signed_duration_since(first).num_days() as usize + 1;
    (0..WEEKS * 7)
        .map(|cell| {
            (cell >= offset && cell < offset + days)
                .then_some(first + Duration::days((cell - offset) as i64))
        })
        .collect()
}

pub(crate) fn render_calendar_pane(
    this: &mut Workspace,
    theme: &Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let viewed = this.calendar_viewed;
    let today = today();
    let selected = selected_shown(viewed, this.calendar_selected);
    let cells = month_cells(viewed);

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
                .rounded(px(4.))
                .p(px(4.))
                .id("calendar-prev-month")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.calendar_prev_month(cx);
                    window.focus(&this.calendar_focus, cx);
                }))
                .child(icon("arrow-left", ICON_SIZE, theme.text_muted)),
        )
        .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
            "{} {}",
            MONTHS[(viewed.month() - 1) as usize],
            viewed.year()
        )))
        .child(
            div()
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
                .rounded(px(4.))
                .p(px(4.))
                .id("calendar-next-month")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.calendar_next_month(cx);
                    window.focus(&this.calendar_focus, cx);
                }))
                .child(icon("arrow-right", ICON_SIZE, theme.text_muted)),
        );

    let labels = div().flex().flex_row().children(
        WEEKDAYS
            .iter()
            .map(|label| {
                div()
                    .w(px(CELL))
                    .h(px(CELL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(*label)
            })
            .collect::<Vec<_>>(),
    );

    let weeks = div().flex().flex_col().gap(px(2.)).children(
        cells
            .chunks(7)
            .enumerate()
            .map(|(week_ix, week)| {
                div().flex().flex_row().gap(px(2.)).children(
                    week.iter()
                        .enumerate()
                        .map(|(cell_ix, day)| match day {
                            Some(day) => {
                                let day = *day;
                                let chosen = day == selected;
                                let is_today = day == today;
                                let cell = div()
                                    .w(px(CELL))
                                    .h(px(CELL))
                                    .rounded(px(4.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(13.))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.bg_alt))
                                    .child(SharedString::from(day.day().to_string()));
                                let cell = if chosen {
                                    cell.bg(theme.accent).text_color(rgb(0xFFFFFF))
                                } else if is_today {
                                    cell.font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme.accent)
                                } else {
                                    cell.text_color(theme.text_muted)
                                };
                                cell.id(format!("calendar-cell-{}-{}", viewed.month(), day.day()))
                                    .on_click(cx.listener(
                                        move |this: &mut Workspace, _, window, cx| {
                                            this.calendar_select(day, cx);
                                            let stamp = Stamp {
                                                year: day.year() as i64,
                                                month: day.month(),
                                                day: day.day(),
                                                hour: 0,
                                                minute: 0,
                                                second: 0,
                                            };
                                            this.open_daily_at(stamp, window, cx);
                                        },
                                    ))
                                    .into_any_element()
                            }
                            None => div()
                                .w(px(CELL))
                                .h(px(CELL))
                                .id(format!(
                                    "calendar-cell-empty-{}-{}",
                                    viewed.month(),
                                    week_ix * 7 + cell_ix
                                ))
                                .into_any_element(),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
    );

    div()
        .id("calendar-pane")
        .key_context("Calendar")
        .track_focus(&this.calendar_focus)
        .on_key_down(cx.listener(move |this, event, window, cx| {
            this.calendar_key_down(event, window, cx);
        }))
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .p(px(8.))
        .child(header)
        .child(labels)
        .child(weeks)
        .into_any_element()
}

impl Workspace {
    /// Bulan sebelumnya, menahan pilihan kalau masih ada di bulan baru.
    pub(crate) fn calendar_prev_month(&mut self, cx: &mut Context<Workspace>) {
        self.calendar_viewed = self
            .calendar_viewed
            .checked_sub_months(Months::new(1))
            .expect("previous month");
        self.calendar_normalize_selection(cx);
    }

    /// Bulan berikutnya, idem.
    pub(crate) fn calendar_next_month(&mut self, cx: &mut Context<Workspace>) {
        self.calendar_viewed = self
            .calendar_viewed
            .checked_add_months(Months::new(1))
            .expect("next month");
        self.calendar_normalize_selection(cx);
    }

    /// Pilih `day` dan sorotnya di grid.
    pub(crate) fn calendar_select(&mut self, day: NaiveDate, cx: &mut Context<Workspace>) {
        self.calendar_selected = Some(day);
        cx.notify();
    }

    /// Pilih ulang tanggal saat grid digambar ulang setelah pindah bulan.
    fn calendar_normalize_selection(&mut self, cx: &mut Context<Workspace>) {
        self.calendar_selected = Some(selected_shown(self.calendar_viewed, self.calendar_selected));
        cx.notify();
    }

    /// Panah di grid: -1/+1 geser satu hari, -7/+7 geser satu minggu,
    /// diklampir ke dalam bulan yang dilihat.
    fn calendar_move(&mut self, delta: isize, cx: &mut Context<Workspace>) {
        let viewed = self.calendar_viewed;
        let (first, last) = month_bounds(viewed);
        let days = last.signed_duration_since(first).num_days() as usize + 1;
        let mut index = self
            .calendar_selected
            .map(|day| day.signed_duration_since(first).num_days() as usize)
            .filter(|&ix| ix < days)
            .unwrap_or_else(|| {
                let today = today();
                let ix = today.signed_duration_since(first).num_days() as usize;
                if ix < days {
                    ix
                } else {
                    0
                }
            });
        index = (index as isize + delta).clamp(0, days as isize - 1) as usize;
        let day = first
            .checked_add_signed(Duration::days(index as i64))
            .expect("day in month");
        self.calendar_selected = Some(day);
        cx.notify();
    }

    /// Panah dan Enter di grid kalender (diaktifkan kalau grid mendapat
    /// fokus): geser pilihan, buka tanggal yang dipilih, atauEscape kembali
    /// ke note utama.
    fn calendar_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        if !self.calendar_focus.is_focused(window) {
            return;
        }
        match event.keystroke.key.as_str() {
            "ArrowLeft" => self.calendar_move(-1, cx),
            "ArrowRight" => self.calendar_move(1, cx),
            "ArrowUp" => self.calendar_move(-7, cx),
            "ArrowDown" => self.calendar_move(7, cx),
            "Enter" => {
                let Some(day) = self.calendar_selected else {
                    return;
                };
                let stamp = Stamp {
                    year: day.year() as i64,
                    month: day.month(),
                    day: day.day(),
                    hour: 0,
                    minute: 0,
                    second: 0,
                };
                self.open_daily_at(stamp, window, cx);
            }
            "Escape" => self.focus_main(window, cx),
            _ => {}
        }
    }
}
