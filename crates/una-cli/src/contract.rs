//! `una contract <qualified.name>` (`spec/13 §5`, ADR 0029): the header of a function, declared
//! or inferred, with its doc comments and requirements.
//!
//! The package is the one in the current directory; the human output is the
//! header in canonical form, with `-- inferred` after each inferred clause and one comment line
//! per requirement; `--json` prints the schema `una.contract/1`.

use serde::Serialize;
use una_check::Header;
use una_syntax::ast::{ClauseKind, Decl, DeclKind, File, FunctionDecl, TextPart};
use una_syntax::{Interner, SourceDb, Span};

/// The JSON schema of `una contract --json` (ADR 0029).
pub(crate) const CONTRACT_SCHEMA: &str = "una.contract/1";

/// The contract of one function, in the key order of ADR 0029.
#[derive(Debug, Serialize)]
pub(crate) struct Contract {
    schema: &'static str,
    function: String,
    public: bool,
    docs: Vec<String>,
    header: String,
    requires: Vec<String>,
    ensures: Vec<String>,
    uses: Vec<Uses>,
    fails: Option<Fails>,
    derived_from: Vec<Requirement>,
    /// The lines of the human output.
    #[serde(skip)]
    lines: Vec<String>,
}

impl Contract {
    /// The lines of its human output, as `una contract` prints them.
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }
}

/// `uses @cap: Capability[ops]`.
#[derive(Debug, Serialize)]
struct Uses {
    cap: String,
    capability: String,
    ops: Vec<String>,
    inferred: bool,
}

/// `fails Kind`.
#[derive(Debug, Serialize)]
struct Fails {
    kind: String,
    inferred: bool,
}

/// A requirement of `derived from`.
#[derive(Debug, Serialize)]
struct Requirement {
    id: String,
    status: &'static str,
    text: String,
    /// The text literal as written, for the human output.
    #[serde(skip)]
    written: String,
}

/// The modules of a checked package: each file's id and tree.
pub(crate) struct Modules<'p> {
    pub(crate) sources: &'p SourceDb,
    pub(crate) interner: &'p Interner,
    pub(crate) trees: &'p [(una_syntax::FileId, File)],
}

impl Modules<'_> {
    /// The source text at `span`.
    pub(crate) fn text(&self, span: Span) -> &str {
        self.sources
            .file(span.file)
            .text()
            .get(span.start..span.end)
            .unwrap_or("")
    }

    /// The text of a name.
    pub(crate) fn name(&self, symbol: una_syntax::Symbol) -> &str {
        self.interner.resolve(symbol)
    }

    /// The module path of `tree`.
    pub(crate) fn path(&self, tree: &File) -> Option<String> {
        let header = tree.module.as_ref()?;
        let segments: Vec<&str> = header
            .path
            .segments
            .iter()
            .map(|segment| self.name(segment.name))
            .collect();
        Some(segments.join("."))
    }

    /// The function `qualified`, a module path followed by the function's name.
    pub(crate) fn function(&self, qualified: &str) -> Option<(String, &Decl, &FunctionDecl)> {
        let (module, name) = qualified.rsplit_once('.')?;
        self.trees.iter().find_map(|(_, tree)| {
            if self.path(tree).as_deref() != Some(module) {
                return None;
            }
            tree.decls.iter().find_map(|decl| match &decl.kind {
                DeclKind::Function(function) if self.name(function.name.name) == name => {
                    Some((module.to_owned(), decl, function))
                }
                _ => None,
            })
        })
    }

    /// The requirement `id`, declared in any module of the package.
    fn requirement(&self, id: &str) -> Option<Requirement> {
        self.trees.iter().find_map(|(_, tree)| {
            tree.decls.iter().find_map(|decl| match &decl.kind {
                DeclKind::Requirement(requirement) if self.name(requirement.id.name) == id => {
                    let text = requirement
                        .text
                        .parts
                        .iter()
                        .map(|part| match part {
                            TextPart::Text(text) => text.clone(),
                            TextPart::Interpolation(expr) => {
                                format!("{{{}}}", self.text(expr.span))
                            }
                        })
                        .collect();
                    Some(Requirement {
                        id: id.to_owned(),
                        status: if requirement.active {
                            "active"
                        } else {
                            "proposed"
                        },
                        text,
                        written: self.text(requirement.text.span).to_owned(),
                    })
                }
                _ => None,
            })
        })
    }
}

