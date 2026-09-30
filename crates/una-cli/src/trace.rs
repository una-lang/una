//! `una trace <REQUIREMENT_ID>` (`spec/08 §1`): a requirement, the functions derived from it,
//! their contracts and the status of their obligations.
//!
//! The package in the current directory, checked at tier 1, as `una contract`
//! reads it; human lines, or `una.trace/1`.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use una_core::Decl;
use una_diag::{ObligationKind, ObligationStatus};
use una_syntax::ast::{
    ClauseKind, DeclKind, RequirementDecl, RequirementSource, TextLit, TextPart,
};

use crate::contract::Modules;
use crate::verification::{report_unverified, tier_one};
use crate::{ExitStatus, analyze, load, on_large_stack, report};

/// The JSON schema of `una trace --json` (ADR 0034).
const TRACE_SCHEMA: &str = "una.trace/1";

#[derive(Serialize)]
struct Trace<'a> {
    schema: &'static str,
    requirement: RequirementDoc,
    functions: Vec<FunctionDoc<'a>>,
}

#[derive(Serialize)]
struct RequirementDoc {
    id: String,
    status: &'static str,
    text: String,
    source: String,
    owner: String,
}

#[derive(Serialize)]
struct FunctionDoc<'a> {
    function: String,
    requires: Vec<&'a str>,
    ensures: Vec<&'a str>,
    obligations: Vec<ObligationDoc<'a>>,
}

#[derive(Serialize)]
struct ObligationDoc<'a> {
    id: &'a str,
    kind: ObligationKind,
    status: ObligationStatus,
    goal: &'a str,
}

/// The options of `una trace`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TraceOptions {
    pub(crate) json: bool,
    pub(crate) color: bool,
    pub(crate) wallclock: Duration,
}

/// `una trace <REQUIREMENT_ID>`.
pub(crate) fn trace_command(
    id: &str,
    options: TraceOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match load(Path::new(".")) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let analysis = on_large_stack(|| analyze(&package, None));
    // R-027, ADR 0036: the package's diagnostics, those of its project files included, come
    // before the identifier is resolved, so that an unknown one never hides them.
    if !analysis.diagnostics.is_empty() {
        return report(
            &analysis.diagnostics,
            &package.sources,
            options.json,
            options.color,
            stdout,
            stderr,
        );
    }
    let modules = Modules {
        sources: &package.sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
    };
    let Some(requirement) = find(&modules, id) else {
        writeln!(stderr, "error: the package has no requirement `{id}`")?;
        return Ok(ExitStatus::Usage);
    };
    let found = tier_one(&package, &analysis.program, options.wallclock);
    let Some(verification) = found.verification else {
        return report_unverified(
            &found.diagnostics,
            &package,
            options.json,
            options.color,
            stdout,
            stderr,
        );
    };
    let derived = derived(&modules, id);
    let mut functions = Vec::new();
    let mut unproven = false;
    for qualified in &derived {
        let fun = analysis.program.modules.iter().find_map(|module| {
            module.decls.iter().find_map(|decl| match decl {
                Decl::Fun(fun) if format!("{}.{}", module.path, fun.name) == *qualified => {
                    Some(fun)
                }
                _ => None,
            })
        });
        let obligations: Vec<ObligationDoc<'_>> = verification
            .obligations
            .iter()
            .zip(&verification.outcomes)
            .filter(|(obligation, _)| obligation.declaration == *qualified)
            .map(|(obligation, outcome)| {
                unproven |= outcome.status != ObligationStatus::Proven;
                ObligationDoc {
                    id: &obligation.id,
                    kind: obligation.kind,
                    status: outcome.status,
                    goal: &obligation.goal,
                }
            })
            .collect();
        functions.push(FunctionDoc {
            function: qualified.clone(),
            requires: fun.map_or_else(Vec::new, |f| {
                f.requires.iter().map(|c| c.text.as_str()).collect()
            }),
            ensures: fun.map_or_else(Vec::new, |f| {
                f.ensures.iter().map(|c| c.text.as_str()).collect()
            }),
            obligations,
        });
    }
    let document = Trace {
        schema: TRACE_SCHEMA,
        requirement: requirement_doc(&modules, requirement),
        functions,
    };
    if options.json {
        match serde_json::to_string(&document) {
            Ok(text) => writeln!(stdout, "{text}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering the trace as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
    } else {
        write!(stdout, "{}", human(&document))?;
    }
    stdout.flush()?;
    Ok(if unproven {
        ExitStatus::Errors
    } else {
        ExitStatus::Success
    })
}

/// The requirement `id`, declared in any module of the package.
fn find<'m>(modules: &'m Modules<'_>, id: &str) -> Option<&'m RequirementDecl> {
    modules.trees.iter().find_map(|(_, tree)| {
        tree.decls.iter().find_map(|decl| match &decl.kind {
            DeclKind::Requirement(requirement) if modules.name(requirement.id.name) == id => {
                Some(requirement)
            }
            _ => None,
        })
    })
}

/// The functions whose `derived from` names `id`, by qualified name.
fn derived(modules: &Modules<'_>, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (_, tree) in modules.trees {
        let Some(path) = modules.path(tree) else {
            continue;
        };
        for decl in &tree.decls {
            let DeclKind::Function(function) = &decl.kind else {
                continue;
            };
            let derives = function.clauses.iter().any(|clause| match &clause.kind {
                ClauseKind::DerivedFrom(ids) => ids.iter().any(|i| modules.name(i.name) == id),
                _ => false,
            });
            if derives {
                out.push(format!("{path}.{}", modules.name(function.name.name)));
            }
        }
    }
    out.sort();
    out
}

fn requirement_doc(modules: &Modules<'_>, requirement: &RequirementDecl) -> RequirementDoc {
    let text = |lit: &TextLit| -> String {
        lit.parts
            .iter()
            .map(|part| match part {
                TextPart::Text(text) => text.clone(),
                TextPart::Interpolation(expr) => format!("{{{}}}", modules.text(expr.span)),
            })
            .collect()
    };
    let source = match &requirement.source {
        RequirementSource::Human => "human".to_owned(),
        RequirementSource::Policy(document) => {
            format!("policy {}", una_core::quote(&text(document)))
        }
        RequirementSource::Regulation(document) => {
            format!("regulation {}", una_core::quote(&text(document)))
        }
        RequirementSource::Generated(task) => {
            format!("generated {}", una_core::quote(&text(task)))
        }
    };
    RequirementDoc {
        id: modules.name(requirement.id.name).to_owned(),
        status: if requirement.active {
            "active"
        } else {
            "proposed"
        },
        text: text(&requirement.text),
        source,
        owner: modules.name(requirement.owner.name).to_owned(),
    }
}

fn human(trace: &Trace<'_>) -> String {
    let requirement = &trace.requirement;
    let mut out = format!(
        "requirement {} ({})\n  text {}\n  source {}\n  owner {}\n",
        requirement.id,
        requirement.status,
        una_core::quote(&requirement.text),
        requirement.source,
        requirement.owner
    );
    for function in &trace.functions {
        out.push_str(&function.function);
        out.push('\n');
        for clause in &function.requires {
            out.push_str(&format!("  requires {clause}\n"));
        }
        for clause in &function.ensures {
            out.push_str(&format!("  ensures {clause}\n"));
        }
        for obligation in &function.obligations {
            out.push_str(&format!(
                "  {:<8} {} {:<13} {}\n",
                obligation.status.as_str(),
                obligation.id,
                obligation.kind.as_str(),
                obligation.goal
            ));
        }
    }
    out
}
