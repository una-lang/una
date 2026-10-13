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
//! Every step is one pass over the text or over the declarations' words, so a check stays
//! linear in the source (rule 12).

use std::collections::{BTreeMap, BTreeSet};

use una_check::Poison;
use una_diag::{Diagnostic, Position};
use una_syntax::{Declaration, DeclarationKind, SourceDb, declarations};

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
    /// The declarations of `text` that hold one of the syntax errors at the byte offsets
    /// `errors`.
    pub(crate) fn new(text: &str, errors: &[usize]) -> BrokenFile {
        let declarations = declarations(text);
        let mut broken = BTreeSet::new();
        let mut whole = declarations.is_empty();
        for &at in errors {
            match declarations.iter().rposition(|d| d.start <= at) {
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

    /// Whether the byte offset `at` lies in a broken declaration.
    pub(crate) fn in_broken(&self, at: usize) -> bool {
        self.broken_declarations()
            .any(|d| d.start <= at && at < d.end)
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

/// Drops every diagnostic but syntax of each dependent declaration (rule 9, as amended): one that names a poisoned name, or a dependent one, through
/// a word of its text, which can only make more declarations dependent than reach one.
pub(crate) fn withhold(diagnostics: &mut Vec<Diagnostic>, modules: &[ModuleText<'_>]) {
    // The declarations that are neither broken nor a header or an import, by module.
    let mut nodes: Vec<(usize, &Declaration)> = Vec::new();
    let mut poisoned: BTreeSet<&str> = BTreeSet::new();
    let mut unnamed_modules: BTreeSet<String> = BTreeSet::new();
    for (m, module) in modules.iter().enumerate() {
        for (i, declaration) in module.declarations.iter().enumerate() {
            if module.broken.contains(&i) {
                poisoned.extend(declaration.names.iter().map(String::as_str));
                if declaration.what == DeclarationKind::Unnamed {
                    unnamed_modules.insert(module.module.to_owned());
                }
            } else if matches!(
                declaration.what,
                DeclarationKind::Named | DeclarationKind::Unnamed
            ) {
                nodes.push((m, declaration));
            }
        }
    }
    // A name imported from a module with an unnamed broken declaration may be one it would
    // have declared.
    for module in modules {
        for declaration in module.declarations {
            if declaration.what == DeclarationKind::Import
                && unnamed_modules
                    .iter()
                    .any(|path| declaration.imports_from(path))
            {
                poisoned.extend(declaration.names.iter().map(String::as_str));
            }
        }
    }
    if poisoned.is_empty() && unnamed_modules.is_empty() {
        return;
    }
    // Who mentions each name declared by a node.
    let declared: BTreeSet<&str> = nodes
        .iter()
        .flat_map(|(_, d)| d.names.iter().map(String::as_str))
        .chain(poisoned.iter().copied())
        .collect();
    let mut mentions: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (n, (_, declaration)) in nodes.iter().enumerate() {
        for word in &declaration.words {
            if declared.contains(word.as_str()) {
                mentions.entry(word.as_str()).or_default().push(n);
            }
        }
    }
    let mut dependent = vec![false; nodes.len()];
    let mut queue: Vec<&str> = poisoned.iter().copied().collect();
    for (n, (m, _)) in nodes.iter().enumerate() {
        if modules
            .get(*m)
            .is_some_and(|module| unnamed_modules.contains(module.module))
            && let Some(slot) = dependent.get_mut(n)
        {
            *slot = true;
            if let Some((_, declaration)) = nodes.get(n) {
                queue.extend(declaration.names.iter().map(String::as_str));
            }
        }
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    while let Some(name) = queue.pop() {
        if !seen.insert(name) {
            continue;
        }
        for &n in mentions.get(name).map_or(&[][..], Vec::as_slice) {
            if dependent.get(n).copied().unwrap_or(true) {
                continue;
            }
            if let Some(slot) = dependent.get_mut(n) {
                *slot = true;
            }
            if let Some((_, declaration)) = nodes.get(n) {
                queue.extend(declaration.names.iter().map(String::as_str));
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
        // ADR 0096 amendment (2026-10-12, dependents): a dependent keeps only its syntax diagnostics, since
        // the error type does not absorb every cascade (a variant of a broken kind, a generic
        // call whose argument is unknown).
        if diagnostic.code.as_str().starts_with("U0") {
            return true;
        }
        let line = (diagnostic.primary.start.line as usize).saturating_sub(1);
        !ranges.iter().any(|(path, first, last)| {
            *path == diagnostic.primary.file && *first <= line && line < *last
        })
    });
}
