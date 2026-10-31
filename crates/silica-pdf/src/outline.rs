//! The document outline: `/Outlines` → `/First`/`/Next` chains, each item's
//! `/Dest` or `/A` GoTo action resolved to a page index.

use std::collections::{HashMap, HashSet};

use hayro::hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, ObjRef, Object, ObjectIdentifier, String as PdfString,
};
use hayro::hayro_syntax::xref::XRef;
use hayro::hayro_syntax::Pdf;

use crate::textstring;
use crate::OutlineItem;

/// Deeper items are skipped: no real outline nests this far, and a malformed
/// one must not recurse without end.
const MAX_DEPTH: usize = 64;
/// The most items read from one outline, a guard against a `/Next` loop of
/// direct (unreferenced, so unvisitable) dictionaries.
const MAX_ITEMS: usize = 50_000;
/// How many named-destination indirections to follow.
const MAX_NAMED_HOPS: usize = 8;
/// How deep the `/Names` tree is walked.
const MAX_TREE_DEPTH: usize = 32;

pub(crate) fn outline(pdf: &Pdf) -> Vec<OutlineItem> {
    let xref = pdf.xref();
    let Some(catalog) = xref.get::<Dict<'_>>(xref.root_id()) else {
        return Vec::new();
    };
    let Some(outlines) = catalog.get::<Dict<'_>>(b"Outlines") else {
        return Vec::new();
    };
    let pages: HashMap<ObjectIdentifier, usize> = pdf
        .pages()
        .iter()
        .enumerate()
        .filter_map(|(index, page)| page.raw().obj_id().map(|id| (id, index)))
        .collect();
    let dests = Dests {
        xref,
        catalog: &catalog,
        pages,
        page_count: pdf.pages().len(),
    };

    let mut items = Vec::new();
    let mut visited: HashSet<ObjectIdentifier> = HashSet::new();
    let mut stack: Vec<(Dict<'_>, usize)> = Vec::new();
    if let Some(first) = outlines.get::<Dict<'_>>(b"First") {
        stack.push((first, 0));
    }
    while let Some((item, depth)) = stack.pop() {
        if items.len() >= MAX_ITEMS {
            break;
        }
        if let Some(id) = item.obj_id() {
            if !visited.insert(id) {
                continue;
            }
        }
        // The sibling goes under the child so the child pops first.
        if let Some(next) = item.get::<Dict<'_>>(b"Next") {
            stack.push((next, depth));
        }
        if depth < MAX_DEPTH {
            if let Some(first) = item.get::<Dict<'_>>(b"First") {
                stack.push((first, depth + 1));
            }
        }
        let title = item
            .get::<PdfString<'_>>(b"Title")
            .map(|s| textstring::decode(s.as_bytes()))
            .unwrap_or_default();
        items.push(OutlineItem {
            title,
            page: dests.page_of_item(&item),
            depth,
        });
    }
    items
}

struct Dests<'a, 'd> {
    xref: &'a XRef,
    catalog: &'d Dict<'a>,
    pages: HashMap<ObjectIdentifier, usize>,
    page_count: usize,
}

impl<'a> Dests<'a, '_> {
    fn page_of_item(&self, item: &Dict<'a>) -> Option<usize> {
        if let Some(dest) = item.get::<Object<'a>>(b"Dest") {
            return self.resolve(dest, 0);
        }
        let action = item.get::<Dict<'a>>(b"A")?;
        if action.get::<Name<'a>>(b"S").as_deref() != Some(b"GoTo".as_slice()) {
            return None;
        }
        self.resolve(action.get::<Object<'a>>(b"D")?, 0)
    }

    /// A destination is an explicit array, a dictionary wrapping one under
    /// `/D`, or a name or string looked up in the catalog.
    fn resolve(&self, dest: Object<'a>, hops: usize) -> Option<usize> {
        if hops > MAX_NAMED_HOPS {
            return None;
        }
        match dest {
            Object::Array(array) => self.page_of_target(array.raw_iter().next()?),
            Object::Dict(dict) => self.resolve(dict.get::<Object<'a>>(b"D")?, hops + 1),
            Object::Name(name) => self.named(&name, hops),
            Object::String(string) => self.named(string.as_bytes(), hops),
            _ => None,
        }
    }

    /// The first element of a destination array: a page reference, or a page
    /// index in a remote go-to that points back into this file.
    fn page_of_target(&self, target: MaybeRef<Object<'a>>) -> Option<usize> {
        match target {
            MaybeRef::Ref(reference) => self.pages.get(&reference.into()).copied(),
            MaybeRef::NotRef(Object::Number(number)) => {
                let index = number.as_i64();
                (index >= 0 && (index as usize) < self.page_count).then_some(index as usize)
            }
            MaybeRef::NotRef(_) => None,
        }
    }

    fn named(&self, key: &[u8], hops: usize) -> Option<usize> {
        // PDF 1.1 kept named destinations in a plain dictionary.
        if let Some(dests) = self.catalog.get::<Dict<'a>>(b"Dests") {
            if let Some(dest) = dests.get::<Object<'a>>(key) {
                return self.resolve(dest, hops + 1);
            }
        }
        let tree = self
            .catalog
            .get::<Dict<'a>>(b"Names")?
            .get::<Dict<'a>>(b"Dests")?;
        let dest = self.lookup_in_tree(&tree, key, 0)?;
        self.resolve(dest, hops + 1)
    }

    /// A name tree node holds either `/Names [key value key value ...]` or
    /// `/Kids`, each kid with `/Limits [lowest highest]`.
    fn lookup_in_tree(&self, node: &Dict<'a>, key: &[u8], depth: usize) -> Option<Object<'a>> {
        if depth > MAX_TREE_DEPTH {
            return None;
        }
        if let Some(names) = node.get::<Array<'a>>(b"Names") {
            let mut entries = names.raw_iter();
            while let (Some(name), Some(value)) = (entries.next(), entries.next()) {
                let matches = match self.deref(name) {
                    Some(Object::String(s)) => s.as_bytes() == key,
                    _ => false,
                };
                if matches {
                    return self.deref(value);
                }
            }
        }
        if let Some(kids) = node.get::<Array<'a>>(b"Kids") {
            for kid in kids.iter::<Dict<'a>>() {
                if let Some(limits) = kid.get::<Array<'a>>(b"Limits") {
                    let mut bounds = limits.iter::<PdfString<'a>>();
                    if let (Some(low), Some(high)) = (bounds.next(), bounds.next()) {
                        if key < low.as_bytes() || key > high.as_bytes() {
                            continue;
                        }
                    }
                }
                if let Some(found) = self.lookup_in_tree(&kid, key, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn deref(&self, object: MaybeRef<Object<'a>>) -> Option<Object<'a>> {
        match object {
            MaybeRef::Ref(reference) => self.xref.get::<Object<'a>>(as_id(reference)),
            MaybeRef::NotRef(object) => Some(object),
        }
    }
}

fn as_id(reference: ObjRef) -> ObjectIdentifier {
    reference.into()
}
