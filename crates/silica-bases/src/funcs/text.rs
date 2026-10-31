//! String and RegExp.
//!
//! Observed set, from the completion list and the brute-force pass:
//!
//! | Receiver | Functions | Properties |
//! | --- | --- | --- |
//! | String | `contains(value: String)`, `containsAll(values: String)`, `containsAny(values: String)`, `endsWith(query: String)`, `isEmpty()`, `lower()`, `repeat(count: Number)`, `replace(pattern: String,RegExp, replacement: String)`, `reverse()`, `slice(start: Number, end: Number?)`, `split(separator: String,RegExp, n: Number?)`, `startsWith(query: String)`, `title()`, `trim()` | `length` |
//! | RegExp | `matches(value: String)` | — |
//!
//! There is **no `upper()`**: both readings agree that nothing uppercases a
//! string. `length` is handled by the dispatcher.
//!
//! A **Link is a String receiver** — `linkish.lower()` is `pumice` and
//! `linkish.length` is 6 — so everything here works off [`Value::as_text`]
//! rather than matching [`Value::Str`], and a Link argument is accepted
//! wherever a String one is. A **RegExp is not**: its completion list is
//! `isTruthy`, `isType`, `matches` and `toString` and nothing else, so every
//! other name on a RegExp is refused here.
//!
//! **The order of a refusal** is observed and is not the obvious one. A call
//! is checked in three steps — too few arguments, then the declared
//! parameters' types, then too many arguments — because
//! `"s".contains(/zz/, /zz/, /zz/)` answers
//! `Type error in "contains", parameter "value". Expected String, given
//! RegExp.` rather than `too many arguments.`, while `number(/zz/, /zz/)`,
//! whose refusal comes from the body rather than from a parameter, does answer
//! `too many arguments.`
//!
//! Rust's `regex` is not JavaScript's: it has no backtracking, so a pattern
//! with a look-around or a back-reference will not compile. A pattern that
//! does not compile is treated here as one that **matches nothing** —
//! `matches()` is false, `replace()` returns the subject unchanged and
//! `split()` returns the whole subject as one element — rather than raising an
//! error text Obsidian never shows in a cell. `unprobed`, and a divergence:
//! Obsidian would have matched.

use regex::{NoExpand, Regex, RegexBuilder};

use crate::eval::{EvalError, EvalResult, Evaluator};
use crate::expr::Expr;
use crate::value::{RegexpValue, Type, Value};

use super::{not_enough, too_many, type_error, unknown_function, want_number, want_string};

/// The longest string `repeat()` will build. A memory guard with no counterpart
/// in Obsidian, which builds the string until V8 refuses; `unprobed`.
const MAX_REPEAT_BYTES: usize = 1 << 20;

