//! The sixteen globals. The completion list is the whole set, and a name
//! outside it fails with `Cannot find function "N"` — **no type suffix**,
//! unlike a method.
//!
//! | Global | What it does |
//! | --- | --- |
//! | `now()` | the current moment, a DateTime |
//! | `today()` | the current date, a Date |
//! | `date(input)` | parses a timestamp or date object; `date("not a date")` is null |
//! | `duration(input: String)` | parses a string as a duration |
//! | `number(…)` | the value as a number; a bad input gives `Unable to convert "RegExp" to a number.` |
//! | `list(element)` | wraps the element in a list, or returns it unchanged if it is one; **exactly one argument** |
//! | `link(path, display: Any?)` | parses a string path and returns a Link |
//! | `file(input: String)` | locates a file for a path or link |
//! | `icon(input: String)` | a Lucide icon name, rendered as the icon |
//! | `image(input: String)` | an image object the view renders |
//! | `html(input: String)` | a string that renders as HTML |
//! | `escapeHTML(input: String)` | escapes for inclusion in HTML |
//! | `if(condition, true_result: Any, false_result: Any?)` | the third argument is optional |
//! | `min(…)`, `max(…)` | the smallest/largest of the numbers given; with no arguments, null |
//! | `random()` | a random number between 0 and 1 |
//!
//! Names the table does **not** know, each tried: `string`, `bool`, `choice`,
//! `ifs`, `switch`, `coalesce`, `sum`, `average`, `mean`, `median`, `round`,
//! `floor`, `ceil`, `abs`, `sqrt`, `pow`, `log`, `trunc`, `sign`, `regexp`,
//! `object`, `type`, `empty`, `notEmpty`, `concat`, `join`, `contains`,
//! `length`, `count`, `first`, `last`, `reverse`, `sort`, `unique`, `filter`,
//! `map`, `reduce`, `markdown`, `embed`, `tag`, `prop`, `property`, `note`,
//! `formula`, `parseInt`, `asDate`, `asNumber`, `asString`, `asLink`,
//! `asFile`, `dateDiff`. `note`, `formula`, `file` and `this` exist, but as
//! idents rather than functions.
//!
//! **The order the three refusals come in** is observed, and it is not the
//! obvious one: a global checks its minimum arity, then the declared types of
//! its parameters, then its maximum arity. `date(/zz/, /zz/, /zz/, /zz/)` is
//! `Type error in "date", parameter "input". Expected String, given RegExp.`
//! rather than *too many arguments*, while `number(/zz/, /zz/)`, whose one
//! parameter is untyped, **is** *too many arguments* rather than
//! `Unable to convert "RegExp" to a number.`
//!
//! Two parameter names are verbatim and easy to get wrong: `link`'s first
//! parameter is named **`self`** (`Type error in "link", parameter "self".`),
//! not `path`, and every other String parameter here is named `input`.
//!
//! Names are case-insensitive — `NOW()`, `Now()`, `nOw()` and `ToDay()` all
//! answer — while the refusal quotes the name as it was written:
//! `NoSuchFunction()` → `Cannot find function "NoSuchFunction"`.

use crate::eval::{EvalError, EvalResult, Evaluator};
use crate::expr::Expr;
use crate::value::{numeric, str_as_number, LinkValue, RenderKind, RenderedValue, Value};

use super::{not_enough, too_many, want_string};

pub fn call(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    match name.to_ascii_lowercase().as_str() {
        "now" => {
            no_args(name, args)?;
            Ok(Value::Date(super::time::now_of(ev.context().now(), true)))
        }
        "today" => {
            no_args(name, args)?;
            Ok(Value::Date(super::time::now_of(ev.context().now(), false)))
        }
        "random" => {
            no_args(name, args)?;
            Ok(Value::Number(ev.context().random()))
        }
        "date" => date(ev, name, args),
        "duration" => duration(ev, name, args),
        "number" => number(ev, name, args),
        "list" => list(ev, name, args),
        "link" => link(ev, name, args),
        "file" => file(ev, name, args),
        "icon" => rendered(ev, name, args, RenderKind::Icon),
        "image" => rendered(ev, name, args, RenderKind::Image),
        "html" => rendered(ev, name, args, RenderKind::Html),
        "escapehtml" => escape_html(ev, name, args),
        "if" => if_(ev, name, args),
        "min" => min_max(ev, args, false),
        "max" => min_max(ev, args, true),
        _ => Err(unknown_global(name)),
    }
}

/// A global that does not exist. Obsidian names no type here.
pub fn unknown_global(name: &str) -> EvalError {
    EvalError::new(format!("Cannot find function \"{name}\""))
}

