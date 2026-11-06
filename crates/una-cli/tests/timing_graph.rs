//! The completeness test of the graph of ADR 0116 P1 (amendment of 2026-10-25, section I).
//!
//! `τ_conc` is sound only if every way that code can run is an edge of the graph: the
//! soundness argument (D, step 2) follows the chain of calls from a part of a construct to a
//! site, one edge of P1 per link. So for every package of the conformance suite, each call node
//! of its Core IR must have its edge, recorded by the type checker at the same call site, from
//! the node whose code holds it:
//! - a call of a function or predicate, and a function named as a value, to that function;
//! - a policy call, to the policy and to its `via` function, which the call runs (#1673);
//! - a call of a function value, a `uses of` parameter's included, to `Λ`;
//! - an operation, to the operation;
//! - an installation by `with` or `given`, to the handler;
//!
//! with a call inside a lambda's body leaving from `Λ`, and one inside a handler's operation
//! from the handler. A prelude function with a function argument has its edge to `Λ` by its
//! declaration, so it needs none per call.
//!
//! The innermost task construct the checker recorded for each edge (P3) must also be the one
//! that an independent walk of Core IR finds: the parts are the branches of `together` and of
//! `first of`, the body of `together for` and the block of `tasks`, the `start`s in it included;
//! a lambda's body starts with none. Every timing site and every construct of Core IR must be
//! recorded too, in that construct, a construct with the parent Core IR gives it; names are
//! compared qualified; and every case must load.
//!
//! Two call nodes of Core IR are written by elaboration, not by the source, and are met
//! otherwise:
//! - `edit m.update(key:, initial:, change:)` on a map or a hash map is a call of the hidden
//!   function `update` of `std.map` or `std.collections`, whose edge the checker records at the
//!   call; elaboration inlines it, with an application of `change` at the argument's span.
//!   That edge reaches `Λ`, since `update` has a parameter of function type (P1).
//! - An operator on dense values (`+`, `-`, a scalar `*` or `/`, ADR 0120 rule 34) is a call of
//!   the prelude's `dense_*_map` or `dense_*_zip_map` with the element type's operator as the
//!   lambda, which Core IR writes and which calls nothing (the walk checks it, as `Λ`'s code).
//!   Those functions run no Una code of the package and hold no site or construct, so the call
//!   can raise no `conc`; recording it would only make `τ⁰` of its caller `⊤` through `Λ`.
//!
//! The test lives here rather than in `una-check`, which the amendment names, since loading a
//! package of the suite (its manifest, its modules, its dependencies) is this crate's job.
//! Examples are not nodes of the graph (P4), so their code is not walked.

// Tests may unwrap, index and panic freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};

use una_check::{TimingGraph, TimingNode, TimingTarget};
use una_core::{Bound, Decl, Expr, LoopSource, Rhs, Source};
use una_syntax::Span;

/// What a call node of Core IR needs in the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Need {
    /// A function or predicate, by its qualified name.
    Function(String),
    /// `Λ`.
    Lambda,
    /// An operation, by its capability and its name.
    Operation(String, String),
    /// A handler, by its qualified name.
    Install(String),
}

/// A call node of Core IR: where it is, the node whose code holds it, what it needs, and the
/// span of the innermost construct whose part holds it.
#[derive(Debug)]
struct CallNode {
    span: Span,
    from: TimingNode,
    need: Need,
    construct: Option<Span>,
}

/// `module.name`, or `name` in a module without a path, as the graph names declarations.
fn qualify(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.to_owned()
    } else {
        format!("{module}.{name}")
    }
}

/// What the walk of Core IR finds: the call nodes, the timing sites and the constructs, each
/// site and construct with the construct whose part holds it.
#[derive(Default)]
struct Found {
    calls: Vec<CallNode>,
    sites: Vec<(&'static str, Span, Option<Span>)>,
    constructs: Vec<(&'static str, Span, Option<Span>)>,
}

/// The walk of one node's code: its node and the constructs whose parts enclose the expression
/// walked, innermost last.
struct Walk<'a> {
    from: TimingNode,
    parts: Vec<Span>,
    found: &'a mut Found,
}

impl Walk<'_> {
    fn need(&mut self, span: Span, need: Need) {
        self.found.calls.push(CallNode {
            span,
            from: self.from.clone(),
            need,
            construct: self.parts.last().copied(),
        });
    }

    fn site(&mut self, kind: &'static str, span: Span) {
        self.found
            .sites
            .push((kind, span, self.parts.last().copied()));
    }

