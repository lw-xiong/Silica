//! Evaluating a parsed statement.
//!
//! Three rules run through everything here and are observed rather than
//! inferred:
//!
//! - **Null absorbs.** A missing property is null, and null answers every
//!   function call and every property access with null. Only `isEmpty()`,
//!   `isTruthy()` and `toString()` differ.
//! - **Comparison coerces; arithmetic does not.** `"3" > 2` is true, while
//!   `1 + true` is `Invalid operator between Number and Boolean`.
//! - **Comparison against null is three-valued.** `nosuchprop > 1` is null,
//!   not false, while `nosuchprop == null` is true.

use std::fmt;

use crate::context::Context;
use crate::expr::{BinaryOp, Expr, Literal};
use crate::funcs;
use crate::value::{compare, loose_eq, Cmp, Value};

/// Why evaluation stopped. Obsidian renders the message as the cell's content,
/// so the text matters as much as the failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError {
    pub message: String,
}

impl EvalError {
    pub fn new(message: impl Into<String>) -> EvalError {
        EvalError {
            message: message.into(),
        }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EvalError {}

pub type EvalResult = Result<Value, EvalError>;

/// How deep `formula.a` referring to `formula.b` may go before the evaluator
/// calls it a cycle. Obsidian's own behaviour here is unprobed.
const MAX_FORMULA_DEPTH: usize = 32;

/// One frame of the magic idents `filter`, `map`, `reduce` and a summary bind.
/// There is no lambda in the language: the argument is an ordinary expression
/// evaluated once per element with these names in scope.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub value: Option<Value>,
    pub index: Option<Value>,
    pub acc: Option<Value>,
    pub values: Option<Value>,
    /// `Object.filter` and `Object.map` bind this instead of `index`:
    /// Obsidian's own completion text says "uses the variables `key` and
    /// `value`".
    pub key: Option<Value>,
}

/// The tree walk.
pub struct Evaluator<'a> {
    pub(crate) ctx: &'a dyn Context,
    scopes: Vec<Scope>,
    depth: usize,
}

/// Evaluates a parsed statement.
pub fn eval(expr: &Expr, ctx: &dyn Context) -> EvalResult {
    Evaluator::new(ctx).eval(expr)
}

/// Parses and evaluates a statement. A statement that does not parse is not an
/// error: Obsidian keeps it as an `invalid` node and the cell renders empty, so
/// this answers null.
pub fn eval_str(source: &str, ctx: &dyn Context) -> EvalResult {
    eval(&crate::expr::parse_node(source), ctx)
}

/// Whether a row passes a filter statement. A filter that fails to parse or to
/// evaluate **silently empties the view** — 0 results, no banner, no notice —
/// so every failure is false here.
pub fn matches_filter(source: &str, ctx: &dyn Context) -> bool {
    eval_str(source, ctx)
        .map(|v| v.is_truthy())
        .unwrap_or(false)
}

impl<'a> Evaluator<'a> {
    pub fn new(ctx: &'a dyn Context) -> Evaluator<'a> {
        Evaluator {
            ctx,
            scopes: Vec::new(),
            depth: 0,
        }
    }

    pub fn context(&self) -> &'a dyn Context {
        self.ctx
    }