/// `now()`, `today()` and `random()` were each only ever called with no
/// arguments, so refusing extras is `unprobed`; a signature with no parameters
/// is the reading.
fn no_args(name: &str, args: &[Expr]) -> Result<(), EvalError> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(too_many(name))
    }
}

/// The minimum-arity half of the observed order: refuse a short call before
/// anything is evaluated, then hand the evaluated arguments back so the
/// parameter types can be checked before the maximum arity is.
fn args_from(
    ev: &mut Evaluator<'_>,
    name: &str,
    args: &[Expr],
    min: usize,
) -> Result<Vec<Value>, EvalError> {
    if args.len() < min {
        return Err(not_enough(name));
    }
    ev.eval_args(args)
}

/// `date(input)`. A Date is handed back as it came — `date(when)` on a Date
/// property answers that date — a String is parsed, and anything else is
/// `Expected String`, which is the message even though a Date is accepted.
/// `date("not a date")` is null, not a refusal.
fn date(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    let vals = args_from(ev, name, args, 1)?;
    let out = match &vals[0] {
        // Null absorbs, as it does everywhere else in the language. `unprobed`:
        // only a RegExp and a Number were ever refused here.
        Value::Null => Value::Null,
        Value::Date(d) => Value::Date(*d),
        other => {
            let input = want_string(name, "input", other)?;
            match super::time::parse_date(&input) {
                Some(d) => Value::Date(d),
                None => Value::Null,
            }
        }
    };
    if args.len() > 1 {
        return Err(too_many(name));
    }
    Ok(out)
}

/// `duration(input: String)`. `duration(1, "week")` is
/// `Type error in "duration", parameter "input". Expected String, given
/// Number.`, so the unit is never a second argument.
fn duration(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    let vals = args_from(ev, name, args, 1)?;
    let out = match &vals[0] {
        Value::Null => Value::Null,
        // `unprobed`, by analogy with `date(when)`.
        Value::Duration(d) => Value::Duration(*d),
        other => {
            let input = want_string(name, "input", other)?;
            match super::time::parse_duration(&input) {
                Some(d) => Value::Duration(d),
                None => Value::Null,
            }
        }
    };
    if args.len() > 1 {
        return Err(too_many(name));
    }
    Ok(out)
}

/// `number(…)`. Its parameter is untyped, so a second argument is *too many
/// arguments* and a bad first one is the conversion refusal, whose text is
/// observed verbatim: `Unable to convert "RegExp" to a number.`
///
/// `unprobed`: only `number("42")` → 42 and `number(/zz/)` → the refusal were
/// seen. The message names a *type*, not a value, so the reading here is that
/// conversion is decided by type — a String that is not a number converts to
/// NaN rather than refusing, and a null stays null.
fn number(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    let vals = args_from(ev, name, args, 1)?;
    if args.len() > 1 {
        return Err(too_many(name));
    }
    match &vals[0] {
        Value::Null => Ok(Value::Null),
        Value::Number(n) => Ok(Value::Number(*n)),
        Value::Bool(b) => Ok(Value::Number(if *b { 1.0 } else { 0.0 })),
        // A Link is a String receiver everywhere else, so its text converts.
        other => match other.as_text() {
            Some(s) => Ok(Value::Number(str_as_number(s).unwrap_or(f64::NAN))),
            None => Err(EvalError::new(format!(
                "Unable to convert \"{}\" to a number.",
                other.ty().as_str()
            ))),
        },
    }
}

/// `list(element)`. Exactly one argument: `list(1, 2)` is *too many
/// arguments*, and `list("a")` is a one-element List.
fn list(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    let vals = args_from(ev, name, args, 1)?;
    if args.len() > 1 {
        return Err(too_many(name));
    }
    Ok(match &vals[0] {
        // Null absorbs rather than becoming a list of one null. `unprobed`.
        Value::Null => Value::Null,
        // "If the element is a list, return it unmodified."
        already @ (Value::List(_) | Value::Tags(_)) => already.clone(),
        other => Value::List(vec![other.clone()]),
    })
}

/// `link(path, display: Any?)`. The refused parameter is named `self`:
/// `Type error in "link", parameter "self". Expected String, given RegExp.`
///
/// The probe could not serialize a Link, so what is pinned down is that
/// `link("Pumice")` and `link("Pumice", "alias")` both carry `Pumice` as the
/// link's own data and tag as `lucide-link`. What the display argument does to
/// the rendering is `unprobed`; it is the alias here, which is what
/// `Link.toString()` → `[[Pumice|alias]]` writes.
fn link(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    let vals = args_from(ev, name, args, 1)?;
    if vals[0].is_null() {
        if args.len() > 2 {
            return Err(too_many(name));
        }
        return Ok(Value::Null);
    }
    let path = want_string(name, "self", &vals[0])?;
    if args.len() > 2 {
        return Err(too_many(name));
    }
    let display = match vals.get(1) {
        Some(v) if !v.is_null() => Some(v.to_string()),
        _ => None,
    };
    Ok(Value::Link(LinkValue { path, display }))
}