    fn construct(&mut self, kind: &'static str, span: Span) {
        self.found
            .constructs
            .push((kind, span, self.parts.last().copied()));
    }

    fn part(&mut self, construct: Span, expr: &Expr) {
        self.parts.push(construct);
        self.expr(expr);
        self.parts.pop();
    }

    /// The right-hand side of a `let` at `at`.
    fn rhs(&mut self, rhs: &Rhs, at: Span) {
        match rhs {
            Rhs::Call { function, span, .. } => self.need(
                *span,
                Need::Function(qualify(&function.module, &function.name)),
            ),
            Rhs::Apply { span, .. } => self.need(*span, Need::Lambda),
            Rhs::Op {
                capability,
                op,
                span,
                ..
            } => self.need(*span, Need::Operation(capability.clone(), op.clone())),
            // A policy call runs the policy's `via` function (`spec/07 §5`, #1673).
            Rhs::Declassify {
                policy, via, span, ..
            } => {
                self.need(*span, Need::Function(qualify(&policy.module, &policy.name)));
                self.need(*span, Need::Function(qualify(&via.module, &via.name)));
            }
            // A function named as a value: an edge from here to it, at the name, which the
            // `let` of the value carries; its Core body is a call of it that the source does not
            // hold.
            Rhs::Lambda {
                function: Some(function),
                ..
            } => self.need(
                at,
                Need::Function(qualify(&function.module, &function.name)),
            ),
            Rhs::Lambda {
                body,
                function: None,
                ..
            } => {
                let mut inner = Walk {
                    from: TimingNode::Lambda,
                    parts: Vec::new(),
                    found: &mut *self.found,
                };
                inner.expr(body);
            }
            Rhs::Checked { predicate, .. } => self.expr(predicate),
            _ => {}
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Atom { .. }
            | Expr::Exit { .. }
            | Expr::Fail { .. }
            | Expr::Unreachable { .. } => {}
            Expr::Let {
                value, body, span, ..
            } => {
                match value.as_ref() {
                    Bound::Rhs(rhs) => self.rhs(rhs, *span),
                    Bound::Expr(inner) => self.expr(inner),
                }
                self.expr(body);
            }
            Expr::LetTuple { body, .. } => self.expr(body),
            Expr::Loop {
                source,
                body,
                on_stop,
                span,
                ..
            } => {
                match source.as_ref() {
                    LoopSource::Until { cond, .. } => self.expr(cond),
                    LoopSource::Ever { within: Some(_) } => self.site("for ever within", *span),
                    _ => {}
                }
                self.expr(body);
                if let Some(on_stop) = on_stop {
                    self.expr(&on_stop.body);
                }
            }
            Expr::Together { tasks, span } => {
                self.construct("together", *span);
                for task in tasks {
                    self.part(*span, task);
                }
            }
            Expr::FirstOf { tasks, span } => {
                self.site("first of", *span);
                self.construct("first of", *span);
                for task in tasks {
                    self.part(*span, task);
                }
            }
            Expr::TogetherFor {
                source, task, span, ..
            } => {
                if let LoopSource::Until { cond, .. } = source.as_ref() {
                    self.expr(cond);
                }
                self.construct("together for", *span);
                self.part(*span, task);
            }
            Expr::Tasks { body, span, .. } => {
                self.site("tasks", *span);
                self.construct("tasks", *span);
                self.part(*span, body);
            }
            Expr::Start { task, .. } => self.expr(task),
            Expr::Quiet { body, .. } | Expr::Attempt { body, .. } => self.expr(body),
            Expr::Within { body, span, .. } => {
                self.site("within", *span);
                self.expr(body);
            }
            Expr::If {
                then, otherwise, ..
            } => {
                self.expr(then);
                self.expr(otherwise);
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                for arm in arms {
                    self.expr(&arm.body);
                }
                if let Some(otherwise) = otherwise {
                    self.expr(otherwise);
                }
            }
            Expr::Handle { body, handler, .. } => {
                self.expr(body);
                self.expr(handler);
            }
            Expr::With { source, body, .. } => {
                if let Source::Handler { handler, span, .. } = source {
                    self.need(
                        *span,
                        Need::Install(qualify(&handler.module, &handler.name)),
                    );
                }
                self.expr(body);
            }
        }
    }
}

/// Whether `to`, in the graph, is what `need` asks for.
fn meets(to: &TimingTarget, need: &Need) -> bool {
    match (to, need) {
        (TimingTarget::Function(a), Need::Function(b))
        | (TimingTarget::Install(a), Need::Install(b)) => a == b,
        (TimingTarget::Lambda, Need::Lambda) => true,
        (TimingTarget::Operation(c, o), Need::Operation(d, p)) => c == d && o == p,
        _ => false,
    }
}

