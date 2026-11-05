//! Checking past a broken declaration (ADR 0096, part C, #575).
//!
//! A file with a syntax error is no longer left out of the later phases as a whole. Its
//! declarations that parsed are resolved and typed; the declarations that hold a syntax error
//! are left out, and the names they would have declared are poisoned: a reference to one is
//! unknown, never U1001 (rule 8). A declaration that reaches a poisoned name is a dependent: its
//! diagnostics, but syntax, are withheld (rule 9, as amended on 2026-10-12), since the error type
//! does not absorb every cascade from a broken declaration. Each declaration that parsed and is not a dependent
//! is compared with its canonical text (rule 10).
//!
//! Every step is one pass over the text or over the declarations' words, and each lookup of the
//! declaration that holds a byte offset is a binary search over the declarations, so a check
//! stays linear in the source up to that logarithmic factor: O(n log n) for `n` declarations
//! and errors, where rule 12 of ADR 0096 says linear (`docs/compiler/architecture.md` §5,
//! #1573). The repairs that the parser attaches are bounded the same way (`una_syntax::rewrite`).

use std::collections::{BTreeMap, BTreeSet};

use una_check::Poison;
use una_diag::{Diagnostic, Position};
use una_syntax::{Declaration, DeclarationKind, KeywordSet, SourceDb, declarations_in};

/// The declarations of one file and which of them hold a syntax error.
pub(crate) struct BrokenFile {
    /// The declarations of the file, as the parser's recovery delimits them.
    pub(crate) declarations: Vec<Declaration>,
    /// The indices of the declarations that hold a syntax error.
    pub(crate) broken: BTreeSet<usize>,
    /// Whether the module header is broken, or an error lies outside every declaration: the
    /// module is then broken as a whole, as before ADR 0096.
    pub(crate) whole: bool,
}

impl BrokenFile {
    /// The declarations of `text`, read under `keywords`, that hold one of the syntax errors
    /// at the byte offsets `errors`.
    pub(crate) fn new(text: &str, errors: &[usize], keywords: KeywordSet) -> BrokenFile {
        let declarations = declarations_in(text, keywords);
        let mut broken = BTreeSet::new();
        let mut whole = declarations.is_empty();
        for &at in errors {
            match last_starting_by(&declarations, at) {
                Some(index) => {
                    if declarations
                        .get(index)
                        .is_some_and(|d| d.what == DeclarationKind::Header)
                    {
                        whole = true;
                    }
                    broken.insert(index);
                }
                None => whole = true,
            }
        }
        BrokenFile {
            declarations,
            broken,
            whole,
        }
    }

    fn broken_declarations(&self) -> impl Iterator<Item = &Declaration> {
        self.broken.iter().filter_map(|&i| self.declarations.get(i))
    }

    /// Whether the byte offset `at` lies in a broken declaration. The declarations follow one
    /// another in the text without overlapping, so the only one that can hold `at` is the last
    /// that starts at or before it: a binary search, O(log d) for `d` declarations, where a scan
    /// of the broken ones made the check of a file quadratic, one scan per declaration (#1573).
    pub(crate) fn in_broken(&self, at: usize) -> bool {
        last_starting_by(&self.declarations, at).is_some_and(|index| {
            self.broken.contains(&index) && self.declarations.get(index).is_some_and(|d| at < d.end)
        })
    }

    /// The poisoned names of the module (rule 8).
    pub(crate) fn poison(&self) -> Poison {
        let mut poison = Poison::default();
        for declaration in self.broken_declarations() {
            poison.names.extend(declaration.names.iter().cloned());
            if declaration.what == DeclarationKind::Unnamed {
                poison.unnamed = true;
            }
        }
        poison
    }

