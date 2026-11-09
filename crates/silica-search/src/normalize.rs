//! Folding akcent dan diakritik: `cafe` cocok `café`, `strasse` cocok
//! `straße`, `ø` menjadi `o`, `ß` menjadi `ss`.
//!
//! Folding memetakan satu karakter input ke nol atau lebih karakter output
//! (`ß` → `ss`, `İ` → `i` + titik di atas). Karakter di luar latin tidak
//! diubah. Setiap karakter output ingat karakter input tempatnya berasal,
//! sehingga range hasil pencarian dapat dikonversi kembali ke byte offset
//! asli — highlight tetap menunjuk ke tulisan asli, bukan ke salinan
//! terfolding.
//!
//! Folding hanya menghilangkan akcent; case ditangani oleh pemanggil
//! (`(?i)` regex, atau `char::to_lowercase`).

use std::borrow::Cow;
use std::ops::Range;

/// Teks asli beserta folding-nya, dengan peta balik ke byte offset asli.
pub struct NormText<'a> {
    /// Teks asli.
    pub orig: &'a str,
    /// Teks hasil folding (akcent dihilangkan, case utuh).
    pub text: String,
    /// Untuk setiap karakter di `text`: byte offset awal karakter asli di
    /// `orig` tempat karakter itu berasal. Beberapa karakter output
    /// (`ß` → `ss`) menunjuk ke karakter asli yang sama.
    pub map: Vec<usize>,
    /// Untuk setiap karakter di `text`: byte offset awal karakter itu di
    /// dalam `text`, untuk memotong `text` menurut indeks karakter.
    pub voff: Vec<usize>,
    /// Untuk setiap karakter di `text`: indeks karakter asli di `orig`
    /// tempat karakter itu berasal (untuk fuzzy/simple search).
    pub sources: Vec<usize>,
}

impl<'a> NormText<'a> {
    /// Folding `s`, siap dipindai oleh regex atau algoritma fuzzy.
    pub fn from(s: &'a str) -> Self {
        let (text, sources) = normalize(s);
        let mut voff = Vec::with_capacity(text.len());
        let mut at = 0;
        for c in text.chars() {
            voff.push(at);
            at += c.len_utf8();
        }
        let orig_starts: Vec<usize> = s.char_indices().map(|(b, _)| b).collect();
        let map: Vec<usize> = sources.iter().map(|&i| orig_starts[i]).collect();
        Self {
            orig: s,
            text,
            map,
            voff,
            sources,
        }
    }

    /// Byte range `[start, end)` di teks asli yang menutupi karakter
    /// terfolding `[bs, be)` (dua-duanya byte offset pada `text`).
    pub fn origin_range(&self, bs: usize, be: usize) -> Range<usize> {
        // * Ujung match selalu terletak di batas karakter teks terfolding,
        // * cari indeks karakter itu melalui peta offset byte.
        let sc = self.voff.partition_point(|&b| b < bs);
        let ec = self.voff.partition_point(|&b| b < be);
        debug_assert!(sc < ec, "empty match range should not be remapped");
        let start = self.map[sc];
        let last = self.map[ec - 1];
        let end = last + self.orig[last..].chars().next().map_or(0, |c| c.len_utf8());
        Range { start, end }
    }
}