pub fn call(ev: &mut Evaluator<'_>, subject: &Value, name: &str, args: &[Expr]) -> EvalResult {
    let lower = name.to_ascii_lowercase();

    // A RegExp carries one function of its own and is *not* a String receiver.
    if let Value::Regexp(re) = subject {
        if lower == "matches" {
            at_least(name, args, 1)?;
            let vals = ev.eval_args(args)?;
            let value = want_string(name, "value", &vals[0])?;
            at_most(name, args, 1)?;
            return Ok(Value::Bool(compile(re).is_some_and(|r| r.is_match(&value))));
        }
        return Err(unknown_function(name, Type::RegExp));
    }

    let Some(text) = subject.as_text() else {
        return Err(unknown_function(name, subject.ty()));
    };

    match lower.as_str() {
        // `contains(value: String)` — "Returns true if the string contains
        // `value`." Values are case-sensitive; only names are not.
        "contains" => {
            at_least(name, args, 1)?;
            let vals = ev.eval_args(args)?;
            let needle = want_string(name, "value", &vals[0])?;
            at_most(name, args, 1)?;
            Ok(Value::Bool(text.contains(&needle)))
        }

        // `containsAll(values: String)` and `containsAny(values: String)` are
        // **variadic**: observed, because with no arguments at all they answer
        // true and false respectively — the vacuous answers, which a fixed
        // arity of one would have refused as "not enough arguments.".
        "containsall" => {
            let vals = ev.eval_args(args)?;
            let needles = strings(name, "values", &vals)?;
            Ok(Value::Bool(needles.iter().all(|n| text.contains(n))))
        }
        "containsany" => {
            let vals = ev.eval_args(args)?;
            let needles = strings(name, "values", &vals)?;
            Ok(Value::Bool(needles.iter().any(|n| text.contains(n))))
        }

        "endswith" => {
            at_least(name, args, 1)?;
            let vals = ev.eval_args(args)?;
            let query = want_string(name, "query", &vals[0])?;
            at_most(name, args, 1)?;
            Ok(Value::Bool(text.ends_with(&query)))
        }
        "startswith" => {
            at_least(name, args, 1)?;
            let vals = ev.eval_args(args)?;
            let query = want_string(name, "query", &vals[0])?;
            at_most(name, args, 1)?;
            Ok(Value::Bool(text.starts_with(&query)))
        }

        // "Returns true if the string has no characters, or is not present."
        // The "not present" half is the dispatcher's: null answers true.
        "isempty" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Bool(text.is_empty()))
        }

        "lower" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Str(text.to_lowercase()))
        }

        // "Returns a string that is constructed from repeating this string the
        // specified times." A fractional count truncates and a negative one is
        // nothing; both `unprobed`.
        "repeat" => {
            at_least(name, args, 1)?;
            let vals = ev.eval_args(args)?;
            let count = want_number(name, "count", &vals[0])?;
            at_most(name, args, 1)?;
            Ok(Value::Str(repeat(text, count)))
        }

        // "Returns a string with occurrences of `pattern` replaced with the
        // `replacement`." The plural is the only evidence of how many are
        // replaced, so **every** occurrence is, for a String pattern as well
        // as for a RegExp one, and a RegExp's `g` flag changes nothing.
        // `unprobed`. The replacement is literal text: `$1` is not a group
        // reference here. Also `unprobed`.
        "replace" => {
            at_least(name, args, 2)?;
            let vals = ev.eval_args(args)?;
            let pattern = want_pattern(name, "pattern", &vals[0])?;
            let replacement = want_string(name, "replacement", &vals[1])?;
            at_most(name, args, 2)?;
            Ok(Value::Str(match pattern {
                Pattern::Regexp(re) => match compile(re) {
                    Some(r) => r.replace_all(text, NoExpand(&replacement)).into_owned(),
                    None => text.to_string(),
                },
                Pattern::Text(p) => text.replace(p.as_str(), &replacement),
            }))
        }

        // Observed: `"Frothed basalt".reverse()` is `tlasab dehtorF`. By
        // Unicode scalar value, which is the unit `length` counts in; a
        // combining mark or an astral pair is `unprobed`.
        "reverse" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Str(text.chars().rev().collect()))
        }

        // "Returns a substring from `start` (inclusive) to `end` (exclusive)."
        // A negative index counting from the end, and an out-of-range index
        // clamping rather than refusing, are both `unprobed`: they are the
        // reading `slice` carries wherever else it is spelled that way, and
        // the alternative (a refusal) has no observed error string.
        "slice" => {
            at_least(name, args, 1)?;
            let vals = ev.eval_args(args)?;
            let start = want_number(name, "start", &vals[0])?;
            let end = match vals.get(1) {
                Some(v) => Some(want_number(name, "end", v)?),
                None => None,
            };
            at_most(name, args, 2)?;
            let chars: Vec<char> = text.chars().collect();
            let from = resolve_index(start, chars.len());
            let to = end.map_or(chars.len(), |e| resolve_index(e, chars.len()));
            Ok(Value::Str(if to <= from {
                String::new()
            } else {
                chars[from..to].iter().collect()
            }))
        }

        // "Returns an list of `n` substrings by splitting on `separator`."
        // Observed: `"Frothed basalt".split(/zz/)` is a List (`lucide-list`)
        // rendering as `Frothed basalt`, so a separator that never matches
        // gives the whole string as one element. `n` is read as a cap on how
        // many substrings come back; what a zero or negative `n` does is
        // `unprobed` and answers an empty list here. An empty String separator
        // splits into characters, also `unprobed`.
        "split" => {
            at_least(name, args, 1)?;
            let vals = ev.eval_args(args)?;
            let separator = want_pattern(name, "separator", &vals[0])?;
            let limit = match vals.get(1) {
                Some(v) => Some(want_number(name, "n", v)?),
                None => None,
            };
            at_most(name, args, 2)?;
            let mut parts: Vec<String> = match separator {
                Pattern::Regexp(re) => match compile(re) {
                    Some(r) => r.split(text).map(str::to_string).collect(),
                    None => vec![text.to_string()],
                },
                Pattern::Text(sep) if sep.is_empty() => {
                    text.chars().map(|c| c.to_string()).collect()
                }
                Pattern::Text(sep) => text.split(sep.as_str()).map(str::to_string).collect(),
            };
            if let Some(n) = limit {
                parts.truncate(count_of(n));
            }
            Ok(Value::List(parts.into_iter().map(Value::Str).collect()))
        }

        // "Converts the string to title case (first letter of each word
        // capitalized)." Observed: `"Frothed basalt".title()` is
        // `Frothed Basalt`. The seed's words were already lower case past
        // their first letter, so whether the rest of a word is lowered is
        // `unprobed`; the sentence claims only that the first letter is
        // capitalized, and there is no `upper()` in the language, so the rest
        // is left as written. A word starts at the string's start or after
        // whitespace — a hyphen or an apostrophe mid-word is `unprobed`.
        "title" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Str(title_case(text)))
        }

        // "Removes whitespace from both ends of the string."
        "trim" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Str(text.trim().to_string()))
        }

        _ => Err(unknown_function(name, subject.ty())),
    }
}

