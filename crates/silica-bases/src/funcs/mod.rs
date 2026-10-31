//! The function table.
//!
//! It is *not* the JavaScript prototype — `12.345.toFixed(2)` works although
//! Number's prototype carries only `renderTo`, and `[3,1,2].length()` is
//! refused although List's prototype has `length`. The table below is the one
//! read off Obsidian two ways that agreed: the formula editor's completion
//! list per receiver, and a brute-force pass over candidate names.
//!
//! Every name is **case-insensitive**, so every family matches on the
//! lowercased name. Values are not.
//!
//! The four refusals are Obsidian's own strings, and the view renders them as
//! the cell's content, so they are part of the behaviour:
//!
//! - `Cannot find function "N" on type T`
//! - `Cannot find "N" on type T` (a property)
//! - `Cannot call function "N", not enough arguments.` / `too many arguments.`
//! - `Type error in "N", parameter "P". Expected X, given Y.`

pub mod file;
pub mod globals;
pub mod list;
pub mod number;
pub mod text;
pub mod time;

use crate::context::lookup_ci;
use crate::eval::{EvalError, EvalResult, Evaluator};
use crate::expr::Expr;
use crate::value::{Type, Value};

/// A method call, `subject.name(args)`.
pub fn call(ev: &mut Evaluator<'_>, subject: &Value, name: &str, args: &[Expr]) -> EvalResult {
    let lower = name.to_ascii_lowercase();

    // Null answers every call with null. Three names differ, and they are the
    // only three: `isEmpty()` is true, `isTruthy()` is false, `toString()` is
    // the String "null".
    if subject.is_null() {
        return Ok(match lower.as_str() {
            "isempty" => Value::Bool(true),
            "istruthy" => Value::Bool(false),
            "tostring" => Value::Str("null".into()),
            _ => Value::Null,
        });
    }

    // Every type has these three.
    match lower.as_str() {
        "istruthy" => {
            arity(name, args, 0, 0)?;
            return Ok(Value::Bool(subject.is_truthy()));
        }
        "tostring" => {
            arity(name, args, 0, 0)?;
            return Ok(Value::Str(subject.to_string()));
        }
        "istype" => {
            arity(name, args, 1, 1)?;
            let want = ev.eval(&args[0])?;
            let Value::Str(want) = want else {
                return Err(type_error(name, "type", "String", want.ty()));
            };
            // Which names `isType` accepts is unprobed; the type tag's own
            // name, case-insensitively, is the reading the error strings
            // support.
            return Ok(Value::Bool(
                want.eq_ignore_ascii_case(subject.ty().as_str()),
            ));
        }
        _ => {}
    }

    match subject.ty() {
        Type::String | Type::RegExp => text::call(ev, subject, name, args),
        Type::Number | Type::Boolean => number::call(ev, subject, name, args),
        Type::List | Type::Tags => list::call(ev, subject, name, args),
        Type::Object => list::call_object(ev, subject, name, args),
        Type::Date | Type::DateTime | Type::Duration => time::call(ev, subject, name, args),
        // A Link is a String receiver as well as a Link, so `file` owns the
        // two names of its own and hands everything else to `text`.
        Type::Link => file::call_link(ev, subject, name, args),
        Type::File => file::call(ev, subject, name, args),
        ty => Err(unknown_function(name, ty)),
    }
}

/// A property access, `subject.name`.
pub fn property(ev: &mut Evaluator<'_>, subject: &Value, name: &str) -> EvalResult {
    // Null answers every property access with null.
    if subject.is_null() {
        return Ok(Value::Null);
    }
    match subject {
        Value::Str(s) => length_or_unknown(name, s.chars().count(), Type::String),
        Value::Link(l) => length_or_unknown(name, l.text().chars().count(), Type::Link),
        Value::List(items) => length_or_unknown(name, items.len(), Type::List),
        Value::Tags(tags) => length_or_unknown(name, tags.len(), Type::Tags),
        // Any key of the map; an unknown key is null.
        Value::Object(map) => Ok(lookup_ci(map, name).unwrap_or(Value::Null)),
        Value::Date(_) | Value::Duration(_) => time::property(subject, name),
        Value::File(_) => file::property(ev, subject, name),
        Value::Formulas => ev.formula(name),
        other => Err(unknown_property(name, other.ty())),
    }
}

fn length_or_unknown(name: &str, len: usize, ty: Type) -> EvalResult {
    if name.eq_ignore_ascii_case("length") {
        Ok(Value::Number(len as f64))
    } else {
        Err(unknown_property(name, ty))
    }
}

// --- the four refusals ------------------------------------------------------

pub fn unknown_function(name: &str, ty: Type) -> EvalError {
    EvalError::new(format!(
        "Cannot find function \"{name}\" on type {}",
        ty.as_str()
    ))
}

pub fn unknown_property(name: &str, ty: Type) -> EvalError {
    EvalError::new(format!("Cannot find \"{name}\" on type {}", ty.as_str()))
}

pub fn not_enough(name: &str) -> EvalError {
    EvalError::new(format!(
        "Cannot call function \"{name}\", not enough arguments."
    ))
}

pub fn too_many(name: &str) -> EvalError {
    EvalError::new(format!(
        "Cannot call function \"{name}\", too many arguments."
    ))
}

pub fn type_error(name: &str, param: &str, expected: &str, given: Type) -> EvalError {
    EvalError::new(format!(
        "Type error in \"{name}\", parameter \"{param}\". Expected {expected}, given {}.",
        given.as_str()
    ))
}

/// Checks a call's argument count against the signature's own bounds.
pub fn arity(name: &str, args: &[Expr], min: usize, max: usize) -> Result<(), EvalError> {
    if args.len() < min {
        return Err(not_enough(name));
    }
    if args.len() > max {
        return Err(too_many(name));
    }
    Ok(())
}

/// An argument that must be a String. A Link is one: it is a String receiver
/// everywhere else, so it is accepted here too.
pub fn want_string(name: &str, param: &str, v: &Value) -> Result<String, EvalError> {
    match v.as_text() {
        Some(s) => Ok(s.to_string()),
        None => Err(type_error(name, param, "String", v.ty())),
    }
}

/// An argument that must be a Number.
pub fn want_number(name: &str, param: &str, v: &Value) -> Result<f64, EvalError> {
    match v {
        Value::Number(n) => Ok(*n),
        other => Err(type_error(name, param, "Number", other.ty())),
    }
}