    /// The text without its broken declarations, and for each kept declaration that can be
    /// compared with its canonical text, its first line in that text, its first line in the
    /// file, and its number of lines up to its last line that is not blank.
    pub(crate) fn kept(&self, text: &str) -> (String, Vec<Kept>) {
        let first = self.declarations.first().map_or(text.len(), |d| d.start);
        let mut out = text.get(..first).unwrap_or("").to_owned();
        let mut line = out.matches('\n').count();
        let mut kept = Vec::new();
        for (index, declaration) in self.declarations.iter().enumerate() {
            if self.broken.contains(&index) {
                continue;
            }
            let body = text.get(declaration.start..declaration.end).unwrap_or("");
            if matches!(
                declaration.what,
                DeclarationKind::Named | DeclarationKind::Unnamed
            ) {
                kept.push(Kept {
                    kept_line: line,
                    file_line: declaration.first_line,
                    lines: declaration.last_line - declaration.first_line,
                });
            }
            let before = out.len();
            out.push_str(body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
            line += out.get(before..).unwrap_or("").matches('\n').count();
        }
        (out, kept)
    }
}

/// The index of the last of `declarations` that starts at or before the byte offset `at`, if
/// any. `declarations_in` delimits them in the order of the text, each starting on a later line
/// than the one before, so their starts increase strictly and a binary search finds it: O(log d).
fn last_starting_by(declarations: &[Declaration], at: usize) -> Option<usize> {
    declarations
        .partition_point(|d| d.start <= at)
        .checked_sub(1)
}

/// A declaration kept in the text without the broken ones (0-based lines).
pub(crate) struct Kept {
    kept_line: usize,
    file_line: usize,
    lines: usize,
}

/// The U7001 diagnostics of the declarations of a file with a syntax error (rule 10): the
/// differences between `kept` and `canonical`, its canonical form, that lie within one kept
/// declaration, moved to that declaration's lines in the file `path`. The blank lines between
/// declarations, the order of the imports and the final line break wait until the file parses.
pub(crate) fn canonical_by_declaration(
    path: &str,
    kept_text: &str,
    canonical: &str,
    kept: &[Kept],
) -> Vec<Diagnostic> {
    let mut sources = SourceDb::new();
    let id = sources.add(path, kept_text.as_bytes());
    let mut out = Vec::new();
    for mut diagnostic in una_fmt::differences(&sources, id, canonical) {
        let start = diagnostic.primary.start.line as usize;
        let Some(declaration) = kept
            .iter()
            .find(|k| k.kept_line < start && start <= k.kept_line + k.lines)
        else {
            continue;
        };
        let last = declaration.kept_line + declaration.lines;
        let within = |position: &Position| {
            let line = position.line as usize;
            line <= last || (line == last + 1 && position.col == 1)
        };
        let edits_within = diagnostic
            .repairs
            .iter()
            .filter_map(|repair| repair.edit.as_ref())
            .all(|edit| within(&edit.location.start) && within(&edit.location.end));
        if !within(&diagnostic.primary.end) || !edits_within {
            continue;
        }
        let delta = declaration.file_line as i64 - declaration.kept_line as i64;
        let shift = |position: &mut Position| {
            position.line =
                u32::try_from(i64::from(position.line) + delta).unwrap_or(position.line);
        };
        shift(&mut diagnostic.primary.start);
        shift(&mut diagnostic.primary.end);
        for repair in &mut diagnostic.repairs {
            if let Some(edit) = repair.edit.as_mut() {
                shift(&mut edit.location.start);
                shift(&mut edit.location.end);
            }
        }
        diagnostic.message = shift_numbers(
            &diagnostic.message,
            (declaration.kept_line + 1)..=(last + 1),
            delta,
        );
        out.push(diagnostic);
    }
    out
}

/// `message` with each number in `lines` moved by `delta`: the line numbers of a U7001 message.
fn shift_numbers(message: &str, lines: std::ops::RangeInclusive<usize>, delta: i64) -> String {
    let mut out = String::new();
    let mut number = String::new();
    let flush = |number: &mut String, out: &mut String| {
        if number.is_empty() {
            return;
        }
        match number.parse::<usize>() {
            Ok(n) if lines.contains(&n) => {
                out.push_str(&(n as i64 + delta).to_string());
            }
            _ => out.push_str(number),
        }
        number.clear();
    };
    for c in message.chars() {
        if c.is_ascii_digit() {
            number.push(c);
        } else {
            flush(&mut number, &mut out);
            out.push(c);
        }
    }
    flush(&mut number, &mut out);
    out
}

/// A module of the package, for [`withhold`]: its path, its module path, its declarations and
/// which of them are broken.
pub(crate) struct ModuleText<'a> {
    pub(crate) path: &'a str,
    pub(crate) module: &'a str,
    pub(crate) declarations: &'a [Declaration],
    pub(crate) broken: &'a BTreeSet<usize>,
}

