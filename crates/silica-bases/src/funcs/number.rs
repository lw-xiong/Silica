//! Number and Boolean.
//!
//! | Receiver | Functions | Properties |
//! | --- | --- | --- |
//! | Number | `abs()`, `ceil()`, `floor()`, `isEmpty()`, `round()`, `toFixed(precision: Number)` | — |
//! | Boolean | — | — |
//!
//! Numbers are IEEE doubles printed as JavaScript prints them, which
//! [`crate::value::js_number`] owns.
//!
//! Observed on the seed `12.345`: `abs()` → `12.345`, `ceil()` → `13`,
//! `floor()` → `12`, `round()` → `12`, `isEmpty()` → `false`, and
//! `toFixed()` refuses with *not enough arguments* while `toFixed(/zz/)`,
//! `toFixed(/zz/, /zz/)` and `toFixed(/zz/, /zz/, /zz/)` all refuse with
//! `Type error in "toFixed", parameter "precision". Expected Number, given
//! RegExp.` — the parameter's type is checked **before** the call's maximum
//! arity, which is why the three-argument try is a type error rather than
//! *too many arguments*.
//!
//! **`toFixed` returns a String, not a Number**: `density.toFixed(3)` is the
//! value `{icon: "lucide-text", data: "1.210"}` on `density: 1.21`.
//!
//! Boolean has no functions of its own: the only names that answer on `true`
//! are the three every type has, and `true.renderTo()` is
//! `Cannot find function "renderTo" on type Boolean`.

use crate::eval::{EvalResult, Evaluator};
use crate::expr::Expr;
use crate::value::{js_number, Value};

use super::{not_enough, too_many, unknown_function, want_number};

pub fn call(ev: &mut Evaluator<'_>, subject: &Value, name: &str, args: &[Expr]) -> EvalResult {
    // Boolean reaches this module with Number, and has nothing of its own.
    let Some(n) = subject.as_number() else {
        return Err(unknown_function(name, subject.ty()));
    };

    match name.to_ascii_lowercase().as_str() {
        "abs" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Number(n.abs()))
        }
        "ceil" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Number(n.ceil()))
        }
        "floor" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Number(n.floor()))
        }
        "round" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Number(round_half_up(n)))
        }
        // Observed false for `12.345`. Whether `0` or `NaN` is "empty" is
        // unprobed; a Number is never empty is the reading that matches the
        // one observation and the String description ("has no characters").
        "isempty" => {
            super::arity(name, args, 0, 0)?;
            Ok(Value::Bool(false))
        }
        "tofixed" => {
            // Minimum arity, then the parameter's type, then the maximum: the
            // order the three refusals above pin down.
            if args.is_empty() {
                return Err(not_enough(name));
            }
            let vals = ev.eval_args(args)?;
            let precision = want_number(name, "precision", &vals[0])?;
            if args.len() > 1 {
                return Err(too_many(name));
            }
            Ok(Value::Str(to_fixed(n, precision)))
        }
        _ => Err(unknown_function(name, subject.ty())),
    }
}

/// `round()`. Halves go **up** rather than away from zero, so `-2.5` rounds to
/// `-2`: the receiver is a JavaScript number and `12.345.round()` → `12` is
/// the only magnitude observed, so the tie rule itself is `unprobed`.
fn round_half_up(n: f64) -> f64 {
    if !n.is_finite() {
        return n;
    }
    let r = (n + 0.5).floor();
    // `n + 0.5` can round up in binary and carry the floor with it; back the
    // result off when the sum was not really big enough.
    if r - 0.5 > n {
        r - 1.0
    } else {
        r
    }
}

/// `toFixed(precision)`, as a String.
///
/// `unprobed` beyond `1.21.toFixed(3)` → `"1.210"`: the padding with a
/// trailing zero is observed, and the rest follows that one reading —
/// a half-way digit rounds away from zero (`2.5.toFixed(0)` → `"3"`), a
/// precision outside 0..=100 is clamped rather than refused (Obsidian names no
/// refusal for one), and a non-finite receiver prints the way it otherwise
/// prints.
fn to_fixed(n: f64, precision: f64) -> String {
    if !n.is_finite() {
        return js_number(n);
    }
    let digits = if precision.is_nan() {
        0
    } else {
        precision.trunc().clamp(0.0, 100.0) as usize
    };
    // A magnitude this large has no fixed notation left to print.
    if n.abs() >= 1e21 {
        return js_number(n);
    }

    let negative = n < 0.0;
    // Rust prints the *exact* decimal expansion of the double, correctly
    // rounded at the precision asked for, so asking for far more digits than
    // are wanted and then cutting leaves the digit at `digits + 1` untouched.
    let wide = format!("{:.*}", digits + 30, n.abs());
    let (int_part, frac) = wide
        .split_once('.')
        .expect("a precision of 30+ writes a point");

    let mut out: Vec<u8> = int_part
        .bytes()
        .chain(frac.bytes().take(digits))
        .map(|b| b - b'0')
        .collect();
    let mut int_len = int_part.len();

    // Everything from the cut onward is at least a half exactly when the first
    // cut digit is 5 or more.
    if frac.as_bytes()[digits] >= b'5' {
        let mut i = out.len();
        loop {
            if i == 0 {
                out.insert(0, 1);
                int_len += 1;
                break;
            }
            i -= 1;
            if out[i] == 9 {
                out[i] = 0;
            } else {
                out[i] += 1;
                break;
            }
        }
    }

    let mut s = String::with_capacity(out.len() + 2);
    if negative {
        s.push('-');
    }
    for (i, d) in out.iter().enumerate() {
        if i == int_len {
            s.push('.');
        }
        s.push((b'0' + d) as char);
    }
    s
}
