//! Obsidian's template placeholders.
//!
//! `{{date}}`, `{{time}}` and `{{title}}`, each with an optional
//! `:format` taking a moment format. What expands and what stays literal
//! was observed rather than guessed, and the literal cases are the
//! surprising half: `{{ date }}` with spaces inside the braces does **not**
//! expand, while `{{DATE}}` does. See
//! [[Specification#Observed templates behavior, Obsidian 1.14.4]].
//!
//! Expansion is over the whole template, including inside fenced code,
//! inline code, math and wikilinks: Obsidian makes no exception for any of
//! them, so neither does this.

use crate::moment;
use crate::stamp::Stamp;

/// What the placeholders of one expansion resolve against.
#[derive(Debug, Clone)]
pub struct Context<'a> {
    pub at: Stamp,
    /// `dateFormat` from `templates.json`; `{{date}}` with no format.
    pub date_format: &'a str,
    /// `timeFormat`; `{{time}}` with no format.
    pub time_format: &'a str,
    /// What `{{title}}` reads: the basename of the note being written into.
    pub title: &'a str,
}

/// Obsidian's defaults, for a `templates.json` that does not say.
pub const DEFAULT_DATE_FORMAT: &str = "YYYY-MM-DD";
pub const DEFAULT_TIME_FORMAT: &str = "HH:mm";

/// `template` with its placeholders filled in.
pub fn expand(template: &str, cx: &Context<'_>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        // A placeholder does not cross a line: `half={{date}` on one line
        // stays literal rather than swallowing the `}}` of a later one.
        let line = after.find('\n').unwrap_or(after.len());
        match after[..line].find("}}") {
            Some(close) => match placeholder(&after[..close], cx) {
                Some(text) => {
                    out.push_str(&text);
                    rest = &after[close + 2..];
                }
                None => {
                    out.push_str("{{");
                    rest = after;
                }
            },
            // No closer on this line: the braces are text like any other.
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// What one `{{...}}` body resolves to, or `None` when it is not a
/// placeholder at all and stays as it was written.
fn placeholder(body: &str, cx: &Context<'_>) -> Option<String> {
    let (name, format) = match body.find(':') {
        Some(ix) => (&body[..ix], Some(&body[ix + 1..])),
        None => (body, None),
    };
    // The name is matched without regard to case, but takes no whitespace
    // at all: `{{DATE}}` expands and `{{ date }}` does not. The format may
    // hold spaces, which is how `{{date:YYYY[-]MM-DD ddd}}` works.
    if name.chars().any(char::is_whitespace) {
        return None;
    }
    let name = name.to_ascii_lowercase();
    // An empty format on the placeholder falls back to the setting, and an
    // empty setting to Obsidian's own default rather than to moment's: a
    // vault with `"dateFormat": ""` still writes `2026-10-18`.
    let fallback = |format: Option<&str>, setting: &str, default: &'static str| {
        non_empty(format)
            .or(non_empty(Some(setting)))
            .unwrap_or(default)
            .to_string()
    };
    Some(match name.as_str() {
        "date" => moment::format(
            &fallback(format, cx.date_format, DEFAULT_DATE_FORMAT),
            &cx.at,
        ),
        "time" => moment::format(
            &fallback(format, cx.time_format, DEFAULT_TIME_FORMAT),
            &cx.at,
        ),
        "title" => cx.title.to_string(),
        _ => return None,
    })
}

fn non_empty(format: Option<&str>) -> Option<&str> {
    format.filter(|f| !f.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx() -> Context<'static> {
        Context {
            at: Stamp {
                year: 2026,
                month: 10,
                day: 18,
                hour: 16,
                minute: 58,
                second: 13,
            },
            date_format: DEFAULT_DATE_FORMAT,
            time_format: DEFAULT_TIME_FORMAT,
            title: "P1",
        }
    }

    /// The whole template the probe expanded, and the bytes Obsidian wrote
    /// for it. A line that stayed literal is as much of the observation as
    /// one that expanded.
    #[test]
    fn the_probe_template_expands_exactly_as_obsidian_expanded_it() {
        let template = "d={{date}}\n\
             t={{time}}\n\
             ti={{title}}\n\
             fmt={{date:YYYY[-]MM-DD ddd}}\n\
             tfmt={{time:HH-mm-ss}}\n\
             colon={{date:HH:mm}}\n\
             UP={{DATE}}\n\
             Cap={{Date}}\n\
             TUP={{TITLE}}\n\
             spaced={{ date }}\n\
             spacedfmt={{ date : YYYY }}\n\
             unknown={{nosuch}}\n\
             empty={{}}\n\
             half={{date}\n\
             single={date}\n\
             emptyfmt={{date:}}\n\
             twice={{date}} x {{date}} x {{title}}\n\
             inline=`{{date}}` ``{{title}}``\n";
        let want = "d=2026-10-18\n\
             t=16:58\n\
             ti=P1\n\
             fmt=2026-10-18 Sun\n\
             tfmt=16-58-13\n\
             colon=16:58\n\
             UP=2026-10-18\n\
             Cap=2026-10-18\n\
             TUP=P1\n\
             spaced={{ date }}\n\
             spacedfmt={{ date : YYYY }}\n\
             unknown={{nosuch}}\n\
             empty={{}}\n\
             half={{date}\n\
             single={date}\n\
             emptyfmt=2026-10-18\n\
             twice=2026-10-18 x 2026-10-18 x P1\n\
             inline=`2026-10-18` ``P1``\n";
        assert_eq!(expand(template, &cx()), want);
    }

    #[test]
    fn nothing_is_exempt_from_expansion() {
        // Obsidian expands inside fenced code, indented code, tilde fences,
        // math and wikilinks alike, so this does too.
        let template = "```\nfenced {{date}} {{title}}\n```\n\
             \x20   indented {{date}}\n\
             ~~~\ntilde {{date}}\n~~~\n\
             math $x_{{date}}$\n\
             link [[{{title}}]] and ![[{{title}}]]\n";
        let want = "```\nfenced 2026-10-18 P1\n```\n\
             \x20   indented 2026-10-18\n\
             ~~~\ntilde 2026-10-18\n~~~\n\
             math $x_2026-10-18$\n\
             link [[P1]] and ![[P1]]\n";
        assert_eq!(expand(template, &cx()), want);
    }

    #[test]
    fn the_settings_decide_the_bare_forms() {
        let mut c = cx();
        c.date_format = "DD/MM/YYYY";
        c.time_format = "h:mm a";
        assert_eq!(expand("{{date}} at {{time}}", &c), "18/10/2026 at 4:58 pm");
        // A format on the placeholder wins over the setting.
        assert_eq!(expand("{{date:YYYY}}", &c), "2026");
    }

    #[test]
    fn an_empty_setting_falls_back_to_obsidian_s_own_default() {
        // A vault whose templates.json holds `"dateFormat": ""` still gets
        // `2026-10-18` and `16:58`, not the empty string a bare moment
        // format would give.
        let mut c = cx();
        c.date_format = "";
        c.time_format = "";
        assert_eq!(expand("{{date}} {{time}}", &c), "2026-10-18 16:58");
        assert_eq!(expand("{{date:}}", &c), "2026-10-18");
        // A format on the placeholder still wins.
        assert_eq!(expand("{{date:YYYY}}", &c), "2026");
    }

    #[test]
    fn a_template_with_no_placeholders_is_its_own_bytes() {
        let plain = "# Heading\n\nBody with { braces } and }} stray.\n";
        assert_eq!(expand(plain, &cx()), plain);
        assert_eq!(expand("", &cx()), "");
    }
}
