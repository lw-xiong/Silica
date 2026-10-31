//! List and Object.
//!
//! | Receiver | Functions | Properties |
//! | --- | --- | --- |
//! | List | `contains(value: Any)`, `containsAll(values: Any)`, `containsAny(values: Any)`, `earliest()`, `filter(value: Boolean)`, `flat()`, `isEmpty()`, `join(separator: String)`, `latest()`, `map(value: Any)`, `max()`, `mean()`, `median()`, `min()`, `reduce(value: Any, acc: Any)`, `reverse()`, `slice(start: Number, end: Number?)`, `sort()`, `stddev()`, `sum()`, `unique()` | `length` |
//! | Object | `filter()`, `isEmpty()`, `keys()`, `map()`, `values()` | any key of the map; an unknown key is null |
//!
//! **There is no lambda.** `filter`, `map` and `reduce` take an *ordinary
//! expression*, evaluated once per element with the idents `value` and `index`
//! in scope — and `acc` for `reduce`, whose second argument is the initial
//! accumulator. Use [`Evaluator::in_scope`] with a [`Scope`], never a closure
//! over an evaluated argument: the argument must stay unevaluated.
//!
//! Observed, on `nums: [3, 1, 2]` and `listish: [alpha, beta, alpha]`:
//! `listish.filter(value != "beta")` → `alpha, alpha`; `nums.filter(index > 0)`
//! → `1, 2`; `nums.map(value * 2)` → `6, 2, 4`; `nums.map(index)` → `0, 1, 2`;
//! `nums.map(item * 2)` → three nulls, because an unknown ident is null rather
//! than an error; `nums.reduce(acc + value, 0)` → 6; `nums.reduce(value, 0)` →
//! 2, the last element, because the expression ignores `acc`;
//! `nums.map(value, index)` → `Cannot call function "map", too many arguments.`
//!
//! `index` is bound inside `reduce` too, which the signature does not say and
//! the probe does: `nums.reduce(index, 0)` → `2` and
//! `nums.reduce(value + index, 0)` → `4`. The *only* accumulator name that
//! binds is `acc`: `accumulator`, `total`, `result`, `previous`, `carry`,
//! `sum`, `current`, `prev` and `a` were each tried with `+ value` and each
//! answered null.
//!
//! **Where a refusal checks the argument's type, it checks it before it counts
//! the arguments.** `[3,1,2].join(/zz/, /zz/, /zz/)` is
//! `Type error in "join", parameter "separator". Expected String, given
//! RegExp.`, not `too many arguments.`, and `slice` behaves the same, while
//! `nums.map(value, index)` and `nums.sort(value)` — neither of which type
//! checks anything — are `too many arguments.`. So every function here reads
//! "too few, then types, then too many".
//!
//! A **Tags** receiver (`file.tags`) is dispatched here and names itself
//! **List** in a refusal: `tags.includes()` is
//! `Cannot find function "includes" on type List`, not `… on type Tags`. Every
//! refusal raised in this module therefore says List.

use std::cmp::Ordering;

use indexmap::IndexMap;

use crate::eval::{EvalResult, Evaluator, Scope};
use crate::expr::Expr;
use crate::value::{loose_eq, DateValue, Type, Value};

use super::{arity, not_enough, too_many, unknown_function, want_number, want_string};

