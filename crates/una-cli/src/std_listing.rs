//! The standard library reference listing (#648): every public item of the prelude and of
//! `std.testing`, as the checker reads them in each edition (`una_check::standard_source`),
//! with its declaration and doc comments. The listing is `docs/std/listing.json`; a test
//! regenerates it and fails when the committed file differs. Its schema,
//! `una.std-listing/1`, is unstable and not part of the specification (`docs/std/README.md`).
//!
//! An item whose declaration or doc comments differ between editions has one variant per
//! run of editions where they are the same. Built-in methods have no declaration: they are
//! listed in `docs/std/builtins.json` (`std_builtins`), and operators are not listed. Generating it is linear in the size of the standard
//! modules, once per edition.

use std::collections::BTreeMap;

use serde::Serialize;
use una_project::Edition;
use una_syntax::ast::{Decl, DeclKind, File};
use una_syntax::{Interner, SourceDb, Span, lex_in, parse};

/// The schema of the listing: unstable, not part of the specification.
pub const STD_LISTING_SCHEMA: &str = "una.std-listing/1";

/// The listing, in its key order.
#[derive(Debug, Serialize)]
struct Listing {
    schema: &'static str,
    note: &'static str,
    items: Vec<Item>,
}

/// One public item of a standard module.
#[derive(Debug, Serialize)]
pub struct Item {
    /// Its name.
    pub name: String,
    /// `prelude`, `std.host`, `std.testing` or `std.time`.
    pub module: &'static str,
    /// `function`, `predicate`, `kind`, `capability` or `constant`.
    pub kind: &'static str,
    /// The spec sections its doc comments cite, as `spec/NN §M`, in order of first mention.
    pub spec: Vec<String>,
    /// Its declaration in each run of editions where it is the same.
    pub variants: Vec<Variant>,
}

/// An item's declaration and doc comments in some editions.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Variant {
    /// The editions, oldest first.
    pub editions: Vec<&'static str>,
    /// The declaration without its body: a function's or a predicate's signature and
    /// clauses, a kind's or a capability's whole declaration.
    pub declaration: String,
    /// The doc comments, without their `---` marker.
    pub docs: Vec<String>,
}

/// The standard modules of the listing: the prelude, `std.host` for the names that only an
/// import of it brings into scope (`spec/31 §1`), and `std.testing`.
const MODULES: [&str; 4] = ["prelude", "std.host", "std.testing", TIME];

/// `std.time`, whose constants the listing holds (ADR 0126 rule 11).
const TIME: &str = "std.time";

/// Every public item of the standard modules, in order of first declaration: the prelude's,
/// then `std.testing`'s, each in source order, oldest edition first.
pub fn std_items() -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    // Each item's position in `items`, by module and name: one lookup per declaration.
    let mut index: BTreeMap<(&'static str, String), usize> = BTreeMap::new();
    for edition in Edition::ALL {
        let source = una_check::standard_source(edition);
        let texts = [
            (MODULES[0], source.prelude.as_str(), source.keywords),
            (MODULES[2], source.testing.as_str(), source.keywords),
        ];
        for (module, text, keywords) in texts {
            let mut sources = SourceDb::new();
            let mut interner = Interner::new();
            let id = sources.add(module, text.as_bytes());
            let lexed = lex_in(sources.file(id), id, &mut interner, keywords);
            let tree: File = parse(&lexed, id, &interner).file;
            for decl in &tree.decls {
                let Some((name, kind, declaration)) = public(decl, &interner, text) else {
                    continue;
                };
                // The functions that the methods of dense values resolve to are reached by
                // their receiver only, never by name (`spec/29 §17`): not part of the listing.
                if module == MODULES[0] && name.starts_with("dense_") {
                    continue;
                }
                let docs = docs(decl, text);
                let module =
                    if module == MODULES[0] && una_check::server_names().contains(&name.as_str()) {
                        MODULES[1]
                    } else {
                        module
                    };
                let at = *index.entry((module, name.clone())).or_insert_with(|| {
                    items.push(Item {
                        name,
                        module,
                        kind,
                        spec: Vec::new(),
                        variants: Vec::new(),
                    });
                    items.len() - 1
                });
                if let Some(item) = items.get_mut(at) {
                    add(item, edition.name(), declaration, docs);
                }
            }
        }
    }
    // The constants of `std.time` (ADR 0126 rules 11 and 12): declared in Una from edition
    // 0.5, and the same built-in names, of the same types and values, in edition 0.4.
    let source = una_check::standard_source(Edition::V0_5);
    if let Some((_, text)) = source.later.iter().find(|(path, _)| *path == "std.time") {
        let mut sources = SourceDb::new();
        let mut interner = Interner::new();
        let id = sources.add("std.time", text.as_bytes());
        let lexed = lex_in(
            sources.file(id),
            id,
            &mut interner,
            una_syntax::KeywordSet::V0_5,
        );
        let tree: File = parse(&lexed, id, &interner).file;
        for decl in &tree.decls {
            let DeclKind::Const(constant) = &decl.kind else {
                continue;
            };
            if !constant.public {
                continue;
            }
            let mut item = Item {
                name: interner.resolve(constant.name.name).to_owned(),
                module: TIME,
                kind: "constant",
                spec: Vec::new(),
                variants: Vec::new(),
            };
            let declaration = span_text(text, decl.span);
            for edition in Edition::ALL.into_iter().filter(|e| *e >= Edition::V0_4) {
                add(
                    &mut item,
                    edition.name(),
                    declaration.clone(),
                    docs(decl, text),
                );
            }
            items.push(item);
        }
    }
    // Keep the order of first declaration, but list the prelude's items, then `std.host`'s,
    // before `std.testing`'s, then `std.time`'s: a stable sort by module.
    items.sort_by_key(|item| MODULES.iter().position(|m| *m == item.module));
    for item in &mut items {
        item.spec = cited(&item.variants);
    }
    items
}