/// The hidden `update` functions that elaboration inlines, of the map module of the prelude and
/// of `std.collections`.
const INLINED_UPDATES: [&str; 2] = ["prelude.map.update", "std.collections.update"];

/// The problems of one package: each call node without its edge, or whose edge records another
/// innermost construct; each site or construct of Core IR not recorded, or recorded in another
/// construct; and the number of sites placed.
fn problems(case: &str, program: &una_core::Program, graph: &TimingGraph) -> (Vec<String>, usize) {
    let mut found = Found::default();
    for module in &program.modules {
        for decl in &module.decls {
            let (from, bodies): (TimingNode, Vec<&Expr>) = match decl {
                Decl::Fun(fun) => (
                    TimingNode::Function(qualify(&module.path, &fun.name)),
                    vec![&fun.body],
                ),
                Decl::Pred { name, body, .. } => (
                    TimingNode::Function(qualify(&module.path, name)),
                    vec![body],
                ),
                Decl::Handler { name, ops, .. } => (
                    TimingNode::Handler(qualify(&module.path, name)),
                    ops.iter().map(|op| &op.body).collect(),
                ),
                _ => continue,
            };
            for body in bodies {
                let mut walk = Walk {
                    from: from.clone(),
                    parts: Vec::new(),
                    found: &mut found,
                };
                walk.expr(body);
            }
        }
    }
    let construct_span = |id: Option<usize>| id.map(|id| graph.constructs[id].span);
    let inside = |inner: Span, outer: Span| {
        inner.file == outer.file && outer.start <= inner.start && inner.end <= outer.end
    };
    let mut problems = Vec::new();
    for call in &found.calls {
        let mut edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.span == call.span && meets(&edge.to, &call.need))
            .collect();
        // The `change` of an inlined `update`: its call's edge.
        if edges.is_empty() && call.need == Need::Lambda {
            edges = graph
                .edges
                .iter()
                .filter(|edge| {
                    inside(call.span, edge.span)
                        && matches!(&edge.to, TimingTarget::Function(name) if INLINED_UPDATES.contains(&name.as_str()))
                })
                .collect();
        }
        // An operator on dense values: a call of the prelude that no source call holds.
        if edges.is_empty()
            && let Need::Function(name) = &call.need
            && name.starts_with("prelude.dense_")
            && name.ends_with("map")
            && !graph.edges.iter().any(|edge| edge.span == call.span)
        {
            continue;
        }
        if !edges.iter().any(|edge| edge.from == call.from) {
            problems.push(format!(
                "{case}: no edge for {:?} at {:?} from {:?} (edges there: {:?})",
                call.need, call.span, call.from, edges
            ));
        } else if !edges
            .iter()
            .any(|edge| edge.from == call.from && construct_span(edge.construct) == call.construct)
        {
            problems.push(format!(
                "{case}: the edge for {:?} at {:?} lies in {:?}, Core IR places it in {:?}",
                call.need,
                call.span,
                edges
                    .iter()
                    .map(|e| construct_span(e.construct))
                    .collect::<Vec<_>>(),
                call.construct
            ));
        }
    }
    let mut placed = 0;
    for (kind, span, construct) in &found.sites {
        let recorded: Vec<Option<Span>> = graph
            .sites
            .iter()
            .filter(|site| site.span == *span && site.kind == *kind)
            .map(|site| construct_span(site.construct))
            .collect();
        if recorded.is_empty() {
            problems.push(format!(
                "{case}: the {kind} site at {span:?} is not recorded"
            ));
        } else if !recorded.contains(construct) {
            problems.push(format!(
                "{case}: the {kind} site at {span:?} lies in {recorded:?}, Core IR places it in {construct:?}"
            ));
        } else {
            placed += 1;
        }
    }
    for (kind, span, parent) in &found.constructs {
        let recorded: Vec<Option<Span>> = graph
            .constructs
            .iter()
            .filter(|c| c.span == *span && c.kind == *kind)
            .map(|c| construct_span(c.construct))
            .collect();
        if recorded.is_empty() {
            problems.push(format!("{case}: the {kind} at {span:?} is not recorded"));
        } else if !recorded.contains(parent) {
            problems.push(format!(
                "{case}: the {kind} at {span:?} has the parent {recorded:?}, Core IR gives it {parent:?}"
            ));
        }
    }
    (problems, placed)
}

