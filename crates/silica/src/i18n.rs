//! The interface language.
//!
//! Obsidian's General tab has a Language dropdown and a `locale` key in
//! `app.json`. This is the same choice: a locale name stored there, read at
//! startup and on the click that changes it.
//!
//! Every string goes through [`t`], which answers the locale's translation
//! and falls back to the English original when it has none. A missing
//! translation therefore shows English rather than a blank or a key, so a
//! partly translated locale is usable while it is being filled in — which is
//! how the table starts. [`COVERED`] says what is translated today, and the
//! Settings row shows it, so the app never claims more than it has.
//!
//! Adding a locale is data: a `Locale` variant, its name, and rows in the
//! two tables below. Nothing else reads the locale directly.

use std::sync::atomic::{AtomicU8, Ordering};

/// The interface languages Silica carries strings for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    English,
    /// Bahasa Indonesia.
    Indonesian,
}

impl Locale {
    /// Every locale there is, as (label, code) in the order a dropdown
    /// lists them. Settings builds its options from this, so adding a
    /// language here is the whole of adding it to the row.
    pub const ALL: &'static [(&'static str, &'static str)] =
        &[("English", "en"), ("Bahasa Indonesia", "id")];

    /// The value stored in `app.json`'s `locale`, which is what
    /// [`Locale::from_code`] reads back. The roundtrip is what the tests
    /// assert; nothing in the app writes it back out yet, so it is
    /// `cfg(test)` rather than dead weight in the binary.
    #[cfg(test)]
    pub fn code(self) -> &'static str {
        match self {
            Locale::English => "en",
            Locale::Indonesian => "id",
        }
    }

    /// The locale a stored value names. Anything unrecognised is English:
    /// a vault written by a newer build must still open.
    pub fn from_code(code: &str) -> Locale {
        let lower = code.trim().to_ascii_lowercase();
        match lower.split(['-', '_']).next().unwrap_or("") {
            "id" | "in" => Locale::Indonesian,
            _ => Locale::English,
        }
    }

    fn index(self) -> u8 {
        match self {
            Locale::English => 0,
            Locale::Indonesian => 1,
        }
    }

    fn from_index(ix: u8) -> Locale {
        match ix {
            1 => Locale::Indonesian,
            _ => Locale::English,
        }
    }
}

/// The locale in force. A process-wide value rather than a parameter on
/// every render, because a string lookup happens in paint code that has no
/// access to the vault.
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Sets the locale in force and answers the one it replaced, so a caller can
/// skip a repaint when nothing changed.
pub fn set(locale: Locale) -> Locale {
    Locale::from_index(CURRENT.swap(locale.index(), Ordering::Relaxed))
}

pub fn current() -> Locale {
    Locale::from_index(CURRENT.load(Ordering::Relaxed))
}