/// The listing as `docs/std/listing.json` holds it: pretty JSON, newline-terminated.
pub fn std_listing() -> String {
    let listing = Listing {
        schema: STD_LISTING_SCHEMA,
        note: "Unstable, not part of the specification: see docs/std/README.md. Built-in methods are in docs/std/builtins.json, operators in spec/14 §2.",
        items: std_items(),
    };
    match serde_json::to_string_pretty(&listing) {
        Ok(text) => format!("{text}\n"),
        // INVARIANT: the listing holds only strings and lists, which always serialize.
        Err(error) => format!("{{\"error\": \"{error}\"}}\n"),
    }
}

/// Adds `edition`'s declaration of `item`: to its last variant when that is the same, as a
/// new variant otherwise.
fn add(item: &mut Item, edition: &'static str, declaration: String, docs: Vec<String>) {
    match item.variants.last_mut() {
        Some(last) if last.declaration == declaration && last.docs == docs => {
            last.editions.push(edition);
        }
        _ => item.variants.push(Variant {
            editions: vec![edition],
            declaration,
            docs,
        }),
    }
}

/// A public declaration's name, kind and declaration text without its body.
fn public(decl: &Decl, interner: &Interner, text: &str) -> Option<(String, &'static str, String)> {
    let slice = |start: usize, end: usize| text.get(start..end).unwrap_or("").to_owned();
    let start = decl.span.start;
    match &decl.kind {
        DeclKind::Function(function) if function.public => {
            let mut declaration = slice(start, function.result.span.end);
            for clause in &function.clauses {
                declaration.push_str("\n  ");
                declaration.push_str(&span_text(text, clause.span));
            }
            Some((
                interner.resolve(function.name.name).to_owned(),
                "function",
                declaration,
            ))
        }
        DeclKind::Predicate(predicate) if predicate.public => Some((
            interner.resolve(predicate.name.name).to_owned(),
            "predicate",
            slice(start, predicate.result.span.end),
        )),
        DeclKind::Kind(kind) if kind.public => Some((
            interner.resolve(kind.name.name).to_owned(),
            "kind",
            span_text(text, decl.span),
        )),
        DeclKind::Capability(capability) if capability.public => Some((
            interner.resolve(capability.name.name).to_owned(),
            "capability",
            span_text(text, decl.span),
        )),
        _ => None,
    }
}

/// The text at `span`.
fn span_text(text: &str, span: Span) -> String {
    text.get(span.start..span.end).unwrap_or("").to_owned()
}

/// A declaration's doc comments, without `---` and the space after it.
fn docs(decl: &Decl, text: &str) -> Vec<String> {
    decl.docs
        .iter()
        .map(|doc| {
            let line = span_text(text, doc.span);
            let line = line.strip_prefix("---").unwrap_or(&line);
            line.strip_prefix(' ').unwrap_or(line).to_owned()
        })
        .collect()
}

/// The spec sections that the doc comments of `variants` cite, as `spec/NN §M`, in order of
/// first mention.
pub(crate) fn cited(variants: &[Variant]) -> Vec<String> {
    let mut cited: Vec<String> = Vec::new();
    for variant in variants {
        let text = variant.docs.join(" ");
        let mut rest = text.as_str();
        while let Some((_, after)) = rest.split_once("spec/") {
            rest = after;
            let chapter: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let Some(section) = rest
                .get(chapter.len()..)
                .and_then(|after| after.strip_prefix(" §"))
            else {
                continue;
            };
            let number: String = section.chars().take_while(char::is_ascii_digit).collect();
            if chapter.len() == 2 && !number.is_empty() {
                let reference = format!("spec/{chapter} §{number}");
                if !cited.contains(&reference) {
                    cited.push(reference);
                }
            }
        }
    }
    cited
}