/// Every case of the conformance suite: each `.una` file and each package directory of an
/// area.
fn cases() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance");
    let mut cases = Vec::new();
    for area in std::fs::read_dir(&root).unwrap() {
        let area = area.unwrap().path();
        if !area.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&area).unwrap() {
            let path = entry.unwrap().path();
            let package = path.join("una.toml").exists() || path.join("ita.una").exists();
            if package || path.extension().is_some_and(|e| e == "una") {
                cases.push(path);
            }
        }
    }
    cases.sort();
    cases
}

#[test]
fn every_call_of_the_conformance_suite_has_its_edge_and_its_construct() {
    let cases = cases();
    assert!(cases.len() > 1_000, "only {} cases found", cases.len());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().min(4));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new((0usize, 0usize, 0usize, Vec::new(), Vec::new()));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(path) = cases.get(at) else {
                        break;
                    };
                    let name = path.display().to_string();
                    let checked = match una_cli::check(path, una_cli::Tier::Zero) {
                        Ok(checked) => checked,
                        Err(error) => {
                            let mut results = results.lock().unwrap();
                            results.4.push(format!("{name}: {error:?}"));
                            continue;
                        }
                    };
                    let (found, placed) = problems(&name, &checked.program, &checked.timing_graph);
                    let mut results = results.lock().unwrap();
                    results.0 += 1;
                    results.1 += checked.timing_graph.edges.len();
                    results.2 += placed;
                    results.3.extend(found);
                }
            });
        }
    });
    let (checked, edges, placed, found, unloaded) = results.into_inner().unwrap();
    // Every case loads, or the walk would skip it unseen.
    assert!(
        unloaded.is_empty(),
        "{} cases did not load:\n{}",
        unloaded.len(),
        unloaded.join("\n")
    );
    assert!(checked > 1_000, "only {checked} cases checked");
    assert!(edges > 1_000, "only {edges} edges recorded");
    // The sites of Core IR are found in the graph by their spans.
    assert!(placed > 100, "only {placed} sites placed");
    assert!(
        found.is_empty(),
        "{} problems:\n{}",
        found.len(),
        found.join("\n")
    );
}

/// The construct tree of one body (P3): nested constructs, the list of `together for` and the
/// bound of `tasks` outside their parts, the `start`s in the part of their `tasks`, and a
/// lambda's body outside every construct of the function that holds it.
#[test]
fn the_construct_tree_follows_the_parts() {
    let dir = std::env::temp_dir().join(format!("una-timing-graph-{}", std::process::id()));
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita t\n  version 0.1.0\n  edition 0.4\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("ita.lock"),
        "-- Written by `una ita`; do not edit.\nlock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n",
    )
    .unwrap();
    std::fs::write(
        src.join("app.una"),
        "module app\n\nwork(n: Int) -> Unit = unit\n\ncount(n: Int) -> Int = n + 1\n\nitems(n: Int) -> List[Int] = [n]\n\napply(f: fn(Int) -> Int, n: Int) -> Int = f(n)\n\npub run(n: Int) -> Int\n=\n  tasks at most count(n) at once\n    start work(n)\n    let ys = together for x in items(n)\n      count(x)\n    unit\n  apply(fn(x: Int) -> count(x), n)\n",
    )
    .unwrap();
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let graph = &checked.timing_graph;
    let kinds: Vec<(&str, Option<usize>)> = graph
        .constructs
        .iter()
        .map(|c| (c.kind, c.construct))
        .collect();
    assert_eq!(kinds, [("tasks", None), ("together for", Some(0))]);
    let edge = |name: &str| -> Vec<(TimingNode, Option<usize>)> {
        graph
            .edges
            .iter()
            .filter(|e| e.to == TimingTarget::Function(format!("app.{name}")))
            .map(|e| (e.from.clone(), e.construct))
            .collect()
    };
    let run = || TimingNode::Function("app.run".to_owned());
    // The bound and the list are outside the parts; the `start` and the body are in them.
    assert_eq!(
        edge("count"),
        [(run(), None), (run(), Some(1)), (TimingNode::Lambda, None)]
    );
    assert_eq!(edge("items"), [(run(), Some(0))]);
    assert_eq!(edge("work"), [(run(), Some(0))]);
    assert_eq!(edge("apply"), [(run(), None)]);
    // The call of the function value `f` in `apply`, to `Λ`.
    assert!(
        graph.edges.iter().any(|e| e.to == TimingTarget::Lambda
            && e.from == TimingNode::Function("app.apply".to_owned()))
    );
}