/// A List or Tags method call.
pub fn call(ev: &mut Evaluator<'_>, subject: &Value, name: &str, args: &[Expr]) -> EvalResult {
    // Tags calls itself List in a refusal, and it is the only other receiver
    // that reaches this function.
    let ty = Type::List;
    let items = subject.as_list().unwrap_or_default();

    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        // `contains(value: Any)` — "Returns true if the list contains
        // `value`." Observed: `listish.contains("alpha")` is true and
        // `listish.contains("ALPHA")` is false, so values are compared as the
        // language's own `==` compares them and not case-insensitively.
        // Whether `[3,1,2].contains("3")` is true is unprobed; loose equality,
        // which is what `==` uses, says it is.
        "contains" => {
            arity(name, args, 1, 1)?;
            let wanted = ev.eval(&args[0])?;
            Ok(Value::Bool(contains(&items, &wanted)))
        }
        // `containsAll(values: Any)` / `containsAny(values: Any)` are
        // **variadic from zero**: `[3,1,2].containsAll()` is true and
        // `[3,1,2].containsAny()` is false, which is "all of nothing" and
        // "any of nothing". Their maximum is unprobed — the brute-force pass
        // stopped at the first try that did not refuse — and whether a single
        // List argument is read as the set of values rather than as one value
        // to look for is unprobed too; each argument is one value here.
        "containsall" => {
            let wanted = ev.eval_args(args)?;
            Ok(Value::Bool(wanted.iter().all(|w| contains(&items, w))))
        }
        "containsany" => {
            let wanted = ev.eval_args(args)?;
            Ok(Value::Bool(wanted.iter().any(|w| contains(&items, w))))
        }
        // `earliest()` / `latest()` — "Returns the earliest/latest date value
        // in the list." Observed: `[3,1,2].earliest()` and `.latest()` are
        // both **null**, so a list with no date in it answers null rather than
        // refusing. Whether a String that parses as a date counts as a date
        // value is unprobed; it does here, because every other place a Date
        // meets a String parses the String (`when > "2026-01-01"`).
        "earliest" => {
            arity(name, args, 0, 0)?;
            Ok(extreme_date(&items, Ordering::Less))
        }
        "latest" => {
            arity(name, args, 0, 0)?;
            Ok(extreme_date(&items, Ordering::Greater))
        }
        // `filter(value: Boolean)` keeps the elements whose expression is
        // truthy — truthy, not Boolean: `[3,1,2].filter(/zz/)` keeps all three
        // and `listish.filter("value != 'beta'")` keeps all three, because a
        // RegExp and a non-empty String are both truthy.
        "filter" => {
            arity(name, args, 1, 1)?;
            let mut out = Vec::new();
            for (i, item) in items.iter().enumerate() {
                if ev.in_scope(element(item, i), &args[0])?.is_truthy() {
                    out.push(item.clone());
                }
            }
            Ok(Value::List(out))
        }
        // `flat()` — "Flattens nested list into a single list." How deep is
        // unprobed (the only observation is `[3,1,2].flat()` → `3,1,2`, which
        // has nothing to flatten); "into a single list" reads as all the way
        // down rather than one level.
        "flat" => {
            arity(name, args, 0, 0)?;
            let mut out = Vec::new();
            flatten(&items, &mut out);
            Ok(Value::List(out))
        }
        "isempty" => {
            arity(name, args, 0, 0)?;
            Ok(Value::Bool(items.is_empty()))
        }
        // `join(separator: String)`. The separator is required and typed:
        // `[3,1,2].join()` is not enough arguments, `[3,1,2].join(/zz/)` is a
        // type error on `separator`.
        "join" => {
            if args.is_empty() {
                return Err(not_enough(name));
            }
            let sep = ev.eval(&args[0])?;
            let sep = want_string(name, "separator", &sep)?;
            if args.len() > 1 {
                return Err(too_many(name));
            }
            let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            Ok(Value::Str(parts.join(&sep)))
        }
        // `map(value: Any)` answers one element per element, in order.
        "map" => {
            arity(name, args, 1, 1)?;
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                out.push(ev.in_scope(element(item, i), &args[0])?);
            }
            Ok(Value::List(out))
        }
        // The five numeric summaries are over "the number values in the list",
        // which is Obsidian's own wording in the completion list, so a
        // non-number is skipped rather than coerced or refused. A list with no
        // number in it is **unprobed**; null is the reading `earliest()` and
        // `latest()` support, since they answer null for a list with no date.
        "max" => {
            arity(name, args, 0, 0)?;
            Ok(extreme_number(&items, Ordering::Greater))
        }
        "min" => {
            arity(name, args, 0, 0)?;
            Ok(extreme_number(&items, Ordering::Less))
        }
        // Whether `mean()` divides by the count of numbers or by the list's
        // length is **unprobed** — they differ only for a list that mixes
        // numbers with anything else, and `[3,1,2].mean()` is 2 either way.
        // "the mean of the number values in the list" reads as the count of
        // numbers.
        "mean" => {
            arity(name, args, 0, 0)?;
            let nums = numbers(&items);
            if nums.is_empty() {
                return Ok(Value::Null);
            }
            Ok(Value::Number(nums.iter().sum::<f64>() / nums.len() as f64))
        }
        // An even count is unprobed; the mean of the two middle numbers is the
        // only reading of "median" that is defined there.
        "median" => {
            arity(name, args, 0, 0)?;
            let mut nums = numbers(&items);
            if nums.is_empty() {
                return Ok(Value::Null);
            }
            nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let n = nums.len();
            Ok(Value::Number(if n % 2 == 0 {
                (nums[n / 2 - 1] + nums[n / 2]) / 2.0
            } else {
                nums[n / 2]
            }))
        }
        // `reduce(value: Any, acc: Any)`. The second argument is the initial
        // accumulator and is evaluated **once, outside the element scope**;
        // the first runs per element with `value`, `index` and `acc` bound,
        // and its result becomes the next `acc`. One argument is not enough
        // arguments, which is what makes the initial accumulator required
        // rather than optional.
        "reduce" => {
            arity(name, args, 2, 2)?;
            let mut acc = ev.eval(&args[1])?;
            for (i, item) in items.iter().enumerate() {
                let mut scope = element(item, i);
                scope.acc = Some(acc);
                acc = ev.in_scope(scope, &args[0])?;
            }
            Ok(acc)
        }
        "reverse" => {
            arity(name, args, 0, 0)?;
            let mut out = items;
            out.reverse();
            Ok(Value::List(out))
        }
        // `slice(start: Number, end: Number?)` — "a shallow copy of a portion
        // of the list from `start` (inclusive) to `end` (exclusive)". Whether
        // a negative index counts from the end is **unprobed**; it clamps to
        // the front here, which is the reading `list[-1]` supports — indexing
        // a List with a negative number is null, not the last element.
        "slice" => {
            if args.is_empty() {
                return Err(not_enough(name));
            }
            let start = ev.eval(&args[0])?;
            let start = want_number(name, "start", &start)?;
            let end = match args.get(1) {
                Some(arg) => {
                    let v = ev.eval(arg)?;
                    Some(want_number(name, "end", &v)?)
                }
                None => None,
            };
            if args.len() > 2 {
                return Err(too_many(name));
            }
            let len = items.len();
            let from = clamp_index(start, len);
            let to = end.map(|e| clamp_index(e, len)).unwrap_or(len);
            Ok(Value::List(if from < to {
                items[from..to].to_vec()
            } else {
                Vec::new()
            }))
        }
        // `sort()` — "Sorts list elements from smallest to largest."
        // `[3,1,2].sort()` is `1,2,3`. What a mixed list orders by is
        // **unprobed**: two numbers compare as numbers here and anything else
        // compares as its rendered text, which is the only total order the
        // observations leave room for. The sort is stable; whether Obsidian's
        // is, is unprobed.
        "sort" => {
            arity(name, args, 0, 0)?;
            let mut out = items;
            out.sort_by(sort_order);
            Ok(Value::List(out))
        }
        // The population standard deviation, not the sample one:
        // `[3,1,2].stddev()` is `0.816496580927726`, which is √(2/3) — the
        // sample deviation would be 1.
        "stddev" => {
            arity(name, args, 0, 0)?;
            let nums = numbers(&items);
            if nums.is_empty() {
                return Ok(Value::Null);
            }
            let mean = nums.iter().sum::<f64>() / nums.len() as f64;
            let var = nums.iter().map(|n| (n - mean) * (n - mean)).sum::<f64>() / nums.len() as f64;
            Ok(Value::Number(var.sqrt()))
        }
        "sum" => {
            arity(name, args, 0, 0)?;
            let nums = numbers(&items);
            if nums.is_empty() {
                return Ok(Value::Null);
            }
            Ok(Value::Number(nums.iter().sum()))
        }
        // `unique()` — "Removes duplicate elements." Two observations, and
        // they disagree about order: `[3,1,2].unique()` is **`1,2,3`** while
        // `listish.unique()` ([alpha, beta, alpha]) is **`alpha,beta`**, which
        // is first-seen order. So the distinct elements are not returned in
        // the order they appear and they are not sorted either: they come back
        // in the order a plain JavaScript object hands back its keys —
        // array-index-like keys ascending first, then every other key in
        // insertion order — which is what grouping the elements by their
        // rendered text produces. `["beta", "alpha"].unique()` is **unprobed**
        // and answers `beta, alpha` here; a "dedupe, then sort" reading would
        // answer `alpha, beta` and fits both observations equally well.
        //
        // Distinctness is by rendered text, which is why `3` and `"3"` are one
        // element; that `[1, "1"].unique()` keeps only the first is unprobed.
        "unique" => {
            arity(name, args, 0, 0)?;
            Ok(Value::List(unique(&items)))
        }
        // The refusal carries the name as it was written, not lowercased.
        _ => Err(unknown_function(name, ty)),
    }
}

