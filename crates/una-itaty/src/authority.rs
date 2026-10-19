//! A package's interface as a release statement states it (ADR 0122 rule 37), computed from
//! its archive, so the registry can refuse a statement that advertises less than the code
//! needs: its `authority`, the union of the `uses` of its public functions and of its
//! handlers, each capability with its operations; and its `trust`, its trusted modules.
//!
//! A public function's `uses` is always written (`spec/05 §9` rule 2: only private functions
//! infer theirs), so the authority is read from the syntax tree, with no type checking: every
//! `uses` clause of a `pub` function's header, and of every handler, public or not. A handler
//! counts whether or not it is public, which can only make the authority larger. Trusted
//! modules are those `ita.una` lists under `trusted` and those whose header says `trusted`.
//!
//! A `uses` clause with no operations needs every operation of its capability (`spec/05 §3`),
//! so it counts as all of them: those the package's own declaration of the capability lists,
//! or, for a capability of the prelude, those the prelude lists. A clause with no operations
//! of a capability declared elsewhere, whose operations the archive does not show, refuses the
//! package.
//!
//! A module that does not lex or parse refuses the package: the interface of code that does
//! not read cannot be computed, and the registry logs nothing it cannot verify.
//!
//! Costs: one lexing and parsing pass per module, O(B) for B bytes of source, on a thread with
//! the parser's stack.

use std::collections::{BTreeMap, BTreeSet};

use una_syntax::ast::{ClauseKind, DeclKind, UsesClause};
use una_syntax::token::KeywordSet;
use una_syntax::{FileId, Interner, SourceDb, lex_in, parse};

/// The stack the parser needs on unoptimized builds, with room to spare
/// (`una_syntax::parse`).
const PARSER_STACK: usize = 64 << 20;

/// What a release statement states of its code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    /// Each capability used, with its operations.
    pub authority: BTreeMap<String, BTreeSet<String>>,
    /// The trusted modules.
    pub trust: BTreeSet<String>,
}

/// The interface of the package whose `ita.una` lists `trusted` and whose modules are `files`
/// (paths under `src/` ending in `.una`, with their bytes; other files are ignored).
pub fn interface(trusted: &[String], files: &[(String, Vec<u8>)]) -> Result<Interface, String> {
    let modules: Vec<(String, Vec<u8>)> = files
        .iter()
        .filter(|(path, _)| path.starts_with("src/") && path.ends_with(".una"))
        .cloned()
        .collect();
    let trusted = trusted.to_vec();
    std::thread::Builder::new()
        .name("itaty-interface".to_owned())
        .stack_size(PARSER_STACK)
        .spawn(move || compute(&trusted, &modules))
        .map_err(|e| format!("cannot start the parser: {e}"))?
        .join()
        .map_err(|_| "the parser stopped".to_owned())?
}

/// The prelude's capabilities, read by the same parser from the files the checker uses.
const PRELUDES: [(&str, &str); 2] = [
    (
        "prelude.una",
        include_str!("../../una-check/src/std/prelude.una"),
    ),
    (
        "prelude_0_2.una",
        include_str!("../../una-check/src/std/prelude_0_2.una"),
    ),
];

/// Each capability declared by `files`, with all its operations.
type Declared = BTreeMap<String, BTreeSet<String>>;

fn declared(interner: &Interner, decls: &[una_syntax::ast::Decl], into: &mut Declared) {
    for decl in decls {
        if let DeclKind::Capability(capability) = &decl.kind {
            into.entry(interner.resolve(capability.name.name).to_owned())
                .or_default()
                .extend(
                    capability
                        .ops
                        .iter()
                        .map(|op| interner.resolve(op.name).to_owned()),
                );
        }
    }
}

fn add(
    authority: &mut BTreeMap<String, BTreeSet<String>>,
    interner: &Interner,
    uses: &UsesClause,
    own: &Declared,
    prelude: &Declared,
) -> Result<(), String> {
    let name = interner.resolve(uses.cap_type.name.name).to_owned();
    let ops: BTreeSet<String> = if uses.cap_type.ops.is_empty() {
        own.get(&name)
            .or_else(|| prelude.get(&name))
            .filter(|ops| !ops.is_empty())
            .cloned()
            .ok_or_else(|| {
                format!(
                    "`uses` of `{name}` lists no operations, and the package does not show which `{name}` has: list them"
                )
            })?
    } else {
        uses.cap_type
            .ops
            .iter()
            .map(|op| interner.resolve(op.name).to_owned())
            .collect()
    };
    authority.entry(name).or_default().extend(ops);
    Ok(())
}