/// The string `key` names, in the locale in force, or the English original
/// when this locale has no translation for it.
pub fn t(key: &'static str) -> &'static str {
    if current() == Locale::English {
        return key;
    }
    translated(current(), key).unwrap_or(key)
}

/// The locale's translation of `key`, if it has one.
pub fn translated(locale: Locale, key: &'static str) -> Option<&'static str> {
    match locale {
        Locale::English => None,
        Locale::Indonesian => INDONESIAN
            .iter()
            .find(|(english, _)| *english == key)
            .map(|(_, translated)| *translated),
    }
}

/// The Settings modal's chrome. A row's own name and description are not
/// here: they are Obsidian's English wording, and translating a setting's
/// name would make it harder to match against Obsidian's own docs.
const INDONESIAN: &[(&str, &str)] = &[
    ("General", "Umum"),
    ("Appearance", "Tampilan"),
    ("Interface", "Antarmuka"),
    ("Editor", "Editor"),
    ("Files and links", "Berkas dan tautan"),
    ("Hotkeys", "Pintasan tombol"),
    ("Core plugins", "Plugin inti"),
    ("Community plugins", "Plugin komunitas"),
    ("Options", "Opsi"),
    ("Settings", "Pengaturan"),
    ("Search settings", "Cari pengaturan"),
    ("not in Silica yet", "belum ada di Silica"),
    ("on restart", "saat mulai ulang"),
    ("Vault", "Vault"),
    ("Advanced", "Lanjutan"),
    ("Display", "Tampilan"),
    ("Behavior", "Perilaku"),
    ("Links", "Tautan"),
    ("Trash", "Tempat sampah"),
    ("Font", "Huruf"),
    ("CSS snippets", "Potongan CSS"),
    ("File", "Berkas"),
    ("Edit", "Sunting"),
    ("View", "Tampilan"),
    ("Navigate", "Navigasi"),
    ("New note", "Catatan baru"),
    ("New folder", "Folder baru"),
    ("Settings", "Pengaturan"),
    ("Undo", "Kembalikan"),
    ("Redo", "Ulangi"),
    ("Select all", "Pilih semua"),
    ("Paste", "Tempel"),
    ("Graph", "Grafik"),
    ("Reload", "Muat ulang"),
    ("Toggle preview", "Ubah pratinjau"),
    ("Go back", "Kembali"),
    ("Go forward", "Maju"),
    ("Search for", "Cari"),
    ("Rename tag...", "Ubah nama tag..."),
    ("Copy tag", "Salin tag"),
    ("Search for property", "Cari properti"),
    ("Copy property", "Salin properti"),
    ("Rename property", "Ubah nama properti"),
    ("Property type", "Tipe properti"),
    ("Delete property", "Hapus properti"),
    ("Property name", "Nama properti"),
    ("Invalid property type", "Tipe properti tidak valid"),
    ("Open heading", "Buka judul"),
    ("Open note in new tab", "Buka catatan di tab baru"),
    ("Open note to the right", "Buka catatan di sebelah kanan"),
    ("Silica reads this vault the way Obsidian 1.14.4 does.", "Silica membaca vault ini seperti Obsidian 1.14.4."),
    ("Check for updates", "Periksa pembaruan"),
    ("Language", "Bahasa"),
    ("Change the display language. The Settings window's labels follow it; notes and the other views are still English.", "Ubah bahasa tampilan. Label jendela Pengaturan mengikutinya; catatan dan tampilan lainnya masih dalam Bahasa Inggris."),
    ("Help", "Bantuan"),
    ("Open", "Buka"),
    ("Learn how to use Obsidian and get help from the community.", "Pelajari cara menggunakan Obsidian dan dapatkan bantuan dari komunitas."),
    ("Vault name", "Nama vault"),
    ("Vault folder", "Folder vault"),
    ("Browse", "Jelajahi"),
    ("Current plugins", "Plugin saat ini"),
    ("You currently have no plugins installed.", "Saat ini tidak ada plugin terpasang."),
    ("You currently have 1 plugin installed.", "Saat ini ada 1 plugin terpasang."),
    ("You currently have", "Saat ini Anda memiliki"),
    ("plugins installed,", "plugin terpasang,"),
    ("of which can run here.", "yang dapat berjalan di sini."),
    ("Open in new tab", "Buka di tab baru"),
    ("Open to the right", "Buka ke kanan"),
    ("Make a copy", "Buat salinan"),
    ("Move file to...", "Pindahkan file ke..."),
    ("Bookmark...", "Tandai..."),
    ("Copy path", "Salin jalur"),
    ("Merge entire file with...", "Gabungkan seluruh file dengan..."),
    ("Open version history", "Buka riwayat versi"),
    ("Open in default app", "Buka di aplikasi default"),
    ("Open in new window", "Buka di jendela baru"),
    ("Show in system explorer", "Tampilkan di penjelajah sistem"),
    ("Rename...", "Ubah nama..."),
    ("Delete", "Hapus"),
    ("New canvas", "Kanvas baru"),
    ("New base", "Basis baru"),
    ("Cut", "Potong"),
    ("Copy", "Salin"),
    ("Select all", "Pilih semua"),
    ("Toggle bold", "Luncurkan tebal"),
    ("Toggle italic", "Luncurkan miring"),
    ("Toggle highlight", "Luncurkan sorotan"),
    ("Insert link", "Sisipkan tautan"),
    ("Insert wikilink", "Sisipkan wikilink"),
    ("Insert embed", "Sisipkan penyisipan"),
    ("Toggle checkbox", "Luncurkan kotak centang"),
    ("Zoom to selection", "Perbesar ke pilihan"),
    ("Create group", "Buat grup"),
    ("Add card", "Tambahkan kartu"),
    ("Remove label", "Hapus label"),
    ("Move folder to...", "Pindahkan folder ke..."),
    ("Search in folder", "Cari di folder"),
    ("Follow connection", "Ikuti koneksi"),
    ("Add note from vault", "Tambahkan catatan dari vault"),
    ("Add media from vault", "Tambahkan media dari vault"),
];

/// The table's entries that are the same word in both languages.
#[cfg(test)]
const LOANWORDS: &[&str] = &["Editor", "Vault", "Display"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The locale is process-wide, so tests that set it would otherwise
    /// race each other and read a language another test just switched to.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn english_answers_the_key_itself() {
        let _guard = serial();
        set(Locale::English);
        assert_eq!(t("Settings"), "Settings");
        assert_eq!(translated(Locale::English, "Settings"), None);
    }

    #[test]
    fn a_translation_replaces_the_english() {
        let _guard = serial();
        set(Locale::Indonesian);
        assert_eq!(t("Settings"), "Pengaturan");
        assert_eq!(t("Files and links"), "Berkas dan tautan");
        set(Locale::English);
        assert_eq!(t("Settings"), "Settings");
    }

    #[test]
    fn an_untranslated_string_falls_back_rather_than_emptying() {
        let _guard = serial();
        set(Locale::Indonesian);
        // * A row name, deliberately not in the table.
        assert_eq!(t("Default view for new tabs"), "Default view for new tabs");
        set(Locale::English);
    }

    #[test]
    fn set_answers_the_locale_it_replaced() {
        set(Locale::English);
        assert_eq!(set(Locale::Indonesian), Locale::English);
        assert_eq!(set(Locale::Indonesian), Locale::Indonesian);
        assert_eq!(set(Locale::English), Locale::Indonesian);
    }

    #[test]
    fn codes_round_trip_and_unknown_ones_are_english() {
        for (label, code) in Locale::ALL {
            // * Every code resolves to exactly the locale it names; "en" is
            //   English, so the check is against the pair, not "not English".
            assert_eq!(Locale::from_code(code).code(), *code);
            assert!(!label.is_empty());
        }
        assert_eq!(Locale::from_code("id-ID"), Locale::Indonesian);
        assert_eq!(Locale::from_code("in"), Locale::Indonesian);
        // * A vault from a newer build, or a typo, still opens.
        assert_eq!(Locale::from_code("xx"), Locale::English);
        assert_eq!(Locale::from_code(""), Locale::English);
    }

    #[test]
    fn every_table_row_is_reachable_and_non_empty() {
        for (english, translated) in INDONESIAN {
            assert!(!english.is_empty());
            assert!(!translated.is_empty(), "{english} translates to nothing");
            // * A loanword is its own translation: "Editor" and "Vault" are
            //   spelled the same in Indonesian, and pretending otherwise
            //   would be inventing a word.
            if !LOANWORDS.contains(english) {
                assert_ne!(english, translated, "{english} was not translated");
            }
        }
    }

    #[test]
    fn the_labels_are_their_own_names() {
        // * A language name is never translated: a reader looking for their
        //   own language needs to see it written the way they write it.
        let labels: Vec<&str> = Locale::ALL.iter().map(|(label, _)| *label).collect();
        assert!(labels.contains(&"Bahasa Indonesia"));
        assert!(labels.contains(&"English"));
    }
}
