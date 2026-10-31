//! File and Link.
//!
//! | Receiver | Functions | Properties |
//! | --- | --- | --- |
//! | File | `asLink(display: Any?)`, `hasLink(dest: File,Link,String)`, `hasProperty(property: String)`, `hasTag(values: String)`, `inFolder(folder: String)` | `name`, `basename`, `fullname`, `path`, `folder`, `ext`, `size`, `ctime`, `mtime`, `tags`, `links`, `embeds`, `backlinks`, `properties`, `file` |
//! | Link | `asFile()`, `linksTo(file: File)`, plus the whole String set | `length` |
//!
//! Observed on 2026-10-25 against Obsidian 1.14.5, on a row whose note is
//! `Silica.md` at the vault root
//! (`tools/obsidian-probe/results/2026-10-25-bases-expr.json`,
//! `e14-function-table`, `e17-case-and-namespaces`, `e08-methods`,
//! `e18-completions`):
//!
//! | Expression | Renders | Type |
//! | --- | --- | --- |
//! | `file` / `file.toString()` | `Silica.md` | File / String |
//! | `file.name` | `Silica` | String |
//! | `file.basename` | `Silica` | String |
//! | `file.path` | `Silica.md` | String |
//! | `file.folder` | `/` | String |
//! | `file.ext` | `md` | String |
//! | `file.size` | `483` | Number |
//! | `file.ctime`, `file.mtime` | `2026-10-25T15:28:44` | DateTime |
//! | `file.tags` | `#rock`, `#igneous/extrusive` | Tags |
//! | `file.links` | `[[Pumice\|Pumice]]`, `[[Pumice\|Pumice]]`, `[[Basalt\|Basalt]]` | List of Link |
//! | `file.embeds` | *(empty)* | List |
//! | `file.backlinks` | `[[Pumice.md\|Pumice]]` | List of Link |
//! | `file.properties` | an Object | Object |
//! | `file.file` | `Silica.md` | File |
//! | `file.asLink()` | `Silica.md`, with the Link icon | Link |
//!
//! **`name` is the display name, not the file name.** The row's note is
//! `Silica.md` and `file.name` is `Silica`, while `this.file.name` — `this`
//! being the base file — is `Main.base`, extension and all. So `name` is the
//! file name with a `.md` extension dropped and any other extension kept,
//! which is how Obsidian names a note everywhere else (the view's own "file
//! name" column reads `Bare`, `Basalt`, `Cells.base` in the same run). The
//! host fills [`crate::FileData`] in, so this module only hands the three
//! names back; the reading above is what the field *means*:
//!
//! - `name` the display name (`Silica`, `Main.base`),
//! - `basename` the stem, extension always dropped (`Silica`, `Main`),
//! - `fullname` the file name with its extension (`Silica.md`, `Main.base`) —
//!   `unprobed`: the completion list has it but the run never evaluated it,
//!   and `basename` was only ever seen on a `.md` file, where it equals
//!   `name`.
//!
//! **Tags are the one case-insensitive value in the language.**
//! `file.hasTag("rock")`, `("Rock")` and `("#rock")` are all true on a note
//! tagged `#rock`, where `listish.contains("ALPHA")` is false and
//! `("alpha")` true. `file.tags` carries the `#`.
//!
//! **Arity.** The minimum is checked first, then the parameter's type, then
//! the maximum: `file.hasLink()` is *not enough arguments*, while
//! `file.hasLink(/zz/)`, `(/zz/, /zz/)` and `(/zz/, /zz/, /zz/)` are all
//! `Type error in "hasLink", parameter "dest". Expected File, given RegExp.`
//! — the type error preempts the count. `hasTag` takes any number of values,
//! and `file.hasTag()` with none is `false` rather than a refusal.
//!
//! The rendered strings in the probe are the *cell's* text, which is not
//! always `toString()`: a List cell joins with `,` where
//! `[3, 1, 2].toString()` is `3, 1, 2`, and a Link cell shows the link's text
//! where `linkish.toString()` is `[[Pumice]]`. [`crate::Value`]'s `Display` is
//! `toString()`, so the tests compare against that.

use std::sync::Arc;

use crate::eval::{EvalError, EvalResult, Evaluator};
use crate::expr::Expr;
use crate::value::{FileData, LinkValue, Value};

use super::{
    arity, not_enough, too_many, type_error, unknown_function, unknown_property, want_string,
};