/// Drops every diagnostic but syntax of each dependent declaration (rule 9, as amended): one
/// that names a poisoned name, or a dependent one. A word of a declaration refers to a name
/// only where the name is visible: declared in the declaration's own module, or imported by
/// that module from the module that declares it. A word of another module that only spells
/// the name, a parameter of a module that imports nothing from the broken one, refers to
/// nothing. Linear in the words of the declarations.
pub(crate) fn withhold(diagnostics: &mut Vec<Diagnostic>, modules: &[ModuleText<'_>]) {
    // The declarations that are neither broken nor a header or an import, by module.
    let mut nodes: Vec<(usize, &Declaration)> = Vec::new();
    // The names each module declares or imports, and the module that declares them.
    let mut visible: Vec<BTreeMap<&str, &str>> = vec![BTreeMap::new(); modules.len()];
    // The names that are poisoned, or declared by a dependent, by declaring module.
    let mut queue: Vec<(&str, &str)> = Vec::new();
    // The modules with a broken declaration whose names cannot be read.
    let mut unnamed: BTreeSet<&str> = BTreeSet::new();
    for (m, module) in modules.iter().enumerate() {
        for (i, declaration) in module.declarations.iter().enumerate() {
            let names = declaration.names.iter().map(String::as_str);
            match declaration.what {
                DeclarationKind::Header => {}
                // A broken import line poisons each name it lists, in its own module.
                DeclarationKind::Import if module.broken.contains(&i) => {
                    if let Some(slot) = visible.get_mut(m) {
                        for name in names {
                            slot.insert(name, module.module);
                            queue.push((module.module, name));
                        }
                    }
                }
                DeclarationKind::Import => {
                    if let (Some(from), Some(slot)) =
                        (declaration.module.as_deref(), visible.get_mut(m))
                    {
                        for name in names {
                            slot.insert(name, from);
                        }
                    }
                }
                _ if module.broken.contains(&i) => {
                    if declaration.what == DeclarationKind::Unnamed {
                        unnamed.insert(module.module);
                    }
                    if let Some(slot) = visible.get_mut(m) {
                        for name in names {
                            slot.insert(name, module.module);
                            queue.push((module.module, name));
                        }
                    }
                }
                _ => {
                    if let Some(slot) = visible.get_mut(m) {
                        for name in names {
                            slot.insert(name, module.module);
                        }
                    }
                    nodes.push((m, declaration));
                }
            }
        }
    }
    if queue.is_empty() && unnamed.is_empty() {
        return;
    }
    // The names the broken declarations would have declared.
    let poisoned: BTreeSet<&str> = queue.iter().map(|(_, name)| *name).collect();
    // The modules that may name a name of a module with an unnamed broken declaration: that
    // module, and every module importing from it. One pass over the modules, not per node.
    let reaches_unnamed: Vec<bool> = modules
        .iter()
        .zip(&visible)
        .map(|(module, names)| {
            unnamed.contains(module.module) || names.values().any(|from| unnamed.contains(from))
        })
        .collect();
    // Who refers to each visible name, by its declaring module.
    let mut mentions: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
    let mut dependent = vec![false; nodes.len()];
    for (n, (m, declaration)) in nodes.iter().enumerate() {
        let Some(names) = visible.get(*m) else {
            continue;
        };
        let own = modules.get(*m).map_or("", |module| module.module);
        for word in &declaration.words {
            if let Some(&from) = names.get(word.as_str()) {
                mentions.entry((from, word.as_str())).or_default().push(n);
            }
        }
        // A module with an unnamed broken declaration, or one importing from it, may name
        // any of its names: every declaration there is a dependent.
        if reaches_unnamed.get(*m).copied().unwrap_or(false)
            && let Some(slot) = dependent.get_mut(n)
        {
            *slot = true;
            queue.extend(declaration.names.iter().map(|name| (own, name.as_str())));
        }
    }
    let mut seen: BTreeSet<(&str, &str)> = BTreeSet::new();
    while let Some(key) = queue.pop() {
        if !seen.insert(key) {
            continue;
        }
        for &n in mentions.get(&key).map_or(&[][..], Vec::as_slice) {
            if dependent.get(n).copied().unwrap_or(true) {
                continue;
            }
            if let Some(slot) = dependent.get_mut(n) {
                *slot = true;
            }
            if let Some((m, declaration)) = nodes.get(n) {
                let own = modules.get(*m).map_or("", |module| module.module);
                queue.extend(declaration.names.iter().map(|name| (own, name.as_str())));
            }
        }
    }
    let ranges: Vec<(&str, usize, usize)> = nodes
        .iter()
        .zip(&dependent)
        .filter(|(_, d)| **d)
        .filter_map(|((m, declaration), _)| {
            modules
                .get(*m)
                .map(|module| (module.path, declaration.first_line, declaration.last_line))
        })
        .collect();
    diagnostics.retain(|diagnostic| {
        // Rule 9: a dependent withholds its effect, flow and canonical-form diagnostics, which
        // read a header that the broken callee leaves incomplete. Its name and type
        // diagnostics stay, since an unknown name reports nothing beyond U1001 (#581, which
        // relaxes the ADR 0096 amendment of 2026-10-12).
        let code = diagnostic.code.as_str();
        // A U1001 at a poisoned name, which type checking reports for a variant or a
        // method-syntax call where name resolution has none, and U1006 at a poisoned
        // requirement, are withheld too (rule 8).
        let at_poisoned = matches!(code, "U1001" | "U1006")
            && poisoned
                .iter()
                .any(|name| diagnostic.message.contains(&format!("`{name}`")));
        if !(code.starts_with("U3") || code.starts_with("U4") || code == "U7001" || at_poisoned) {
            return true;
        }
        // `last_line` is exclusive: just past the declaration's last line that is not blank.
        let line = (diagnostic.primary.start.line as usize).saturating_sub(1);
        !ranges.iter().any(|(path, first, last)| {
            *path == diagnostic.primary.file && *first <= line && line < *last
        })
    });
}