/// An Object method call. Its *key access* is the dispatcher's, not this
/// function's.
pub fn call_object(
    ev: &mut Evaluator<'_>,
    subject: &Value,
    name: &str,
    args: &[Expr],
) -> EvalResult {
    let Value::Object(map) = subject else {
        return Err(unknown_function(name, subject.ty()));
    };

    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        // `filter` keeps whole entries and answers an **Object**;
        // `file.properties.filter(/zz/)` renders as `[object Object]`.
        //
        // What the per-entry expression binds is only half settled.
        // Obsidian's own completion text says "which uses the variables `key`
        // and `value`" — `key`, not the List family's `index` — but no probe
        // ever bound one: the only try was a RegExp, which is truthy whatever
        // is in scope. So `key` and `value` are bound here, on the strength of
        // that sentence alone, and `index` is not: it would claim a name
        // Obsidian's own text does not use.
        "filter" => {
            arity(name, args, 1, 1)?;
            let mut out = IndexMap::new();
            for (key, value) in map {
                if ev.in_scope(entry(key, value), &args[0])?.is_truthy() {
                    out.insert(key.clone(), value.clone());
                }
            }
            Ok(Value::Object(out))
        }
        "isempty" => {
            arity(name, args, 0, 0)?;
            Ok(Value::Bool(map.is_empty()))
        }
        // "Returns a list containing the keys of the object", in the map's own
        // order: `file.properties.keys()` comes back in frontmatter order.
        "keys" => {
            arity(name, args, 0, 0)?;
            Ok(Value::List(map.keys().map(Value::str).collect()))
        }
        // `map` answers a **List**, not an Object — "Transform each key value
        // pair of this object into a list": `file.properties.map(/zz/)` on a
        // 22-key object is a List of 22 RegExps.
        "map" => {
            arity(name, args, 1, 1)?;
            let mut out = Vec::with_capacity(map.len());
            for (key, value) in map {
                out.push(ev.in_scope(entry(key, value), &args[0])?);
            }
            Ok(Value::List(out))
        }
        "values" => {
            arity(name, args, 0, 0)?;
            Ok(Value::List(map.values().cloned().collect()))
        }
        _ => Err(unknown_function(name, Type::Object)),
    }
}