/// `file(input: String)` — "attempt to locate a file for a given path or
/// link". A path that resolves to nothing is null; `unprobed`, but it is what
/// *attempt* and the language's null absorption both read as.
fn file(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    let vals = args_from(ev, name, args, 1)?;
    let wanted = match &vals[0] {
        Value::Null => None,
        // `unprobed`: a File handed back as it came.
        Value::File(f) => {
            if args.len() > 1 {
                return Err(too_many(name));
            }
            return Ok(Value::File(f.clone()));
        }
        Value::Link(l) => Some(l.path.clone()),
        other => Some(want_string(name, "input", other)?),
    };
    if args.len() > 1 {
        return Err(too_many(name));
    }
    Ok(wanted
        .and_then(|path| ev.context().lookup_file(&path))
        .map(Value::File)
        .unwrap_or(Value::Null))
}

/// `icon(input)`, `image(input)` and `html(input)`: a thing the view renders
/// rather than a value evaluation looks into. All three tag as `lucide-image`
/// and carry the string they were given, and all three refuse a RegExp with
/// `parameter "input"`.
fn rendered(ev: &mut Evaluator<'_>, name: &str, args: &[Expr], kind: RenderKind) -> EvalResult {
    let vals = args_from(ev, name, args, 1)?;
    if vals[0].is_null() {
        if args.len() > 1 {
            return Err(too_many(name));
        }
        return Ok(Value::Null);
    }
    let source = want_string(name, "input", &vals[0])?;
    if args.len() > 1 {
        return Err(too_many(name));
    }
    Ok(Value::Rendered(RenderedValue { kind, source }))
}

/// `escapeHTML(input)` — "escapes special characters in a string to make it
/// safe for inclusion in HTML".
///
/// `unprobed`: the completion list is the only place this global was ever
/// seen, so neither the parameter's name (`input`, as every other String
/// parameter here) nor the set of characters is observed. The five of
/// `& < > " '` are the set that makes a string safe in both text and attribute
/// position.
fn escape_html(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    let vals = args_from(ev, name, args, 1)?;
    if vals[0].is_null() {
        if args.len() > 1 {
            return Err(too_many(name));
        }
        return Ok(Value::Null);
    }
    let input = want_string(name, "input", &vals[0])?;
    if args.len() > 1 {
        return Err(too_many(name));
    }
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    Ok(Value::Str(out))
}

/// `if(condition, true_result, false_result?)`.
///
/// The branch not taken is **never evaluated**: the arguments stay expressions
/// here rather than going through [`Evaluator::eval_args`]. `if(/zz/)` is
/// *not enough arguments* and `if(/zz/, /zz/)` answers the RegExp, so the
/// condition is coerced through truthiness rather than having to be a Boolean,
/// and the third argument really is optional — `if(true, 1)` is 1. What a
/// false condition with no third argument answers is `unprobed`; null is the
/// reading, since that is what every other missing value in the language is.
fn if_(ev: &mut Evaluator<'_>, name: &str, args: &[Expr]) -> EvalResult {
    if args.len() < 2 {
        return Err(not_enough(name));
    }
    if args.len() > 3 {
        return Err(too_many(name));
    }
    if ev.eval(&args[0])?.is_truthy() {
        ev.eval(&args[1])
    } else {
        match args.get(2) {
            Some(otherwise) => ev.eval(otherwise),
            None => Ok(Value::Null),
        }
    }
}

/// `min(…)` and `max(…)` — "the smallest/largest of all the provided
/// numbers". With no arguments at all both answer null.
///
/// `unprobed` past `min(1, 2)` → 1, `max(1, 2)` → 2 and the empty call: what a
/// non-number argument does is not observed, and Obsidian names no parameter
/// to refuse, so a value with no number in it is passed over rather than
/// refused. The coercion is the one the comparison operators use, so `"3"`
/// counts as 3 the way `"3" > 2` does.
fn min_max(ev: &mut Evaluator<'_>, args: &[Expr], want_max: bool) -> EvalResult {
    let vals = ev.eval_args(args)?;
    let mut best: Option<f64> = None;
    for v in &vals {
        let Some(n) = numeric(v) else { continue };
        best = Some(match best {
            None => n,
            Some(b) if b.is_nan() || n.is_nan() => f64::NAN,
            Some(b) if (n > b) == want_max && n != b => n,
            Some(b) => b,
        });
    }
    Ok(best.map(Value::Number).unwrap_or(Value::Null))
}