/// The contract of `function` of module `module`, whose header after inference is `header`.
/// The package checked without diagnostics, so its sources are in canonical form (U7001).
pub(crate) fn contract(
    modules: &Modules<'_>,
    module: &str,
    decl: &Decl,
    function: &FunctionDecl,
    header: Option<&Header>,
) -> Contract {
    let name = modules.name(function.name.name).to_owned();
    let docs: Vec<String> = decl
        .docs
        .iter()
        .map(|doc| {
            let text = modules.text(doc.span);
            let text = text.strip_prefix("---").unwrap_or(text);
            text.strip_prefix(' ').unwrap_or(text).to_owned()
        })
        .collect();
    let signature = modules
        .text(Span::new(
            decl.span.file,
            decl.span.start,
            function.result.span.end,
        ))
        .to_owned();
    let mut lines: Vec<String> = decl
        .docs
        .iter()
        .map(|doc| modules.text(doc.span).to_owned())
        .collect();
    lines.push(signature.clone());
    let mut requires = Vec::new();
    let mut ensures = Vec::new();
    let mut derived = Vec::new();
    let clause_lines = |pick: &dyn Fn(&ClauseKind) -> bool, lines: &mut Vec<String>| {
        for clause in function.clauses.iter().filter(|c| pick(&c.kind)) {
            lines.push(format!("  {}", modules.text(clause.span)));
        }
    };
    clause_lines(&|k| matches!(k, ClauseKind::Requires(_)), &mut lines);
    clause_lines(&|k| matches!(k, ClauseKind::Ensures(_)), &mut lines);
    for clause in &function.clauses {
        match &clause.kind {
            ClauseKind::Requires(expr) => requires.push(modules.text(expr.span).to_owned()),
            ClauseKind::Ensures(expr) => ensures.push(modules.text(expr.span).to_owned()),
            ClauseKind::DerivedFrom(ids) => {
                derived.extend(ids.iter().map(|id| modules.name(id.name).to_owned()));
            }
            ClauseKind::Uses(_)
            | ClauseKind::Fails(_)
            | ClauseKind::UsesOf(_)
            | ClauseKind::FailsOf(_) => {}
        }
    }
    let uses_inferred = header.is_some_and(|h| h.uses_inferred);
    let uses: Vec<Uses> = header
        .map(|h| {
            h.uses
                .iter()
                .map(|u| Uses {
                    cap: u.cap.clone(),
                    capability: u.capability.clone(),
                    ops: u.ops.clone(),
                    inferred: uses_inferred,
                })
                .collect()
        })
        .unwrap_or_default();
    if uses_inferred {
        for u in &uses {
            lines.push(format!(
                "  uses @{}: {}[{}] -- inferred",
                u.cap,
                u.capability,
                u.ops.join(", ")
            ));
        }
    } else {
        clause_lines(&|k| matches!(k, ClauseKind::Uses(_)), &mut lines);
    }
    // ADR 0062: `uses of p`, as written, or inferred for a private function.
    let written_of = function
        .clauses
        .iter()
        .any(|c| matches!(c.kind, ClauseKind::UsesOf(_)));
    if written_of {
        clause_lines(&|k| matches!(k, ClauseKind::UsesOf(_)), &mut lines);
    } else if let Some(header) = header {
        for param in &header.uses_of {
            lines.push(format!("  uses of {param} -- inferred"));
        }
    }
    let fails_inferred = header.is_some_and(|h| h.fails_inferred);
    let fails = header.and_then(|h| h.fails.clone()).map(|kind| Fails {
        kind,
        inferred: fails_inferred,
    });
    match (&fails, fails_inferred) {
        (Some(fails), true) => lines.push(format!("  fails {} -- inferred", fails.kind)),
        _ => clause_lines(&|k| matches!(k, ClauseKind::Fails(_)), &mut lines),
    }
    let written_fails_of = function
        .clauses
        .iter()
        .any(|c| matches!(c.kind, ClauseKind::FailsOf(_)));
    if written_fails_of {
        clause_lines(&|k| matches!(k, ClauseKind::FailsOf(_)), &mut lines);
    } else if let Some(param) = header.and_then(|h| h.fails_of.clone()) {
        lines.push(format!("  fails of {param} -- inferred"));
    }
    clause_lines(&|k| matches!(k, ClauseKind::DerivedFrom(_)), &mut lines);
    let derived_from: Vec<Requirement> = derived
        .iter()
        .filter_map(|id| modules.requirement(id))
        .collect();
    for requirement in &derived_from {
        lines.push(format!(
            "-- requirement {} ({}): {}",
            requirement.id, requirement.status, requirement.written
        ));
    }
    Contract {
        schema: CONTRACT_SCHEMA,
        function: format!("{module}.{name}"),
        public: function.public,
        docs,
        header: signature,
        requires,
        ensures,
        uses,
        fails,
        derived_from,
        lines,
    }
}

impl Contract {
    /// The human output: one line each, newline-terminated.
    pub(crate) fn human(&self) -> String {
        self.lines.iter().map(|line| format!("{line}\n")).collect()
    }
}