fn compute(trusted: &[String], modules: &[(String, Vec<u8>)]) -> Result<Interface, String> {
    let mut out = Interface {
        trust: trusted.iter().cloned().collect(),
        ..Interface::default()
    };
    let mut interner = Interner::new();
    let mut db = SourceDb::new();
    let ids: Vec<(FileId, &String)> = modules
        .iter()
        .map(|(path, bytes)| (db.add(path.clone(), bytes), path))
        .collect();
    let mut prelude = Declared::new();
    for (path, text) in PRELUDES {
        let id = db.add(path, text.as_bytes());
        let lexed = lex_in(db.file(id), id, &mut interner, KeywordSet::V0_4);
        let parsed = parse(&lexed, id, &interner);
        if !lexed.errors.is_empty() || !parsed.errors.is_empty() {
            return Err(format!("the prelude `{path}` does not read"));
        }
        declared(&interner, &parsed.file.decls, &mut prelude);
    }
    let mut trees = Vec::with_capacity(ids.len());
    for (id, path) in ids {
        let lexed = lex_in(db.file(id), id, &mut interner, KeywordSet::V0_4);
        if !lexed.errors.is_empty() {
            return Err(format!("`{path}` does not lex, so its authority cannot be computed"));
        }
        let parsed = parse(&lexed, id, &interner);
        if !parsed.errors.is_empty() {
            return Err(format!("`{path}` does not parse, so its authority cannot be computed"));
        }
        let header = parsed
            .file
            .module
            .as_ref()
            .ok_or_else(|| format!("`{path}` has no module header"))?;
        if header.trusted {
            let module: Vec<&str> = header
                .path
                .segments
                .iter()
                .map(|segment| interner.resolve(segment.name))
                .collect();
            out.trust.insert(module.join("."));
        }
        trees.push(parsed.file);
    }
    let mut own = Declared::new();
    for tree in &trees {
        declared(&interner, &tree.decls, &mut own);
    }
    for tree in &trees {
        for decl in &tree.decls {
            match &decl.kind {
                DeclKind::Function(function) if function.public => {
                    for clause in &function.clauses {
                        if let ClauseKind::Uses(uses) = &clause.kind {
                            add(&mut out.authority, &interner, uses, &own, &prelude)?;
                        }
                    }
                }
                DeclKind::Handler(handler) => {
                    for uses in &handler.uses {
                        add(&mut out.authority, &interner, uses, &own, &prelude)?;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str) -> (String, Vec<u8>) {
        (path.to_owned(), text.as_bytes().to_vec())
    }

    #[test]
    fn public_uses_and_trusted_modules_are_the_interface() {
        let files = vec![
            file(
                "src/fetch.una",
                "module ledger.fetch\n\npub page(url: Text) -> Text\n  uses @net: Http[get]\n=\n  url\n\nquiet(url: Text) -> Text\n  uses @net: Http[post]\n=\n  url\n",
            ),
            file("src/keys.una", "trusted module ledger.keys\n\npub one() -> Int = 1\n\npub say(t: Text) -> Unit\n  uses @c: Console\n=\n  @c.print(t)\n"),
            file("README.md", "not code"),
        ];
        let found = interface(&["ledger.audit".to_owned()], &files).unwrap();
        let ops: Vec<&str> = found.authority["Http"].iter().map(String::as_str).collect();
        assert_eq!(ops, vec!["get"], "a private function's uses is not the interface");
        let all: Vec<&str> = found.authority["Console"].iter().map(String::as_str).collect();
        assert_eq!(all, vec!["print"], "no operations is all of them");
        assert_eq!(
            found.trust.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["ledger.audit", "ledger.keys"]
        );
    }

    #[test]
    fn code_that_does_not_parse_refuses() {
        let files = vec![file("src/bad.una", "module bad\n\npub (\n")];
        assert!(interface(&[], &files).is_err());
    }
}