// --- the per-element scopes -------------------------------------------------

/// The frame `filter`, `map` and `reduce` run an element under.
fn element(value: &Value, index: usize) -> Scope {
    Scope {
        value: Some(value.clone()),
        index: Some(Value::Number(index as f64)),
        ..Scope::default()
    }
}

/// The frame `Object.filter` and `Object.map` run an entry under: `key` and
/// `value`, which are the two names Obsidian's own completion text gives.
fn entry(key: &str, value: &Value) -> Scope {
    Scope {
        value: Some(value.clone()),
        key: Some(Value::str(key)),
        ..Scope::default()
    }
}

// --- the helpers ------------------------------------------------------------

fn contains(items: &[Value], wanted: &Value) -> bool {
    items.iter().any(|item| loose_eq(item, wanted))
}

/// The numbers in a list, in order. A non-number is skipped, not coerced: the
/// summaries are documented as being over "the number values in the list".
fn numbers(items: &[Value]) -> Vec<f64> {
    items.iter().filter_map(|v| v.as_number()).collect()
}

/// The dates in a list, in order. A String that parses as a date is one.
fn dates(items: &[Value]) -> Vec<DateValue> {
    items
        .iter()
        .filter_map(|v| match v {
            Value::Date(d) => Some(*d),
            Value::Str(s) => crate::funcs::time::parse_date(s),
            _ => None,
        })
        .collect()
}