    pub fn eval(&mut self, expr: &Expr) -> EvalResult {
        match expr {
            Expr::Primitive(lit) => Ok(match lit {
                Literal::Number(n) => Value::Number(*n),
                Literal::String(s) => Value::Str(s.clone()),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::Null => Value::Null,
            }),
            Expr::Ident(name) => self.ident(name),
            Expr::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item)?);
                }
                Ok(Value::List(out))
            }
            Expr::Regexp { pattern, flags } => Ok(Value::Regexp(crate::value::RegexpValue {
                pattern: pattern.clone(),
                flags: flags.clone(),
            })),
            Expr::Paren(inner) => self.eval(inner),
            Expr::Binary { op, left, right } => self.binary(*op, left, right),
            Expr::Negate(inner) => match self.eval(inner)? {
                Value::Null => Ok(Value::Null),
                Value::Number(n) => Ok(Value::Number(-n)),
                other => Err(EvalError::new(format!(
                    "Invalid operator between Number and {}",
                    other.ty().as_str()
                ))),
            },
            Expr::Not(inner) => Ok(Value::Bool(!self.eval(inner)?.is_truthy())),
            Expr::Function {
                name,
                subject,
                args,
            } => match subject {
                Some(subject) => {
                    let receiver = self.eval(subject)?;
                    funcs::call(self, &receiver, name, args)
                }
                None => funcs::globals::call(self, name, args),
            },
            Expr::ObjectAccess { object, index } => {
                let receiver = self.eval(object)?;
                funcs::property(self, &receiver, index)
            }
            Expr::ArrayAccess { array, index } => {
                let receiver = self.eval(array)?;
                let idx = self.eval(index)?;
                Ok(index_into(&receiver, &idx))
            }
            // A statement that did not parse renders as an empty cell, not as
            // an error: Obsidian keeps the `invalid` node and evaluates it to
            // nothing.
            Expr::Invalid { .. } => Ok(Value::Null),
        }
    }

    /// Evaluates every argument of a call. `filter`, `map`, `reduce` and `if`
    /// do not use this: their arguments are expressions they evaluate
    /// themselves, under a scope or a condition.
    pub fn eval_args(&mut self, args: &[Expr]) -> Result<Vec<Value>, EvalError> {
        args.iter().map(|a| self.eval(a)).collect()
    }

    /// Evaluates an expression with `value`, `index`, `acc` or `values` bound.
    pub fn in_scope(&mut self, scope: Scope, expr: &Expr) -> EvalResult {
        self.scopes.push(scope);
        let out = self.eval(expr);
        self.scopes.pop();
        out
    }

    fn ident(&mut self, name: &str) -> EvalResult {
        // The magic idents first: they are bound only while a per-element
        // expression runs, so they cannot shadow a property outside one.
        if let Some(scope) = self.scopes.last() {
            let bound = match name.to_ascii_lowercase().as_str() {
                "value" => scope.value.clone(),
                "index" => scope.index.clone(),
                "acc" => scope.acc.clone(),
                "values" => scope.values.clone(),
                "key" => scope.key.clone(),
                _ => None,
            };
            if let Some(v) = bound {
                return Ok(v);
            }
        }
        match name.to_ascii_lowercase().as_str() {
            "file" => Ok(self.ctx.file().map(Value::File).unwrap_or(Value::Null)),
            // **`this` is the base file, not the row** — and it is a context
            // of its own rather than a File: `this.file.name` is the base
            // file's name, `this.note` an Object and `this.formula`
            // `Formulas[]`, while `this.status` is null even when the row has
            // one. The completion list for `this.` offers `file`, `note`,
            // `formula`, `this` and the vault's frontmatter names, and none of
            // the File properties, so a bare `Value::File` would answer
            // `this.path` where Obsidian answers nothing.
            "this" => Ok(self.this_value()),
            "note" => Ok(Value::Object(self.ctx.properties())),
            "formula" => Ok(Value::Formulas),
            // An unknown ident is null, not an error: `nums.map(item * 2)` is
            // a list of nulls.
            _ => Ok(self.ctx.property(name).unwrap_or(Value::Null)),
        }
    }

    /// `this`: the base file's context. A frontmatter key of the base file
    /// that is called `file`, `note` or `formula` is shadowed by the
    /// namespace, which is unprobed.
    fn this_value(&self) -> Value {
        let Some(base) = self.ctx.base_file() else {
            return Value::Null;
        };
        let mut map = base.properties.clone();
        map.insert("file".into(), Value::File(base.clone()));
        map.insert("note".into(), Value::Object(base.properties.clone()));
        map.insert("formula".into(), Value::Formulas);
        Value::Object(map)
    }

    /// `formula.<name>`, evaluated in this row's context.
    pub(crate) fn formula(&mut self, name: &str) -> EvalResult {
        let Some(source) = self.ctx.formula(name) else {
            return Err(EvalError::new(format!(
                "Cannot find \"{name}\" on type Formulas"
            )));
        };
        if self.depth >= MAX_FORMULA_DEPTH {
            return Err(EvalError::new(format!(
                "Cannot find \"{name}\" on type Formulas"
            )));
        }
        self.depth += 1;
        let out = self.eval(&crate::expr::parse_node(&source));
        self.depth -= 1;
        out
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> EvalResult {
        // `&&` and `||` coerce through truthiness and answer a Boolean.
        match op {
            BinaryOp::And => {
                let l = self.eval(left)?;
                if !l.is_truthy() {
                    return Ok(Value::Bool(false));
                }
                return Ok(Value::Bool(self.eval(right)?.is_truthy()));
            }
            BinaryOp::Or => {
                let l = self.eval(left)?;
                if l.is_truthy() {
                    return Ok(Value::Bool(true));
                }
                return Ok(Value::Bool(self.eval(right)?.is_truthy()));
            }
            _ => {}
        }
        let l = self.eval(left)?;
        let r = self.eval(right)?;
        match op {
            BinaryOp::Eq => Ok(Value::Bool(loose_eq(&l, &r))),
            BinaryOp::Ne => Ok(Value::Bool(!loose_eq(&l, &r))),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let ord = match compare(&l, &r) {
                    // A null operand makes the comparison null.
                    Cmp::Null => return Ok(Value::Null),
                    // Two values that do not order answer false, as a NaN
                    // comparison does: `"abc" > 1` is false.
                    Cmp::Unordered => return Ok(Value::Bool(false)),
                    Cmp::Ordered(ord) => ord,
                };
                let yes = match op {
                    BinaryOp::Lt => ord.is_lt(),
                    BinaryOp::Le => ord.is_le(),
                    BinaryOp::Gt => ord.is_gt(),
                    _ => ord.is_ge(),
                };
                Ok(Value::Bool(yes))
            }
            _ => arithmetic(op, &l, &r),
        }
    }
}