pub fn call(ev: &mut Evaluator<'_>, subject: &Value, name: &str, args: &[Expr]) -> EvalResult {
    let Value::File(file) = subject else {
        return Err(unknown_function(name, subject.ty()));
    };

    match name.to_ascii_lowercase().as_str() {
        // `file.asLink()` is a Link to the file's own path with no alias: the
        // cell renders `Silica.md`, which is the path, and a Link renders its
        // alias when it has one. What a `display` argument does is
        // `unprobed` — the run only ever called it with none — so it becomes
        // the alias, as `link(path, display)` does, and a null one is no alias
        // at all.
        "aslink" => {
            let display = match args.first() {
                None => None,
                Some(arg) => {
                    let given = ev.eval(arg)?;
                    if args.len() > 1 {
                        return Err(too_many(name));
                    }
                    match given {
                        Value::Null => None,
                        other => Some(other.to_string()),
                    }
                }
            };
            Ok(Value::Link(LinkValue {
                path: file.path.clone(),
                display,
            }))
        }

        // "Returns true if `file` links to `otherFile`." Both sides are
        // resolved through the host, because the two spellings differ in the
        // run itself: `file.links` holds the written target (`Pumice`) while a
        // File holds the path (`Pumice.md`).
        //
        // `unprobed`: whether an *embed* counts as a link. It does not here —
        // `links` and `embeds` are separate properties, and the File value
        // carries a separate reader for each.
        "haslink" => {
            if args.is_empty() {
                return Err(not_enough(name));
            }
            let given = ev.eval_args(args)?;
            let want = dest_key(ev, name, &given[0])?;
            if args.len() > 1 {
                return Err(too_many(name));
            }
            Ok(Value::Bool(
                file.links.iter().any(|l| link_key(ev, &l.path) == want),
            ))
        }

        // "Returns true if the note has the given file property." The name is
        // matched case-insensitively, as every name in the language is
        // (`file.properties.STATUS` is `todo`). A property that is present and
        // null still counts: `unprobed`, but it is the note's own key set that
        // the sentence asks about.
        "hasproperty" => {
            if args.is_empty() {
                return Err(not_enough(name));
            }
            let given = ev.eval_args(args)?;
            let want = want_string(name, "property", &given[0])?;
            if args.len() > 1 {
                return Err(too_many(name));
            }
            Ok(Value::Bool(
                file.properties
                    .keys()
                    .any(|k| k.eq_ignore_ascii_case(&want)),
            ))
        }

        // "Returns true if the file has any of the tags in `values`", and with
        // no values at all it is false. Every argument is type-checked even
        // once one has matched, so a refusal does not depend on the data.
        "hastag" => {
            let given = ev.eval_args(args)?;
            let mut hit = false;
            for value in &given {
                let want = want_string(name, "values", value)?;
                hit |= file.tags.iter().any(|tag| tag_matches(tag, &want));
            }
            Ok(Value::Bool(hit))
        }

        // "Returns true if the file is in the specified folder."
        "infolder" => {
            if args.is_empty() {
                return Err(not_enough(name));
            }
            let given = ev.eval_args(args)?;
            let want = want_string(name, "folder", &given[0])?;
            if args.len() > 1 {
                return Err(too_many(name));
            }
            Ok(Value::Bool(in_folder(&file.folder, &want)))
        }

        _ => Err(unknown_function(name, subject.ty())),
    }
}

/// A Link's own two names; everything else is the String set, so this falls
/// through to [`super::text::call`].
///
/// The two readings of the table disagree here and the brute-force pass wins:
/// the formula editor completes a Link with the String set alone, but
/// `linkish.asFile()` really does answer the File `Pumice.md` and
/// `linkish.linksTo()` really does refuse with *not enough arguments*.
/// `isEmpty()` is **not** one of a Link's own: it answered `false` on
/// `[[Pumice]]`, which is what the String one answers for `Pumice`.
pub fn call_link(ev: &mut Evaluator<'_>, subject: &Value, name: &str, args: &[Expr]) -> EvalResult {
    let Value::Link(link) = subject else {
        return super::text::call(ev, subject, name, args);
    };

    match name.to_ascii_lowercase().as_str() {
        // `linkish.asFile()` is the File `Pumice.md` and
        // `linkish.asFile().name` is `Pumice`. A target the host cannot
        // resolve is null: `unprobed`, and the only answer null absorption
        // leaves room for.
        "asfile" => {
            arity(name, args, 0, 0)?;
            Ok(resolve(ev, &link.path)
                .map(Value::File)
                .unwrap_or(Value::Null))
        }

        // "Returns true if the link points to `file`". Only a File is
        // accepted — `linkish.linksTo(/zz/)` is `Expected File, given RegExp`
        // — and whether a Link or a String would be is `unprobed`, so the
        // signature is taken at its word.
        "linksto" => {
            if args.is_empty() {
                return Err(not_enough(name));
            }
            let given = ev.eval_args(args)?;
            let Value::File(target) = &given[0] else {
                return Err(type_error(name, "file", "File", given[0].ty()));
            };
            let want = path_key(&target.path);
            if args.len() > 1 {
                return Err(too_many(name));
            }
            Ok(Value::Bool(link_key(ev, &link.path) == want))
        }

        _ => super::text::call(ev, subject, name, args),
    }
}