fn extreme_number(items: &[Value], want: Ordering) -> Value {
    let nums = numbers(items);
    let mut best: Option<f64> = None;
    for n in nums {
        match best {
            Some(b) if n.partial_cmp(&b) != Some(want) => {}
            _ => best = Some(n),
        }
    }
    best.map(Value::Number).unwrap_or(Value::Null)
}

fn extreme_date(items: &[Value], want: Ordering) -> Value {
    let mut best: Option<DateValue> = None;
    for d in dates(items) {
        match best {
            Some(b) if d.at.cmp(&b.at) != want => {}
            _ => best = Some(d),
        }
    }
    best.map(Value::Date).unwrap_or(Value::Null)
}

fn flatten(items: &[Value], out: &mut Vec<Value>) {
    for item in items {
        match item.as_list() {
            Some(inner) => flatten(&inner, out),
            None => out.push(item.clone()),
        }
    }
}

/// `sort()`'s comparator: numbers as numbers, everything else as its rendered
/// text. See the note on `sort` — the mixed case is unprobed.
fn sort_order(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        _ => a.to_string().cmp(&b.to_string()),
    }
}

/// The distinct elements, keyed by rendered text, in the order a plain
/// JavaScript object enumerates those keys. See the note on `unique`.
fn unique(items: &[Value]) -> Vec<Value> {
    let mut groups: IndexMap<String, Value> = IndexMap::new();
    for item in items {
        groups
            .entry(item.to_string())
            .or_insert_with(|| item.clone());
    }
    let mut indexed: Vec<(u32, Value)> = Vec::new();
    let mut rest: Vec<Value> = Vec::new();
    for (key, value) in groups {
        match array_index(&key) {
            Some(i) => indexed.push((i, value)),
            None => rest.push(value),
        }
    }
    indexed.sort_by_key(|(i, _)| *i);
    let mut out: Vec<Value> = indexed.into_iter().map(|(_, v)| v).collect();
    out.append(&mut rest);
    out
}

/// Whether a key is one JavaScript treats as an array index, which is what
/// decides whether it sorts ahead of the rest: a canonical non-negative
/// integer below 2³²-1, with no sign, no leading zero and no decimal point.
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if key.len() > 1 && key.starts_with('0') {
        return None;
    }
    match key.parse::<u32>() {
        Ok(n) if n < u32::MAX => Some(n),
        _ => None,
    }
}

/// A `slice` bound, clamped into the list. A negative is the front, not the
/// back — unprobed, and the reading `list[-1]` supports.
fn clamp_index(n: f64, len: usize) -> usize {
    if n.is_nan() || n <= 0.0 {
        0
    } else if n >= len as f64 {
        len
    } else {
        n as usize
    }
}