/// Folding satu karakter ke basis latin (atau karakter itu sendiri).
fn fold(c: char) -> Cow<'static, str> {
    match c {
        // Latin-1 Supplement.
        'À'..='Å' => Cow::Borrowed("a"),
        'Ç' => Cow::Borrowed("c"),
        'È'..='Ë' => Cow::Borrowed("e"),
        'Ì'..='Ï' => Cow::Borrowed("i"),
        'Ð' => Cow::Borrowed("d"),
        'Ñ' => Cow::Borrowed("n"),
        'Ò'..='Ö' => Cow::Borrowed("o"),
        'Ø' => Cow::Borrowed("o"),
        'Ù'..='Ü' => Cow::Borrowed("u"),
        'Ý' => Cow::Borrowed("y"),
        'Þ' => Cow::Borrowed("th"),
        'ß' => Cow::Borrowed("ss"),
        'à'..='å' => Cow::Borrowed("a"),
        'ç' => Cow::Borrowed("c"),
        'è'..='ë' => Cow::Borrowed("e"),
        'ì'..='ï' => Cow::Borrowed("i"),
        'ð' => Cow::Borrowed("d"),
        'ñ' => Cow::Borrowed("n"),
        'ò'..='ö' => Cow::Borrowed("o"),
        'ø' => Cow::Borrowed("o"),
        'ù'..='ü' => Cow::Borrowed("u"),
        'ý' => Cow::Borrowed("y"),
        'þ' => Cow::Borrowed("th"),
        // Latin Extended-A.
        'Ā'..='Ą' => Cow::Borrowed("a"),
        'Ć'..='Č' => Cow::Borrowed("c"),
        'Ď' => Cow::Borrowed("d"),
        'Đ' => Cow::Borrowed("d"),
        'ď' => Cow::Borrowed("d"),
        'đ' => Cow::Borrowed("d"),
        'Ē'..='Ě' => Cow::Borrowed("e"),
        'Ĝ' => Cow::Borrowed("g"),
        'Ğ' => Cow::Borrowed("g"),
        'Ġ' => Cow::Borrowed("g"),
        'Ģ' => Cow::Borrowed("g"),
        'ĝ' => Cow::Borrowed("g"),
        'ğ' => Cow::Borrowed("g"),
        'ġ' => Cow::Borrowed("g"),
        'ģ' => Cow::Borrowed("g"),
        'Ĥ' => Cow::Borrowed("h"),
        'Ħ' => Cow::Borrowed("h"),
        'ĥ' => Cow::Borrowed("h"),
        'ħ' => Cow::Borrowed("h"),
        'Ĭ' => Cow::Borrowed("i"),
        'Į' => Cow::Borrowed("i"),
        'İ' => Cow::Borrowed("i\u{0307}"),
        'ı' => Cow::Borrowed("i"),
        'ĭ' => Cow::Borrowed("i"),
        'į' => Cow::Borrowed("i"),
        'Ĵ' => Cow::Borrowed("j"),
        'ĵ' => Cow::Borrowed("j"),
        'Ķ' => Cow::Borrowed("k"),
        'ķ' => Cow::Borrowed("k"),
        'Ĺ'..='Ł' => Cow::Borrowed("l"),
        'ł' => Cow::Borrowed("l"),
        'Ń'..='ŉ' => Cow::Borrowed("n"),
        'Ŋ' => Cow::Borrowed("n"),
        'ŋ' => Cow::Borrowed("n"),
        'Ō'..='Ő' => Cow::Borrowed("o"),
        'Œ' => Cow::Borrowed("oe"),
        'œ' => Cow::Borrowed("oe"),
        'Ǒ' => Cow::Borrowed("o"),
        'ǒ' => Cow::Borrowed("o"),
        'Ǫ' => Cow::Borrowed("o"),
        'ǫ' => Cow::Borrowed("o"),
        'Ǭ' => Cow::Borrowed("o"),
        'ǭ' => Cow::Borrowed("o"),
        'Ǿ' => Cow::Borrowed("o"),
        'ǿ' => Cow::Borrowed("o"),
        'Ŕ'..='Ř' => Cow::Borrowed("r"),
        'Ś'..='Š' => Cow::Borrowed("s"),
        'ſ' => Cow::Borrowed("s"),
        'Ţ'..='ŧ' => Cow::Borrowed("t"),
        'Ũ'..='Ų' => Cow::Borrowed("u"),
        'Ŵ' => Cow::Borrowed("w"),
        'ŵ' => Cow::Borrowed("w"),
        'Ŷ' => Cow::Borrowed("y"),
        'ÿ' => Cow::Borrowed("y"),
        'ŷ' => Cow::Borrowed("y"),
        'Ÿ' => Cow::Borrowed("y"),
        'Ź'..='Ž' => Cow::Borrowed("z"),
        other => {
            let mut buf = [0u8; 4];
            Cow::Owned(other.encode_utf8(&mut buf).to_string())
        }
    }
}

/// Folding teks penuh: setiap karakter dipetakan melalui [`fold`], dan setiap
/// karakter output menunjuk ke indeks karakter asli tempatnya berasal.
pub fn normalize(s: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(s.len());
    let mut sources = Vec::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        for m in fold(c).chars() {
            out.push(m);
            sources.push(i);
        }
    }
    (out, sources)
}