pub fn property(_ev: &mut Evaluator<'_>, subject: &Value, name: &str) -> EvalResult {
    let Value::File(file) = subject else {
        return Err(unknown_property(name, subject.ty()));
    };

    Ok(match name.to_ascii_lowercase().as_str() {
        "name" => Value::Str(file.name.clone()),
        "basename" => Value::Str(file.basename.clone()),
        "fullname" => Value::Str(file.fullname.clone()),
        "path" => Value::Str(file.path.clone()),
        "folder" => Value::Str(file.folder.clone()),
        "ext" => Value::Str(file.ext.clone()),
        // Bytes, not characters: the run's `483` is exactly the byte
        // length of the fixture note in `tools/obsidian-probe/bases-expr.mjs`.
        "size" => Value::Number(file.size),
        // Both are DateTimes (`lucide-clock`), not Dates. A file without one
        // is null: `unprobed`, since every real file has both.
        "ctime" => file.ctime.map(Value::Date).unwrap_or(Value::Null),
        "mtime" => file.mtime.map(Value::Date).unwrap_or(Value::Null),
        // Its own type tag, `lucide-tags`, and the `#` is part of each tag.
        "tags" => Value::Tags(file.tags.clone()),
        "links" => links_of(&file.links),
        "embeds" => links_of(&file.embeds),
        "backlinks" => links_of(&file.backlinks),
        "properties" => Value::Object(file.properties.clone()),
        // The File itself: `file.file` renders `Silica.md` with the File icon.
        "file" => Value::File(Arc::clone(file)),
        _ => return Err(unknown_property(name, subject.ty())),
    })
}

fn links_of(links: &[LinkValue]) -> Value {
    Value::List(links.iter().cloned().map(Value::Link).collect())
}

/// `hasLink`'s `dest`, as a comparable key. A File, a Link or a String is
/// accepted and anything else is the observed type error, which names `File`
/// however many types the signature lists.
fn dest_key(ev: &Evaluator<'_>, name: &str, dest: &Value) -> Result<String, EvalError> {
    match dest {
        Value::File(f) => Ok(path_key(&f.path)),
        Value::Link(l) => Ok(link_key(ev, &l.path)),
        Value::Str(s) => Ok(link_key(ev, s)),
        other => Err(type_error(name, "dest", "File", other.ty())),
    }
}

/// The key a link target compares by: the resolved file's path when the host
/// resolves it, the written target otherwise. A subpath (`Note#Heading`) and
/// an alias tail are cut off first, and the comparison is case-insensitive,
/// as every vault path comparison is.
fn link_key(ev: &Evaluator<'_>, target: &str) -> String {
    let head = target.split(['#', '|']).next().unwrap_or(target).trim();
    match ev.context().lookup_file(head) {
        Some(f) => path_key(&f.path),
        None => path_key(head),
    }
}

fn path_key(path: &str) -> String {
    path.trim().to_lowercase()
}

/// A file resolved from a link target, for `asFile()`.
fn resolve(ev: &Evaluator<'_>, target: &str) -> Option<Arc<FileData>> {
    let head = target.split(['#', '|']).next().unwrap_or(target).trim();
    ev.context().lookup_file(head)
}

/// Whether one of the file's tags answers `hasTag(want)`. The `#` is optional
/// on either side and the match ignores case — the one case-insensitive value
/// comparison in the language.
///
/// `unprobed`: whether a parent matches a nested tag. It does here, because
/// that is what Obsidian's tags are everywhere else — `tag:#igneous` in search
/// finds `#igneous/extrusive`, and `silica-search` already matches "an exact
/// tag or a parent of a nested tag". The run only ever matched `#rock`
/// exactly.
fn tag_matches(tag: &str, want: &str) -> bool {
    let tag = tag.trim().trim_start_matches('#');
    let want = want.trim().trim_start_matches('#');
    if want.is_empty() {
        return false;
    }
    if tag.eq_ignore_ascii_case(want) {
        return true;
    }
    tag.get(..want.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(want))
        && tag.as_bytes().get(want.len()) == Some(&b'/')
}

/// Whether a file whose folder is `folder` is in `want`.
///
/// `unprobed` beyond the one observation that a root file's `folder` is `/`:
/// the folder is matched the way a path is matched, so the root (`/` or the
/// empty string) holds every file and a folder holds the files of its
/// subfolders. Case-insensitively, as vault paths compare.
fn in_folder(folder: &str, want: &str) -> bool {
    let folder = folder.trim().trim_matches('/');
    let want = want.trim().trim_matches('/');
    if want.is_empty() {
        return true;
    }
    if folder.eq_ignore_ascii_case(want) {
        return true;
    }
    folder
        .get(..want.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(want))
        && folder.as_bytes().get(want.len()) == Some(&b'/')
}