/// `a[i]`. A List takes a number, an Object a key; anything else is null.
fn index_into(receiver: &Value, index: &Value) -> Value {
    match receiver {
        Value::List(items) => match index {
            Value::Number(n) => {
                let i = *n as isize;
                if i < 0 {
                    Value::Null
                } else {
                    items.get(i as usize).cloned().unwrap_or(Value::Null)
                }
            }
            _ => Value::Null,
        },
        Value::Tags(tags) => match index {
            Value::Number(n) if *n >= 0.0 => tags
                .get(*n as usize)
                .map(|t| Value::Str(t.clone()))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        },
        Value::Object(map) => match index {
            Value::Str(key) => crate::context::lookup_ci(map, key).unwrap_or(Value::Null),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

/// `+ - * / %`. Typed, and it refuses: `1 + true` and `when - "2026-01-01"`
/// are both errors. `+` with a String on either side concatenates.
pub fn arithmetic(op: BinaryOp, l: &Value, r: &Value) -> EvalResult {
    use BinaryOp::*;
    // Null absorbs: `nosuchprop + 1` is null, with no error.
    if l.is_null() || r.is_null() {
        return Ok(Value::Null);
    }
    // `+` with a String on either side concatenates — `1 + "2"` is the String
    // `12` — **except** against a Date, where the String is a Duration:
    // `when + "1 month"` moves the date on. A Date never concatenates; a
    // String it cannot read as a Duration is the same refusal `-` gives.
    if op == Add
        && !matches!(l, Value::Date(_))
        && !matches!(r, Value::Date(_))
        && (string_side(l).is_some() || string_side(r).is_some())
    {
        return Ok(Value::Str(format!("{l}{r}")));
    }
    match (l, r) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(match op {
            Add => x + y,
            Sub => x - y,
            Mul => x * y,
            Div => x / y,
            Rem => x % y,
            _ => unreachable!("not an arithmetic operator"),
        })),
        (Value::Date(d), Value::Duration(dur)) if matches!(op, Add | Sub) => {
            Ok(Value::Date(crate::funcs::time::shift(d, dur, op == Sub)))
        }
        // A String in a Duration's position is parsed as one: `now() - "1 week"`.
        (Value::Date(d), Value::Str(s)) if matches!(op, Add | Sub) => {
            match crate::funcs::time::parse_duration(s) {
                Some(dur) => Ok(Value::Date(crate::funcs::time::shift(d, &dur, op == Sub))),
                None => Err(invalid_operator(l, r)),
            }
        }
        (Value::Str(s), Value::Date(d)) if op == Add => {
            match crate::funcs::time::parse_duration(s) {
                Some(dur) => Ok(Value::Date(crate::funcs::time::shift(d, &dur, false))),
                None => Err(invalid_operator(l, r)),
            }
        }
        (Value::Date(a), Value::Date(b)) if op == Sub => {
            Ok(Value::Duration(crate::funcs::time::between(b, a)))
        }
        (Value::Duration(x), Value::Duration(y)) if matches!(op, Add | Sub) => Ok(Value::Duration(
            crate::funcs::time::combine(x, y, op == Sub),
        )),
        (Value::Duration(x), Value::Number(n)) if matches!(op, Mul | Div) => Ok(Value::Duration(
            crate::funcs::time::scale(x, if op == Mul { *n } else { 1.0 / *n }),
        )),
        (Value::Number(n), Value::Duration(y)) if op == Mul => {
            Ok(Value::Duration(crate::funcs::time::scale(y, *n)))
        }
        _ => Err(invalid_operator(l, r)),
    }
}

fn string_side(v: &Value) -> Option<&str> {
    match v {
        Value::Str(s) => Some(s),
        _ => None,
    }
}

fn invalid_operator(l: &Value, r: &Value) -> EvalError {
    EvalError::new(format!(
        "Invalid operator between {} and {}",
        l.ty().as_str(),
        r.ty().as_str()
    ))
}