/// A `pattern` or a `separator`: the one parameter shape that takes either.
enum Pattern<'a> {
    Text(String),
    Regexp(&'a RegexpValue),
}

fn want_pattern<'a>(name: &str, param: &str, v: &'a Value) -> Result<Pattern<'a>, EvalError> {
    match v {
        Value::Regexp(re) => Ok(Pattern::Regexp(re)),
        // A Link is a String here as everywhere else.
        other => match other.as_text() {
            Some(s) => Ok(Pattern::Text(s.to_string())),
            // The expected-type name a two-type parameter prints is the
            // signature's own spelling; `unprobed`, because the probe only
            // ever passed this parameter a type it accepts.
            None => Err(type_error(name, param, "String,RegExp", other.ty())),
        },
    }
}

/// Every argument of a variadic String parameter.
fn strings(name: &str, param: &str, vals: &[Value]) -> Result<Vec<String>, EvalError> {
    vals.iter().map(|v| want_string(name, param, v)).collect()
}

/// Too few arguments, checked before the parameters' types.
fn at_least(name: &str, args: &[Expr], min: usize) -> Result<(), EvalError> {
    if args.len() < min {
        return Err(not_enough(name));
    }
    Ok(())
}

/// Too many arguments, checked *after* the parameters' types.
fn at_most(name: &str, args: &[Expr], max: usize) -> Result<(), EvalError> {
    if args.len() > max {
        return Err(too_many(name));
    }
    Ok(())
}

/// A count argument as a number of things: truncated, never negative, and NaN
/// is none.
fn count_of(n: f64) -> usize {
    if n.is_nan() || n <= 0.0 {
        0
    } else if n >= usize::MAX as f64 {
        usize::MAX
    } else {
        n.trunc() as usize
    }
}

fn repeat(text: &str, count: f64) -> String {
    let mut n = count_of(count);
    if !text.is_empty() {
        n = n.min((MAX_REPEAT_BYTES / text.len()).max(1));
    }
    text.repeat(n)
}

/// A `slice` bound: negative counts back from the end, and either end clamps
/// into the string rather than refusing.
fn resolve_index(i: f64, len: usize) -> usize {
    if i.is_nan() {
        return 0;
    }
    let len = len as f64;
    let i = i.trunc();
    let i = if i < 0.0 {
        (len + i).max(0.0)
    } else {
        i.min(len)
    };
    i as usize
}

fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    for c in text.chars() {
        if at_word_start && !c.is_whitespace() {
            out.extend(c.to_uppercase());
            at_word_start = false;
        } else {
            out.push(c);
            at_word_start = c.is_whitespace();
        }
    }
    out
}

/// A literal's pattern as Rust's `regex` wants it, or `None` when Rust cannot
/// compile what JavaScript would have. `\/` is unescaped first: the parser
/// keeps a literal's bytes as written, and `\/` is a JavaScript escape Rust
/// rejects. `g` and `y` are ignored — every replacement here is global
/// already, and nothing holds a `lastIndex`.
fn compile(re: &RegexpValue) -> Option<Regex> {
    let pattern = re.pattern.replace("\\/", "/");
    RegexBuilder::new(&pattern)
        .case_insensitive(re.flags.contains('i'))
        .multi_line(re.flags.contains('m'))
        .dot_matches_new_line(re.flags.contains('s'))
        .build()
        .ok()
}
