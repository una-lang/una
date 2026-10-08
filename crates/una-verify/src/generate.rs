//! Obligation generation (`spec/12 §3`): a walk over Core IR that keeps the path context Π,
//! and builds the query of each obligation (`spec/12 §4`).

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use una_core::{
    Atom, Base, BinOp, Bound, CapOp, Clause, Ctor, DataVariant, Decl, Expr, FixedOp, Fun, IntType,
    LoopSource, MapOp, Name, Path, Prim, Program, Refinement, Rhs, Source, Term, Type,
    print_term_with,
};
use una_diag::ObligationKind;
use una_syntax::{SourceDb, Span};

use crate::smt::{
    Const, ConstRole, Definition, F, Fun as SmtFun, FunRole, HASHED, LIST, MAP, Query, Sort, TEXT,
    UNIT, kind_sort, path_selector,
};

/// A proof obligation of `spec/12 §3`, with its query.
#[derive(Clone, Debug)]
pub struct Obligation {
    /// The stable id, `ob-` and 12 hex digits (`spec/12 §8`).
    pub id: String,
    /// The kind.
    pub kind: ObligationKind,
    /// The module path.
    pub module: String,
    /// The enclosing declaration, qualified.
    pub declaration: String,
    /// The goal, as text.
    pub goal: String,
    /// The site of the obligation, or the clause of a return obligation.
    pub source: Span,
    /// The declaring clause, and what it is.
    pub related: Option<(Span, &'static str)>,
    /// For a return obligation, the returned expression of each path, in the order of the
    /// query's path selectors.
    pub paths: Vec<Span>,
    /// The hypotheses of the path context, as text.
    pub hypotheses: Vec<String>,
    /// The query: `Π ∧ ¬goal`.
    pub(crate) query: Query,
    /// The terms whose values a counterexample shows.
    pub(crate) shown: Vec<Shown>,
    /// Where the obligation arises, which decides its repairs (`spec/13 §3`).
    pub site: Site,
    /// The source names of the values that a loop made fresh and that the counterexample
    /// shows (ADR 0071 D3): a refutation is then unknown.
    pub(crate) forgotten: Vec<String>,
    /// The equalities that put the enclosing loops at their first iteration, for the second
    /// query of a refutation that sets loop-fresh values (ADR 0071 D3).
    pub(crate) entry: Vec<F>,
    /// The divisions by a value that is not a literal that each path evaluates before the
    /// obligation, in the order of `paths` (one entry for an obligation at one site): the
    /// divisor and the division. A model that sets one of them to zero is no run (ADR 0054,
    /// amendment of 2026-10-08).
    pub(crate) divisions: Vec<Vec<(F, Span)>>,
}

/// Where an obligation arises: what decides which repair candidates of `spec/13 §3` apply
/// ("when applicable", R-021).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    /// The declaration that encloses the obligation.
    pub within: Within,
    /// The parameter of the enclosing function that the value is, when the goal's only free
    /// binder is that parameter.
    pub parameter: Option<String>,
    /// Whether the goal has free binders, all of them parameters of the enclosing function,
    /// so that a `requires` clause can state it.
    pub over_parameters: bool,
    /// Whether the goal has no free binder: no type, `requires` or guard can make it hold.
    pub ground: bool,
    /// For an obligation of a function value flowing into a function type, or of a lambda's
    /// result, its only repair (ADR 0037 rule 6).
    pub flow: Option<String>,
    /// For a precondition of a named function passed as a function value, the function's
    /// name: the function type it flows into has no precondition (issue #76).
    pub function_value: Option<String>,
    /// For an `overflow` obligation, a guard that is valid code and states its goal without
    /// overflowing itself: the operand's range for a conversion, or the operation on `Int`
    /// (`spec/18 §3`).
    pub guard: Option<String>,
}

/// The kind of declaration that encloses an obligation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Within {
    /// A function, public or not.
    Function {
        /// Whether it is `pub`.
        public: bool,
    },
    /// An example.
    Example,
    /// An operation of a handler, or the body of an opaque predicate.
    Other,
}

/// A term that a counterexample shows, by source name.
#[derive(Clone, Debug)]
pub(crate) struct Shown {
    pub(crate) name: String,
    pub(crate) term: F,
    pub(crate) show: Show,
}

/// How a value shows.
#[derive(Clone, Debug)]
pub(crate) enum Show {
    /// An `Int` or a `Bool`, as the solver writes it.
    Value,
    /// A value of a sum, by the variant its tag names.
    Variant { tag: F, names: Vec<String> },
    /// Another value, as `<Kind>#<n>`.
    Opaque(String),
}

/// The obligations of `program`, in the order of `spec/12 §5`: file, start offset, kind.
pub fn obligations(program: &Program, sources: &SourceDb) -> Vec<Obligation> {
    let index = Index::new(program);
    let mut all = Vec::new();
    for module in &program.modules {
        for decl in &module.decls {
            let mut walker = Walker::new(&index, sources, &module.path);
            match decl {
                Decl::Fun(fun) => walker.function(fun),
                Decl::Handler {
                    name,
                    params,
                    capability_path,
                    ops,
                    ..
                } => {
                    for op in ops {
                        walker.declaration = format!("{}.{name}.{}", module.path, op.name);
                        walker.lambdas.clear();
                        walker.handler_op(params, capability_path, op);
                    }
                }
                Decl::Pred {
                    name,
                    reflect: false,
                    params,
                    body,
                    ..
                } => {
                    walker.declaration = format!("{}.{name}", module.path);
                    walker.lambdas.clear();
                    walker.opaque(params, body);
                }
                _ => {}
            }
            all.extend(walker.obligations);
        }
        for example in &module.examples {
            let mut walker = Walker::new(&index, sources, &module.path);
            walker.declaration =
                format!("{}.example {}", module.path, una_core::quote(&example.name));
            walker.within = Within::Example;
            let mut leaves = Vec::new();
            walker.walk(&example.body, &Ctx::default(), &mut leaves);
            // `spec/08 §2` rule 4: `expect` lines are verified too, with `result` the value of
            // one of the `do` line's paths.
            for expectation in &example.expectations {
                if let una_core::Expectation::Holds { result, check, .. } = expectation {
                    let mut ctx = Ctx::default();
                    if let Some(ty) = leaves.iter().find_map(|l| l.value.as_ref().map(|v| &v.ty)) {
                        std::rc::Rc::make_mut(&mut ctx.types).insert(result.clone(), ty.clone());
                        let x = own(&ctx, result);
                        let paths: Vec<F> =
                            leaves.iter().map(|leaf| walker.path(leaf, 0, &x)).collect();
                        ctx.define(F::or(paths));
                    }
                    let mut discarded = Vec::new();
                    walker.walk(check, &ctx, &mut discarded);
                }
            }
            all.extend(walker.obligations);
        }
    }
    all.sort_by_key(|o| (o.source.file, o.source.start, o.kind));
    all
}

/// For each `ensures` clause of a public function, a query without any hypothesis, its
/// parameters and `result` unconstrained: a clause the solver proves this way is trivial, a
/// missing contract (`spec/13 §6`).
pub fn trivial_ensures(program: &Program, sources: &SourceDb) -> Vec<Obligation> {
    let index = Index::new(program);
    let mut all = Vec::new();
    for module in &program.modules {
        for decl in &module.decls {
            let Decl::Fun(fun) = decl else {
                continue;
            };
            if !fun.public || fun.ensures.is_empty() {
                continue;
            }
            let mut walker = Walker::new(&index, sources, &module.path);
            walker.declaration = format!("{}.{}", module.path, fun.name);
            walker.lambdas.clear();
            let mut ctx = Ctx::default();
            for (name, ty) in &fun.params {
                std::rc::Rc::make_mut(&mut ctx.types).insert(name.clone(), ty.clone());
            }
            std::rc::Rc::make_mut(&mut ctx.types).insert("result".to_owned(), fun.result.clone());
            let result = own(&ctx, "result");
            for clause in &fun.ensures {
                let goal = {
                    let vars = |name: &str| Some(own(&ctx, name));
                    let env = Env {
                        vars: &vars,
                        it: None,
                        result: Some(result.clone()),
                    };
                    walker.term(&clause.term, &env)
                };
                walker.site(
                    ObligationKind::Postcondition,
                    &ctx,
                    goal.map(|g| g.f),
                    clause.text.clone(),
                    clause.span.unwrap_or(fun.span),
                    None,
                );
            }
            all.extend(walker.obligations);
        }
    }
    all
}

// ----- declarations -----

/// The contract of a callee: a function of the package or of the standard library.
struct Contract<'p> {
    /// The type parameters of a generic callee.
    type_params: &'p [String],
    params: &'p [(Name, Type)],
    result: &'p Type,
    requires: &'p [Clause],
    ensures: &'p [Clause],
    /// The error kind of a function that can fail.
    fails: Option<&'p str>,
    /// The result at the callee's type arguments, for the text of a repair; the result
    /// itself when the callee is not generic.
    instantiated_result: Option<&'p Type>,
}

/// A predicate: its parameters, and the definition of a reflect one.
type Predicate<'p> = (&'p [(Name, Type)], Option<&'p Term>);

/// A kind's variants, with its type parameters.
struct Kind<'p> {
    params: &'p [String],
    variants: &'p [DataVariant],
    record: bool,
}

/// The declarations of a program that obligations refer to.
pub(crate) struct Index<'p> {
    contracts: BTreeMap<(&'p str, &'p str), Contract<'p>>,
    predicates: BTreeMap<(&'p str, &'p str), Predicate<'p>>,
    /// Capabilities by qualified identity (`spec/10 §11` rule 8), the host ones included.
    capabilities: BTreeMap<(&'p str, &'p str), &'p [CapOp]>,
    handlers: BTreeMap<(&'p str, &'p str), &'p [(Name, Type)]>,
    kinds: BTreeMap<(&'p str, &'p str), Kind<'p>>,
    policies: BTreeMap<(&'p str, &'p str), (&'p Type, &'p Type)>,
    /// The `to` type of each policy, by its `via` function.
    vias: BTreeMap<(&'p str, &'p str), Vec<&'p Type>>,
}

/// Whether `module` is the prelude's (`spec/14 §1`). The prelude is identified by its module
/// path, which no module of a package that reaches the verifier can have: a user module path
/// whose first segment is `prelude` is U1009 at tier 0 (ADR 0038 D7), and tier 1 runs only
/// when tier 0 reports nothing. D-135: the reserved path is the
/// prelude's identity, rather than a new Core IR path that no source can spell, and a kind the
/// package declares is never taken for a prelude kind ([`Index::prelude_kind`]).
fn is_prelude(module: &str) -> bool {
    module == una_core::PRELUDE_MODULE
}

/// The variants of the prelude's kinds, which the program does not declare: names and field
/// counts (`spec/14 §1, §3`).
fn prelude_variants(kind: &str) -> Option<&'static [(&'static str, &'static [&'static str])]> {
    match kind {
        "Option" => Some(&[("Some", &["value"]), ("None", &[])]),
        "Result" => Some(&[("Ok", &["value"]), ("Err", &["failure"])]),
        "FileError" => Some(&[("NotFound", &[]), ("Denied", &[]), ("Io", &[])]),
        "KeyError" => Some(&[("UnknownKey", &[])]),
        _ => None,
    }
}

impl<'p> Index<'p> {
    fn new(program: &'p Program) -> Index<'p> {
        let mut index = Index {
            contracts: BTreeMap::new(),
            predicates: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            handlers: BTreeMap::new(),
            kinds: BTreeMap::new(),
            policies: BTreeMap::new(),
            vias: BTreeMap::new(),
        };
        for signature in &program.library {
            index.contracts.insert(
                (signature.path.module.as_str(), signature.path.name.as_str()),
                Contract {
                    type_params: &signature.type_params,
                    params: &signature.params,
                    result: &signature.result,
                    requires: &signature.requires,
                    ensures: &signature.ensures,
                    fails: None,
                    instantiated_result: None,
                },
            );
        }
        // `spec/14 §3`: the host capabilities, declared in `std.host`.
        for decl in &program.host {
            if let Decl::Cap { name, ops } = decl {
                index
                    .capabilities
                    .insert((una_core::HOST_MODULE, name.as_str()), ops);
            }
        }
        for module in &program.modules {
            let path = module.path.as_str();
            for decl in &module.decls {
                match decl {
                    Decl::Fun(fun) => {
                        index.contracts.insert(
                            (path, fun.name.as_str()),
                            Contract {
                                type_params: &fun.type_params,
                                params: &fun.params,
                                result: &fun.result,
                                requires: &fun.requires,
                                ensures: &fun.ensures,
                                fails: fun.fails.as_deref(),
                                instantiated_result: None,
                            },
                        );
                    }
                    Decl::Pred {
                        name,
                        params,
                        formula,
                        ..
                    } => {
                        index
                            .predicates
                            .insert((path, name.as_str()), (params, formula.as_ref()));
                    }
                    Decl::Cap { name, ops } => {
                        index.capabilities.insert((path, name.as_str()), ops);
                    }
                    Decl::Handler { name, params, .. } => {
                        index.handlers.insert((path, name.as_str()), params);
                    }
                    Decl::Data {
                        name,
                        params,
                        variants,
                        record,
                    } => {
                        index.kinds.insert(
                            (path, name.as_str()),
                            Kind {
                                params,
                                variants,
                                record: *record,
                            },
                        );
                    }
                    Decl::Policy {
                        name,
                        from,
                        to,
                        via,
                    } => {
                        index.policies.insert((path, name.as_str()), (from, to));
                        index
                            .vias
                            .entry((via.module.as_str(), via.name.as_str()))
                            .or_default()
                            .push(to);
                    }
                    Decl::Alias { .. } => {}
                }
            }
        }
        index
    }

    /// The operation `op` of the capability declared as `capability`, looked up by resolved
    /// identity, never by bare name (ADR 0038 D6, `spec/10 §11` rule 8).
    fn operation(&self, capability: &una_core::Path, op: &str) -> Option<&'p CapOp> {
        self.capabilities
            .get(&(capability.module.as_str(), capability.name.as_str()))
            .and_then(|ops| ops.iter().find(|o| o.name == op))
    }

    /// The refinements of the `to` types of the policies whose `via` function is `name` of
    /// `module`, except those of `result`, the via function's own result type, which its own
    /// result obligations prove already.
    fn via_refinements(&self, module: &str, name: &str, result: &Type) -> Vec<&'p Refinement> {
        let own = refinements(result);
        let mut out: Vec<&'p Refinement> = Vec::new();
        for to in self.vias.get(&(module, name)).into_iter().flatten() {
            for refinement in refinements(to) {
                if !own.contains(refinement) && !out.contains(&refinement) {
                    out.push(refinement);
                }
            }
        }
        out
    }

    /// Whether the kind at (`module`, `name`) is a kind of the prelude. A kind the package
    /// declares never is, whatever its module is called.
    fn prelude_kind(&self, module: &str, name: &str) -> bool {
        is_prelude(module) && !self.kinds.contains_key(&(module, name))
    }

    /// The number of variants of the kind at (`module`, `name`), and their names.
    fn variant_names(&self, module: &str, name: &str) -> Vec<String> {
        if self.prelude_kind(module, name)
            && let Some(variants) = prelude_variants(name)
        {
            return variants.iter().map(|(v, _)| (*v).to_owned()).collect();
        }
        self.kinds
            .get(&(module, name))
            .map(|kind| kind.variants.iter().map(|v| v.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Whether the kind is a sum whose variants have no fields, so that equality and order
    /// are those of its tags (`spec/03 §1`).
    fn payload_free(&self, module: &str, name: &str) -> bool {
        if self.prelude_kind(module, name) {
            return prelude_variants(name)
                .is_some_and(|variants| variants.iter().all(|(_, fields)| fields.is_empty()));
        }
        self.kinds
            .get(&(module, name))
            .is_some_and(|kind| !kind.record && kind.variants.iter().all(|v| v.fields.is_empty()))
    }

    /// The declared type of field `index` of `variant` of the kind of `ty`, with the kind's
    /// type parameters replaced by its arguments.
    fn field_type(&self, ty: &Type, variant: usize, index: usize) -> Option<Type> {
        let Type::Base {
            base: Base::Named { module, name, args },
            ..
        } = ty
        else {
            return None;
        };
        if self.prelude_kind(module, name) {
            // `Some(value: T)`, `Ok(value: T)`, `Err(failure: E)`.
            return match (name.as_str(), variant) {
                ("Option" | "Result", 0) | ("Result", 1) if index == 0 => {
                    args.get(variant).cloned()
                }
                _ => None,
            };
        }
        let kind = self.kinds.get(&(module.as_str(), name.as_str()))?;
        let (_, field) = kind.variants.get(variant)?.fields.get(index)?;
        Some(substitute(field, kind.params, args))
    }
}

/// `ty` with the type parameters `params` replaced by `args`.
fn substitute(ty: &Type, params: &[String], args: &[Type]) -> Type {
    match ty {
        Type::Base {
            base: Base::Param(name),
            refinements,
            label,
        } => match params
            .iter()
            .position(|p| p == name)
            .and_then(|i| args.get(i))
        {
            Some(Type::Base {
                base,
                refinements: own,
                ..
            }) => Type::Base {
                base: base.clone(),
                refinements: own.iter().chain(refinements).cloned().collect(),
                label: label.clone(),
            },
            Some(other) => other.clone(),
            None => ty.clone(),
        },
        Type::Base {
            base:
                Base::Named {
                    module,
                    name,
                    args: own,
                },
            refinements,
            label,
        } => Type::Base {
            base: Base::Named {
                module: module.clone(),
                name: name.clone(),
                args: own.iter().map(|a| substitute(a, params, args)).collect(),
            },
            refinements: refinements.clone(),
            label: label.clone(),
        },
        Type::Fn {
            params: own,
            result,
            uses,
            fails,
            label,
        } => Type::Fn {
            params: own.iter().map(|p| substitute(p, params, args)).collect(),
            result: Box::new(substitute(result, params, args)),
            uses: uses.clone(),
            fails: fails.clone(),
            label: label.clone(),
        },
        other => other.clone(),
    }
}

/// The refinements of a type (`spec/03 §2` rule 3).
fn refinements(ty: &Type) -> &[Refinement] {
    match ty {
        Type::Base { refinements, .. } => refinements,
        Type::Fn { .. } => &[],
    }
}

fn base_type(base: Base) -> Type {
    Type::Base {
        base,
        refinements: Vec::new(),
        label: una_core::Label::public(),
    }
}

/// The sort of a Una type (`spec/12 §4`), in the namespaces of ADR 0038 D8:
/// `Int` and `Bool` are SMT-LIB's, the other built-in types and the kinds of the prelude are
/// the encoder's (`i!Text`, `i!List_<T>`, `i!Option_<T>`, `i!FileError`), and the kinds and
/// type parameters of the program are the user's (`u!Kind_<module>_<Name>`, `u!Param_<name>`).
pub(crate) fn sort(ty: &Type) -> Sort {
    match ty {
        Type::Fn { .. } => "i!Fn".to_owned(),
        Type::Base { base, .. } => match base {
            // A fixed-width integer is an `Int` with its range bounds (`spec/18 §6`).
            Base::Int | Base::Fixed(_) => "Int".to_owned(),
            Base::Bool => "Bool".to_owned(),
            Base::Text => TEXT.to_owned(),
            Base::Unit => UNIT.to_owned(),
            Base::Hashed => HASHED.to_owned(),
            Base::Named { module, name, args } => {
                let args: Vec<Sort> = args.iter().map(sort).collect();
                if is_prelude(module) {
                    let mut sort = format!("i!{name}");
                    for arg in &args {
                        sort.push('_');
                        sort.push_str(arg);
                    }
                    sort
                } else {
                    kind_sort(module, name, &args)
                }
            }
            Base::Param(name) => format!("u!Param_{name}"),
            // A tuple of the memory model's desugaring (`spec/17 §4`): an uninterpreted sort
            // with a projection per component.
            Base::Tuple(types) => tuple_sort(types),
            Base::Unknown => "i!Unknown".to_owned(),
        },
    }
}

/// The sort of a tuple of `types`, `i!Tuple<n>!<id>`: `<id>` is the first 32 hex digits of
/// SHA-256 over the components' sorts, so the name has the same length whatever `n` is (#333).
/// Each projection names the sort, so spelling every component in it made a query of n
/// components O(n²) long; now it is O(n). Two distinct tuple sorts share a name only if their
/// spellings collide in 128 bits of SHA-256, and a test checks the names of every query of the
/// repository's programs.
fn tuple_sort(types: &[Type]) -> Sort {
    let mut spelling = String::new();
    for ty in types {
        spelling.push_str(&self::sort(ty));
        spelling.push('\n');
    }
    let digest = Sha256::digest(spelling.as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("i!Tuple{}!{hex}", types.len())
}

/// How the values of a type show in a counterexample that cannot print them, `<name>#<n>`
/// (`spec/12 §7`): by the type's source name, never its sort.
fn type_name(ty: &Type) -> String {
    match ty {
        Type::Fn { .. } => "Fn".to_owned(),
        Type::Base { base, .. } => match base {
            Base::Int => "Int".to_owned(),
            Base::Fixed(int) => int.as_str().to_owned(),
            Base::Bool => "Bool".to_owned(),
            Base::Text => "Text".to_owned(),
            Base::Unit => "Unit".to_owned(),
            Base::Hashed => "Hashed".to_owned(),
            Base::Named { name, .. } | Base::Param(name) => name.clone(),
            Base::Tuple(_) => "Tuple".to_owned(),
            Base::Unknown => "Unknown".to_owned(),
        },
    }
}

// ----- the walk -----

/// A value with its Una type, whose sort and fields the encoding needs.
#[derive(Clone, Debug)]
struct Tv {
    f: F,
    ty: Type,
}

/// A hypothesis of the path context; a hidden one defines a temporary and is not listed.
#[derive(Clone, Debug)]
struct Hyp {
    f: F,
    shown: bool,
}

/// The hypotheses of a path, in order: a persistent list, so a context is copied in O(1)
/// whatever its length, and a path extends its parent's hypotheses without copying them
/// (#326). Iteration yields them in the order they were added, as a `Vec` did.
#[derive(Clone, Debug, Default)]
struct Hyps {
    last: Option<std::rc::Rc<HypNode>>,
    len: usize,
}

#[derive(Debug)]
struct HypNode {
    hyp: Hyp,
    previous: Option<std::rc::Rc<HypNode>>,
}

impl Hyps {
    fn push(&mut self, hyp: Hyp) {
        let previous = self.last.take();
        self.last = Some(std::rc::Rc::new(HypNode { hyp, previous }));
        self.len += 1;
    }

    fn len(&self) -> usize {
        self.len
    }

    /// The hypotheses in the order they were added.
    fn iter(&self) -> std::vec::IntoIter<&Hyp> {
        let mut out = Vec::with_capacity(self.len);
        let mut node = self.last.as_deref();
        while let Some(current) = node {
            out.push(&current.hyp);
            node = current.previous.as_deref();
        }
        out.reverse();
        out.into_iter()
    }
}

impl Drop for Hyps {
    // Dropping a long list node by node would recurse: unlink the nodes nothing else holds.
    fn drop(&mut self) {
        let mut node = self.last.take();
        while let Some(current) = node {
            match std::rc::Rc::try_unwrap(current) {
                Ok(mut owned) => node = owned.previous.take(),
                Err(_) => break,
            }
        }
    }
}

/// The path context Π and the types of the binders in scope.
#[derive(Clone, Debug, Default)]
struct Ctx {
    hyps: Hyps,
    /// Shared until changed, so a context is copied in O(1) (#326).
    types: std::rc::Rc<BTreeMap<Name, Type>>,
    /// In a loop body, the equalities that put every enclosing loop at its first iteration:
    /// each state binding at its value on entry, and a range's variable at its lower bound
    /// (ADR 0071 D3, decision N-13). Shared until changed, as `types`.
    entry: std::rc::Rc<Vec<F>>,
    /// The divisions by a value that is not a literal evaluated on this path, with their
    /// divisors (ADR 0054, amendment of 2026-10-08). Shared until changed, as `types`.
    divisions: std::rc::Rc<Vec<(F, Span)>>,
}

impl Ctx {
    fn assume(&mut self, f: F) {
        self.hyps.push(Hyp { f, shown: true });
    }

    fn define(&mut self, f: F) {
        self.hyps.push(Hyp { f, shown: false });
    }
}

/// A normal return path: its context, the value it returns and where it is written.
#[derive(Clone, Debug)]
struct Leaf {
    hyps: Hyps,
    /// The divisions its path evaluates, as [`Ctx::divisions`].
    divisions: std::rc::Rc<Vec<(F, Span)>>,
    value: Option<Tv>,
    span: Span,
    /// For the value of an `attempt`, the normal returns of the attempted call.
    returns: Vec<Leaf>,
}

/// How the terms of a clause are replaced: its binders, `it` and `result`.
struct Env<'a> {
    vars: &'a dyn Fn(&str) -> Option<Tv>,
    it: Option<Tv>,
    result: Option<Tv>,
}

struct Walker<'p, 'i> {
    index: &'i Index<'p>,
    sources: &'i SourceDb,
    module: &'p str,
    declaration: String,
    obligations: Vec<Obligation>,
    /// The definitions of the reflect predicates met so far.
    definitions: BTreeMap<String, Definition>,
    /// How each temporary shows in listed hypotheses.
    temps: BTreeMap<String, String>,
    /// The term each temporary is defined as, where it is one: how it shows.
    defs: BTreeMap<String, F>,
    /// The declaration being walked, for the repairs of its obligations.
    within: Within,
    /// The parameters of the function being walked.
    parameters: BTreeSet<String>,
    /// How each temporary bound to a lambda names it in the obligations of its flows
    /// (ADR 0037 rules 3 and 4).
    lambdas: BTreeMap<Name, Who>,
    /// The number of fresh binders introduced so far for function-type flows (rule 9).
    fresh: usize,
    /// Whether the walk is in the body of a lambda that stands for a declared function, whose
    /// call passes each parameter at its declared type: the refinements of its arguments
    /// hold by the parameters' own, and give no obligation. It holds the source names of
    /// the lambda's parameters, which the goal text of the call's `requires` prints (R-032).
    eta: Option<Vec<String>>,
    /// The name of the named function whose eta-expansion is walked (issue #76).
    eta_name: Option<String>,
    /// The binder that an assignment's refinement obligation is about, which a counterexample
    /// shows as `x (assigned)` (#277).
    assigned: Option<String>,
    /// Whether each state binding of each loop met so far keeps its length (ADR 0071 D1).
    frames: BTreeMap<Span, Vec<bool>>,
    /// The values that loops make fresh, by binder, with their source names (ADR 0071 D3).
    loop_fresh: BTreeMap<String, BTreeSet<String>>,
    /// The binders of loops' final values, with the source names of their components.
    loop_results: BTreeMap<Name, Vec<String>>,
    /// The loop-fresh binders that come from a loop's final value, which no equality of an
    /// enclosing loop's first iteration fixes (ADR 0071 D3).
    unpinned: BTreeSet<String>,
    /// Whether the paths of the return obligation being pushed return such a value.
    returned_unpinned: bool,
    /// The loop-fresh values that the paths of the return obligation being pushed return.
    returned_forgotten: Vec<String>,
}

/// How the obligations of a function value flowing into a function type name the value and
/// the binders of its parameters (ADR 0037 rules 3 and 4).
#[derive(Clone, Debug)]
struct Who {
    /// `takes_pos` for a named function, `the lambda`, or the value's source text.
    name: String,
    /// The binders of its parameters: the declared or written names; empty when the value
    /// has no names of its own, and the binders are then `arg1` to `argN`.
    params: Vec<String>,
}

impl<'p, 'i> Walker<'p, 'i> {
    fn new(index: &'i Index<'p>, sources: &'i SourceDb, module: &'p str) -> Self {
        Walker {
            index,
            sources,
            module,
            declaration: String::new(),
            obligations: Vec::new(),
            definitions: BTreeMap::new(),
            temps: BTreeMap::new(),
            defs: BTreeMap::new(),
            within: Within::Other,
            parameters: BTreeSet::new(),
            lambdas: BTreeMap::new(),
            fresh: 0,
            eta: None,
            eta_name: None,
            assigned: None,
            frames: BTreeMap::new(),
            loop_fresh: BTreeMap::new(),
            loop_results: BTreeMap::new(),
            returned_forgotten: Vec::new(),
            unpinned: BTreeSet::new(),
            returned_unpinned: false,
        }
    }

    fn text(&self, span: Span) -> String {
        self.sources
            .file(span.file)
            .text()
            .get(span.start..span.end)
            .unwrap_or("")
            .to_owned()
    }

    // ----- declarations -----

    fn function(&mut self, fun: &'p Fun) {
        self.declaration = format!("{}.{}", self.module, fun.name);
        self.lambdas.clear();
        self.within = Within::Function { public: fun.public };
        self.parameters = fun.params.iter().map(|(name, _)| name.clone()).collect();
        let mut ctx = Ctx::default();
        for (name, ty) in &fun.params {
            self.bind(&mut ctx, name, ty);
        }
        for clause in &fun.requires {
            let vars = |name: &str| Some(own(&ctx, name));
            let env = Env {
                vars: &vars,
                it: None,
                result: None,
            };
            if let Some(f) = self.term(&clause.term, &env) {
                ctx.assume(f.f);
            }
        }
        let mut leaves = Vec::new();
        self.forget_loops();
        self.walk(&fun.body, &ctx, &mut leaves);
        // ADR 0037 rule 2: a function value returned into a declared function type.
        self.returned_functions(&ctx, &leaves, &fun.result, None);
        let result = Tv {
            f: constant("result", &fun.result),
            ty: fun.result.clone(),
        };
        // With `edit` parameters, the result is the last component of the tuple (`spec/17
        // §4`); the components of the parameters hold by their rebinds.
        let subject = match &fun.result {
            Type::Base {
                base: Base::Tuple(types),
                ..
            } => projection(&result, types.len().saturating_sub(1)),
            _ => Some(result.clone()),
        };
        if let Some(subject) = &subject {
            for refinement in refinements(&subject.ty) {
                self.result_refinement(&ctx, &leaves, &result, subject, refinement, fun.span);
            }
        }
        // ADR 0038 D5: the result of a policy's `via` function satisfies the refinement of the
        // policy's `to` type, which T-Declassify assumes (`spec/12 §3`, `spec/07 §5` rule 3).
        for refinement in self
            .index
            .via_refinements(self.module, &fun.name, &fun.result)
        {
            self.result_refinement(&ctx, &leaves, &result, &result, refinement, fun.span);
        }
        for clause in &fun.ensures {
            let goal = {
                let vars = |name: &str| Some(own(&ctx, name));
                let env = Env {
                    vars: &vars,
                    it: None,
                    result: Some(result.clone()),
                };
                self.term(&clause.term, &env)
            };
            let source = clause.span.unwrap_or(fun.span);
            self.returns(
                ObligationKind::Postcondition,
                &ctx,
                &leaves,
                &result,
                goal,
                clause.text.clone(),
                source,
                clause.span.map(|span| (span, "ensures clause")),
            );
        }
    }

    /// A `refinement` obligation over every return path: `subject`, `result` or a component
    /// of it, satisfies `refinement`.
    fn result_refinement(
        &mut self,
        ctx: &Ctx,
        leaves: &[Leaf],
        result: &Tv,
        subject: &Tv,
        refinement: &Refinement,
        source: Span,
    ) {
        let goal = {
            let vars = |name: &str| Some(own(ctx, name));
            let env = Env {
                vars: &vars,
                it: Some(subject.clone()),
                result: Some(subject.clone()),
            };
            self.term(&refinement.clause.term, &env)
        };
        let text = print_term_with(&refinement.clause.term, &|term| {
            matches!(term, Term::It).then(|| "result".to_owned())
        });
        self.returns(
            ObligationKind::Refinement,
            ctx,
            leaves,
            result,
            goal,
            text,
            source,
            refinement
                .clause
                .span
                .map(|span| (span, clause_role(refinement))),
        );
    }

    /// An operation of a handler, checked against the contract of its capability's operation
    /// (`spec/10 §8`).
    fn handler_op(
        &mut self,
        handler_params: &'p [(Name, Type)],
        capability: &una_core::Path,
        op: &'p una_core::HandlerOp,
    ) {
        let mut ctx = Ctx::default();
        for (name, ty) in handler_params {
            self.bind(&mut ctx, name, ty);
        }
        let signature = self.index.operation(capability, &op.name);
        let mut renames: BTreeMap<&str, &str> = BTreeMap::new();
        if let Some(signature) = signature {
            for ((name, ty), binder) in signature.params.iter().zip(&op.params) {
                self.bind(&mut ctx, binder, ty);
                renames.insert(name.as_str(), binder.as_str());
            }
            for clause in &signature.requires {
                let vars = |name: &str| renames.get(name).map(|binder| own(&ctx, binder));
                let env = Env {
                    vars: &vars,
                    it: None,
                    result: None,
                };
                if let Some(f) = self.term(&clause.term, &env) {
                    ctx.assume(f.f);
                }
            }
        }
        let mut leaves = Vec::new();
        self.forget_loops();
        self.walk(&op.body, &ctx, &mut leaves);
        let Some(signature) = signature else {
            return;
        };
        // ADR 0037 rule 2: a function value returned by a handler operation flows into the
        // operation's declared result, as a function's does into its own (flow sweep).
        self.returned_functions(&ctx, &leaves, &signature.result, None);
        let result = Tv {
            f: constant("result", &signature.result),
            ty: signature.result.clone(),
        };
        let Some(span) = leaves.first().map(|leaf| leaf.span) else {
            return;
        };
        for refinement in refinements(&signature.result) {
            let goal = {
                let env = Env {
                    vars: &|_| None,
                    it: Some(result.clone()),
                    result: Some(result.clone()),
                };
                self.term(&refinement.clause.term, &env)
            };
            let text = print_term_with(&refinement.clause.term, &|term| {
                matches!(term, Term::It).then(|| "result".to_owned())
            });
            self.returns(
                ObligationKind::Refinement,
                &ctx,
                &leaves,
                &result,
                goal,
                text,
                span,
                refinement
                    .clause
                    .span
                    .map(|span| (span, clause_role(refinement))),
            );
        }
        for clause in &signature.ensures {
            let goal = {
                let vars = |name: &str| renames.get(name).map(|binder| own(&ctx, binder));
                let env = Env {
                    vars: &vars,
                    it: None,
                    result: Some(result.clone()),
                };
                self.term(&clause.term, &env)
            };
            self.returns(
                ObligationKind::Postcondition,
                &ctx,
                &leaves,
                &result,
                goal,
                clause.text.clone(),
                clause.span.unwrap_or(span),
                clause.span.map(|span| (span, "ensures clause")),
            );
        }
    }

    /// The body of an opaque predicate: code, whose obligations are generated as usual.
    fn opaque(&mut self, params: &'p [(Name, Type)], body: &'p Expr) {
        let mut ctx = Ctx::default();
        for (name, ty) in params {
            self.bind(&mut ctx, name, ty);
        }
        let mut leaves = Vec::new();
        self.forget_loops();
        self.walk(body, &ctx, &mut leaves);
    }

    /// Brings a binder into scope with the refinements of its type as hypotheses.
    fn bind(&mut self, ctx: &mut Ctx, name: &str, ty: &Type) {
        std::rc::Rc::make_mut(&mut ctx.types).insert(name.to_owned(), ty.clone());
        let value = own(ctx, name);
        self.assume_refinements(ctx, &value);
    }

    fn assume_refinements(&mut self, ctx: &mut Ctx, value: &Tv) {
        // A value of a fixed-width type lies in its range (`spec/18 §6`).
        if let Type::Base {
            base: Base::Fixed(int),
            ..
        } = &value.ty
            && value.f.sort() == "Int"
        {
            ctx.define(in_range(value.f.clone(), *int));
        }
        for refinement in refinements(&value.ty) {
            let env = Env {
                vars: &|_| None,
                it: Some(value.clone()),
                result: None,
            };
            if let Some(f) = self.term(&refinement.clause.term, &env) {
                ctx.assume(f.f);
            }
        }
    }

    // ----- expressions -----

    fn walk(&mut self, expr: &'p Expr, start: &Ctx, leaves: &mut Vec<Leaf>) {
        let mut ctx = start.clone();
        let mut expr = expr;
        loop {
            match expr {
                Expr::Atom { atom, span } => {
                    leaves.push(Leaf {
                        hyps: ctx.hyps.clone(),
                        divisions: ctx.divisions.clone(),
                        value: Some(self.atom(atom, &ctx)),
                        span: *span,
                        returns: Vec::new(),
                    });
                    return;
                }
                // A `for` loop (`spec/17 §4`, `§5` rule 3; ADR 0056, amendment of
                // 2026-10-02). The body is checked once, for any iteration: the state is fresh
                // at its declared types, whose refinements are the invariant, and the context
                // of the loop is kept, since the bindings it does not update keep their values.
                // The loop's value says nothing but the types of its components, so no fact
                // about the state survives it.
                Expr::Loop {
                    source,
                    var,
                    state,
                    init,
                    body,
                    span,
                } => {
                    let mut body_ctx = ctx.clone();
                    self.bind(&mut body_ctx, &var.0, &var.1);
                    if let LoopSource::Range {
                        low,
                        high,
                        inclusive,
                    } = source.as_ref()
                    {
                        let x = own(&body_ctx, &var.0);
                        let low = self.atom(low, &ctx);
                        let high = self.atom(high, &ctx);
                        if x.f.sort() == low.f.sort() && x.f.sort() == high.f.sort() {
                            std::rc::Rc::make_mut(&mut body_ctx.entry)
                                .push(F::eq(x.f.clone(), low.f.clone()));
                            body_ctx.assume(F::op("<=", vec![low.f, x.f.clone()]));
                            let op = if *inclusive { "<=" } else { "<" };
                            body_ctx.assume(F::op(op, vec![x.f, high.f]));
                        }
                    }
                    // The body of a loop over a list runs only when the list has an element
                    // (#278).
                    if let LoopSource::List(list) = source.as_ref() {
                        let list = self.atom(list, &ctx);
                        if let Some(length) = self.len(&list) {
                            body_ctx.assume(F::op(">=", vec![length, F::Int("1".to_owned())]));
                        }
                    }
                    for ((name, ty), atom) in state.iter().zip(init) {
                        self.bind(&mut body_ctx, name, ty);
                        self.loop_fresh
                            .insert(name.clone(), BTreeSet::from([source_name(name)]));
                        // A state that starts from an earlier loop's final value is fixed by
                        // no first iteration (ADR 0071 D3, #278).
                        if let Atom::Var(start) = atom
                            && self.unpinned.contains(start)
                        {
                            self.unpinned.insert(name.clone());
                        }
                    }
                    for ((name, _), atom) in state.iter().zip(init) {
                        let x = own(&body_ctx, name);
                        let entry = self.atom(atom, &ctx);
                        if x.f.sort() == entry.f.sort() {
                            std::rc::Rc::make_mut(&mut body_ctx.entry).push(F::eq(x.f, entry.f));
                        }
                    }
                    // ADR 0071 D2: a length-preserving binding keeps its length at entry.
                    let preserved = self.preserved(*span, state, body);
                    for (((name, _), atom), kept) in state.iter().zip(init).zip(&preserved) {
                        if !kept {
                            continue;
                        }
                        let x = own(&body_ctx, name);
                        let entry = self.atom(atom, &ctx);
                        if let (Some(now), Some(then)) = (self.len(&x), self.len(&entry)) {
                            body_ctx.assume(F::eq(now, then));
                        }
                    }
                    let mut body_leaves = Vec::new();
                    self.walk(body, &body_ctx, &mut body_leaves);
                    leaves.push(Leaf {
                        hyps: ctx.hyps.clone(),
                        divisions: ctx.divisions.clone(),
                        value: None,
                        span: *span,
                        returns: Vec::new(),
                    });
                    return;
                }
                // `together_for x in s. e` (`spec/19 §4`): the task is checked once, for any
                // element, in the context before it, with the facts of a range on its variable
                // as in a loop. The list it makes says nothing but its type.
                Expr::TogetherFor {
                    source,
                    var,
                    task,
                    span,
                } => {
                    let mut task_ctx = ctx.clone();
                    self.bind(&mut task_ctx, &var.0, &var.1);
                    if let LoopSource::Range {
                        low,
                        high,
                        inclusive,
                    } = source.as_ref()
                    {
                        let x = own(&task_ctx, &var.0);
                        let low = self.atom(low, &ctx);
                        let high = self.atom(high, &ctx);
                        if x.f.sort() == low.f.sort() && x.f.sort() == high.f.sort() {
                            task_ctx.assume(F::op("<=", vec![low.f, x.f.clone()]));
                            let op = if *inclusive { "<=" } else { "<" };
                            task_ctx.assume(F::op(op, vec![x.f, high.f]));
                        }
                    }
                    let mut task_leaves = Vec::new();
                    self.walk(task, &task_ctx, &mut task_leaves);
                    leaves.push(Leaf {
                        hyps: ctx.hyps.clone(),
                        divisions: ctx.divisions.clone(),
                        value: None,
                        span: *span,
                        returns: Vec::new(),
                    });
                    return;
                }
                // The tasks of a `together` block outside the `let` that binds their tuple,
                // which the desugar never makes: each is checked in the context before it.
                Expr::Together { tasks, span } => {
                    for task in tasks {
                        let mut task_leaves = Vec::new();
                        self.walk(task, &ctx, &mut task_leaves);
                    }
                    leaves.push(Leaf {
                        hyps: ctx.hyps.clone(),
                        divisions: ctx.divisions.clone(),
                        value: None,
                        span: *span,
                        returns: Vec::new(),
                    });
                    return;
                }
                // `let (x, ...) = a in e`: each binder is a projection of the tuple
                // (`spec/17 §4`), with the refinements of its type.
                Expr::LetTuple {
                    names, value, body, ..
                } => {
                    let tuple = self.atom(value, &ctx);
                    // The components of a loop's final value are fresh (ADR 0071 D3).
                    if let Atom::Var(tuple_name) = value
                        && let Some(sources) = self.loop_results.get(tuple_name).cloned()
                    {
                        for ((name, _), source) in names.iter().zip(sources) {
                            self.loop_fresh
                                .insert(name.clone(), BTreeSet::from([source]));
                            self.unpinned.insert(name.clone());
                        }
                    } else if let Atom::Var(tuple_name) = value
                        && let Some(sources) = self.loop_fresh.get(tuple_name).cloned()
                    {
                        let unpinned = self.unpinned.contains(tuple_name);
                        for (name, _) in names {
                            self.loop_fresh.insert(name.clone(), sources.clone());
                            if unpinned {
                                self.unpinned.insert(name.clone());
                            }
                        }
                    }
                    for (index, (name, ty)) in names.iter().enumerate() {
                        std::rc::Rc::make_mut(&mut ctx.types).insert(name.clone(), ty.clone());
                        let x = own(&ctx, name);
                        if let Some(component) = projection(&tuple, index)
                            && component.f.sort() == x.f.sort()
                        {
                            ctx.define(F::eq(x.f.clone(), component.f.clone()));
                            if name.starts_with('%') {
                                self.defs.insert(name.clone(), component.f);
                            }
                        }
                        self.assume_refinements(&mut ctx, &x);
                    }
                    expr = body;
                }
                Expr::Unreachable { span, goal } => {
                    // The path after the last arm is infeasible.
                    self.site(
                        ObligationKind::Exhaustive,
                        &ctx,
                        Some(F::Bool(false)),
                        goal.clone(),
                        *span,
                        None,
                    );
                    return;
                }
                Expr::Let {
                    name,
                    ty,
                    value,
                    body,
                    span,
                    written,
                } => {
                    self.taint(name, value);
                    match value.as_ref() {
                        Bound::Rhs(rhs) => self.rhs(name, ty, rhs, *span, &mut ctx),
                        // The tuple of a `together` block (`spec/19 §4`): each task is checked
                        // in the context before the block, with none of its siblings' facts,
                        // and each component of the tuple is the value of one of its task's
                        // paths, so each branch's binding carries its facts after the join.
                        Bound::Expr(Expr::Together { tasks, .. }) => {
                            std::rc::Rc::make_mut(&mut ctx.types).insert(name.clone(), ty.clone());
                            let x = own(&ctx, name);
                            let base = ctx.hyps.len();
                            let mut paths = Vec::new();
                            for (index, task) in tasks.iter().enumerate() {
                                let mut task_leaves = Vec::new();
                                self.walk(task, &ctx, &mut task_leaves);
                                let Some(component) = projection(&x, index) else {
                                    continue;
                                };
                                self.returned_functions(
                                    &ctx,
                                    &task_leaves,
                                    &component.ty,
                                    Some(*span),
                                );
                                if !task_leaves.is_empty() {
                                    let options: Vec<F> = task_leaves
                                        .iter()
                                        .map(|leaf| self.path(leaf, base, &component))
                                        .collect();
                                    paths.push(F::or(options));
                                }
                            }
                            for path in paths {
                                ctx.define(path);
                            }
                        }
                        Bound::Expr(inner) => {
                            let mut inner_leaves = Vec::new();
                            self.walk(inner, &ctx, &mut inner_leaves);
                            // No path of the bound expression completes, as `fail`: the rest
                            // is never reached, so it has no obligation and no return (#274).
                            if inner_leaves.is_empty() {
                                return;
                            }
                            // ADR 0037 rule 2: the value of each path flows into the binder,
                            // at the bound expression.
                            self.returned_functions(&ctx, &inner_leaves, ty, Some(*span));
                            std::rc::Rc::make_mut(&mut ctx.types).insert(name.clone(), ty.clone());
                            let x = own(&ctx, name);
                            // `spec/03 §7`: a value in the refinement language is selfified;
                            // `and`, `or`, `implies`, `in` and `is` are branches in Core.
                            let mut locals = BTreeMap::new();
                            if let Some(value) = self.pure(inner, &ctx, &mut locals)
                                && value.sort() == x.f.sort()
                            {
                                ctx.define(F::eq(x.f.clone(), value.clone()));
                                if name.starts_with('%') {
                                    self.defs.insert(name.clone(), value);
                                }
                            } else if !inner_leaves.is_empty() {
                                // The binder is the value of one of the paths.
                                let base = ctx.hyps.len();
                                let paths: Vec<F> = inner_leaves
                                    .iter()
                                    .map(|leaf| self.path(leaf, base, &x))
                                    .collect();
                                ctx.define(F::or(paths));
                            }
                            // ADR 0071 D2: after a loop, a length-preserving binding has its
                            // length at entry; the others are fresh (D3).
                            if let Expr::Loop {
                                state,
                                init,
                                body,
                                span: loop_span,
                                ..
                            } = inner
                            {
                                let preserved = self.preserved(*loop_span, state, body);
                                let sources = state.iter().map(|(name, _)| source_name(name));
                                self.loop_results.insert(name.clone(), sources.collect());
                                for (index, ((_, _), atom)) in state.iter().zip(init).enumerate() {
                                    let Some(component) = projection(&x, index) else {
                                        continue;
                                    };
                                    if !preserved.get(index).copied().unwrap_or(false) {
                                        continue;
                                    }
                                    let entry = self.atom(atom, &ctx);
                                    if let (Some(now), Some(then)) =
                                        (self.len(&component), self.len(&entry))
                                    {
                                        ctx.define(F::eq(now, then));
                                    }
                                }
                            }
                            // The list of a `together for` has one element per task (`spec/19
                            // §4` rule 2).
                            if let Expr::TogetherFor { source, .. } = inner
                                && let Some(length) = self.len(&x)
                                && let Some(count) = self.task_count(source, &ctx)
                            {
                                ctx.define(F::eq(length, count));
                            }
                        }
                    }
                    if name.starts_with('%') && !self.temps.contains_key(name.as_str()) {
                        let text = self.text(*span);
                        self.temps.insert(name.clone(), text);
                    }
                    let x = own(&ctx, name);
                    if *written {
                        let value_text = self.text(*span);
                        // An assignment's refinement is about the new value; the counterexample
                        // names it apart from the value before (#277).
                        self.assigned = name.contains('%').then(|| name.clone());
                        for refinement in refinements(ty) {
                            let env = Env {
                                vars: &|_| None,
                                it: Some(x.clone()),
                                result: None,
                            };
                            let goal = self.term(&refinement.clause.term, &env);
                            let text = instantiate(&refinement.clause.term, &value_text);
                            self.site(
                                ObligationKind::Refinement,
                                &ctx,
                                goal.map(|g| g.f),
                                text,
                                *span,
                                refinement
                                    .clause
                                    .span
                                    .map(|span| (span, clause_role(refinement))),
                            );
                        }
                        self.assigned = None;
                    }
                    self.assume_refinements(&mut ctx, &x);
                    expr = body;
                }
                Expr::If {
                    cond,
                    then,
                    otherwise,
                } => {
                    let condition = self.atom(cond, &ctx).f;
                    let mut then_ctx = ctx.clone();
                    then_ctx.assume(condition.clone());
                    self.walk(then, &then_ctx, leaves);
                    ctx.assume(F::not(condition));
                    expr = otherwise;
                }
                Expr::Case {
                    scrutinee,
                    arms,
                    otherwise,
                } => {
                    let value = self.atom(scrutinee, &ctx);
                    for arm in arms {
                        let mut arm_ctx = ctx.clone();
                        if let Some(tag) = self.tag(&value) {
                            arm_ctx.assume(F::eq(tag, F::Int(arm.ctor.index.to_string())));
                        }
                        for (index, bind) in arm.binds.iter().enumerate() {
                            let Some(bind) = bind else {
                                continue;
                            };
                            let Some(field_ty) =
                                self.index.field_type(&value.ty, arm.ctor.index, index)
                            else {
                                continue;
                            };
                            std::rc::Rc::make_mut(&mut arm_ctx.types)
                                .insert(bind.clone(), field_ty.clone());
                            // A temporary that unwraps the value of `if none` or `if error`
                            // shows as the expression it unwraps (final review F-004).
                            if bind.starts_with('%')
                                && !self.temps.contains_key(bind.as_str())
                                && let Atom::Var(scrutinee) = scrutinee
                            {
                                let shown = self.show_name(scrutinee);
                                self.temps.insert(bind.clone(), shown);
                            }
                            let x = own(&arm_ctx, bind);
                            if let Some(field) = self.field(&value, arm.ctor.index, index) {
                                arm_ctx.define(F::eq(x.f.clone(), field.f));
                            }
                            self.assume_refinements(&mut arm_ctx, &x);
                        }
                        self.walk(&arm.body, &arm_ctx, leaves);
                    }
                    let Some(otherwise) = otherwise else {
                        return;
                    };
                    if let Some(tag) = self.tag(&value) {
                        for arm in arms {
                            ctx.assume(F::not(F::eq(
                                tag.clone(),
                                F::Int(arm.ctor.index.to_string()),
                            )));
                        }
                    }
                    expr = otherwise;
                }
                Expr::Fail {
                    ctor,
                    args,
                    arg_spans,
                    ..
                } => {
                    self.fields(ctor, args, arg_spans, &ctx);
                    return;
                }
                Expr::Handle {
                    body,
                    error: _,
                    handler,
                } => {
                    self.walk(body, &ctx, leaves);
                    expr = handler;
                }
                Expr::Attempt { body, span } => {
                    let mut returns = Vec::new();
                    self.walk(body, &ctx, &mut returns);
                    leaves.push(Leaf {
                        hyps: ctx.hyps.clone(),
                        divisions: ctx.divisions.clone(),
                        value: None,
                        span: *span,
                        returns,
                    });
                    return;
                }
                Expr::With { source, body, .. } => {
                    if let Source::Handler {
                        handler,
                        args,
                        arg_spans,
                        ..
                    } = source
                        && let Some(params) = self
                            .index
                            .handlers
                            .get(&(handler.module.as_str(), handler.name.as_str()))
                    {
                        self.arguments(params, args, arg_spans, &ctx);
                    }
                    expr = body;
                }
            }
        }
    }

    /// `x is Ok implies ...` for the value `x` of an `attempt`: `Ok` is a normal return of the
    /// attempted call (`spec/06 §5`), so the facts of one of its return paths hold, among them
    /// the callee's `ensures`, and `x.value` is its value; `Err` carries nothing, as failures
    /// have no postcondition (`spec/04 §2`).
    fn attempted(&self, x: &Tv, leaf: &Leaf) -> Option<F> {
        if leaf.returns.is_empty() {
            return None;
        }
        let tag = self.tag(x)?;
        let payload = self.field(x, 0, 0)?;
        let base = leaf.hyps.len();
        let paths: Vec<F> = leaf
            .returns
            .iter()
            .map(|path| self.path(path, base, &payload))
            .collect();
        Some(F::op(
            "=>",
            vec![F::eq(tag, F::Int("0".to_owned())), F::or(paths)],
        ))
    }

    /// One path of an expression as a formula: its facts after the first `base` hypotheses,
    /// and `x` equal to its value, or for an `attempt`, to what the call returned.
    fn path(&self, leaf: &Leaf, base: usize, x: &Tv) -> F {
        let mut parts: Vec<F> = leaf.hyps.iter().skip(base).map(|h| h.f.clone()).collect();
        if let Some(value) = &leaf.value
            && value.f.sort() == x.f.sort()
        {
            parts.push(F::eq(x.f.clone(), value.f.clone()));
        }
        if let Some(ok) = self.attempted(x, leaf) {
            parts.push(ok);
        }
        F::and(parts)
    }

    /// A pure expression of Core as one term, when it is one: atoms, primitives and fields
    /// bound by `let`, `if`, and `case` on a single variant without bindings, which is how
    /// `and`, `or`, `implies`, `in` and `is` elaborate.
    fn pure(&mut self, expr: &Expr, ctx: &Ctx, locals: &mut BTreeMap<String, Tv>) -> Option<F> {
        let value = |walker: &mut Self, atom: &Atom, locals: &BTreeMap<String, Tv>| match atom {
            Atom::Var(name) => locals.get(name).cloned().unwrap_or_else(|| own(ctx, name)),
            other => walker.atom(other, ctx),
        };
        match expr {
            Expr::Atom { atom, .. } => Some(value(self, atom, locals).f),
            Expr::Let {
                name,
                ty,
                value: bound,
                body,
                ..
            } => {
                let Bound::Rhs(rhs) = bound.as_ref() else {
                    return None;
                };
                let bound_value = match rhs {
                    Rhs::Atom(atom) => value(self, atom, locals),
                    Rhs::Prim { prim, args, .. } => {
                        let values: Vec<Tv> = args.iter().map(|a| value(self, a, locals)).collect();
                        Tv {
                            f: self.prim(*prim, &values)?,
                            ty: ty.clone(),
                        }
                    }
                    Rhs::Field {
                        value: record,
                        index,
                        ..
                    } => {
                        let record = value(self, record, locals);
                        self.field(&record, 0, *index)?
                    }
                    _ => return None,
                };
                locals.insert(name.clone(), bound_value);
                self.pure(body, ctx, locals)
            }
            Expr::If {
                cond,
                then,
                otherwise,
            } => {
                let condition = value(self, cond, locals).f;
                let then = self.pure(then, ctx, locals)?;
                let otherwise = self.pure(otherwise, ctx, locals)?;
                Some(F::op("ite", vec![condition, then, otherwise]))
            }
            Expr::Case {
                scrutinee,
                arms,
                otherwise: Some(otherwise),
            } => {
                let [arm] = arms.as_slice() else {
                    return None;
                };
                if arm.binds.iter().any(Option::is_some) {
                    return None;
                }
                let scrutinee = value(self, scrutinee, locals);
                let test = F::eq(self.tag(&scrutinee)?, F::Int(arm.ctor.index.to_string()));
                let then = self.pure(&arm.body, ctx, locals)?;
                let otherwise = self.pure(otherwise, ctx, locals)?;
                Some(F::op("ite", vec![test, then, otherwise]))
            }
            _ => None,
        }
    }

    /// `let x: ty = rhs`: the obligations of the right-hand side, then its facts.
    fn rhs(&mut self, name: &str, ty: &Type, rhs: &'p Rhs, span: Span, ctx: &mut Ctx) {
        // ADR 0037 rule 2: a function value bound to a binder flows into the binder's type.
        if let Rhs::Atom(atom @ Atom::Var(_)) = rhs
            && contains_fn(ty)
        {
            let value = self.atom(atom, ctx);
            let who = self.who(&value);
            self.fn_flow(&value.ty, ty, &who, span, ctx);
        }
        std::rc::Rc::make_mut(&mut ctx.types).insert(name.to_owned(), ty.clone());
        let x = own(ctx, name);
        match rhs {
            Rhs::Atom(atom) => {
                let value = self.atom(atom, ctx);
                if value.f.sort() == x.f.sort() {
                    ctx.define(F::eq(x.f.clone(), value.f.clone()));
                    if name.starts_with('%') {
                        self.defs.insert(name.to_owned(), value.f);
                    }
                }
                self.show_temp(name, atom);
            }
            Rhs::Prim {
                prim,
                args,
                arg_spans,
                span,
            } => {
                let values: Vec<Tv> = args.iter().map(|a| self.atom(a, ctx)).collect();
                if matches!(prim, Prim::Div | Prim::Mod)
                    && let (Some(divisor), Some(divisor_atom)) = (values.get(1), args.get(1))
                    && !matches!(divisor_atom, Atom::Int(digits) if digits.trim_start_matches('-') != "0")
                    // A negated literal, as in `a / -1`, is a nonzero literal divisor, as the
                    // checker reads it (`spec/12 §3`).
                    && !nonzero_literal(&divisor.f)
                    && !matches!(divisor_atom, Atom::Var(name) if self.defs.get(name).is_some_and(nonzero_literal))
                {
                    let text = format!(
                        "{} != 0",
                        arg_spans.get(1).map_or_else(String::new, |s| self.text(*s))
                    );
                    self.site(
                        ObligationKind::Division,
                        ctx,
                        Some(F::not(F::eq(divisor.f.clone(), F::Int("0".to_owned())))),
                        text,
                        *span,
                        None,
                    );
                    std::rc::Rc::make_mut(&mut ctx.divisions).push((divisor.f.clone(), *span));
                }
                self.fixed_sites(ty, *prim, args, &values, arg_spans, *span, ctx);
                if let Prim::Fixed(op, int) = prim {
                    self.fixed_facts(*op, *int, args, &values, &x.f, ctx);
                }
                if let Prim::Map(op) = prim {
                    self.map_facts(*op, &values, &x, ctx);
                }
                if let Some(value) = self.prim(*prim, &values)
                    && value.sort() == x.f.sort()
                {
                    ctx.define(F::eq(x.f.clone(), value.clone()));
                    if name.starts_with('%') {
                        self.defs.insert(name.to_owned(), value);
                    }
                    let texts: Vec<String> = arg_spans.iter().map(|s| self.text(*s)).collect();
                    if name.starts_with('%') {
                        self.temps
                            .insert(name.to_owned(), prim_text(*prim, &texts, &self.text(*span)));
                    }
                }
            }
            Rhs::Call {
                function,
                type_args,
                args,
                arg_spans,
                span,
            } => {
                let values: Vec<Tv> = args.iter().map(|a| self.atom(a, ctx)).collect();
                let key = (function.module.as_str(), function.name.as_str());
                if let Some(contract) = self.index.contracts.get(&key) {
                    // A generic callee's parameters, at its type arguments, for the flows of
                    // function values (ADR 0037 rule 2).
                    let params: Vec<(Name, Type)> = contract
                        .params
                        .iter()
                        .map(|(name, ty)| {
                            (
                                name.clone(),
                                substitute(ty, contract.type_params, type_args),
                            )
                        })
                        .collect();
                    let result = substitute(contract.result, contract.type_params, type_args);
                    let contract = Contract {
                        params: &params,
                        instantiated_result: Some(&result),
                        ..*contract
                    };
                    self.call(&contract, &values, arg_spans, *span, &x, ctx);
                    if is_prelude(&function.module) {
                        self.text_facts(&function.name, &values, &x, ctx);
                    }
                } else if let Some((params, formula)) = self.index.predicates.get(&key) {
                    self.arguments(params, args, arg_spans, ctx);
                    let predicate = self.predicate(function, params, *formula, &values);
                    if let Some(predicate) = predicate {
                        ctx.define(F::eq(x.f.clone(), predicate));
                    }
                }
            }
            // ADR 0037 rule 7: a call through a function value proves the refinements of its
            // parameters at the arguments and assumes those of its result (T-App).
            Rhs::Apply {
                callee,
                args,
                arg_spans,
                ..
            } => {
                let callee = self.atom(callee, ctx);
                if let Type::Fn { params, result, .. } = &callee.ty {
                    for ((param, atom), arg_span) in params.iter().zip(args).zip(arg_spans) {
                        let value = self.atom(atom, ctx);
                        self.refinement_sites(param, &value, *arg_span, ctx);
                    }
                    let value = Tv {
                        f: x.f.clone(),
                        ty: (**result).clone(),
                    };
                    self.assume_refinements(ctx, &value);
                }
            }
            Rhs::Op {
                capability_path,
                op,
                args,
                arg_spans,
                span,
                ..
            } => {
                let values: Vec<Tv> = args.iter().map(|a| self.atom(a, ctx)).collect();
                let signature = self.index.operation(capability_path, op);
                if let Some(signature) = signature {
                    let contract = Contract {
                        type_params: &[],
                        params: &signature.params,
                        result: &signature.result,
                        requires: &signature.requires,
                        ensures: &signature.ensures,
                        fails: signature.fails.as_deref(),
                        instantiated_result: None,
                    };
                    self.call(&contract, &values, arg_spans, *span, &x, ctx);
                }
            }
            Rhs::Construct {
                ctor,
                args,
                arg_spans,
            } => {
                self.fields(ctor, args, arg_spans, ctx);
                self.field_functions(&x.ty, ctor, args, arg_spans, ctx);
                if let Some(tag) = self.tag(&x) {
                    ctx.define(F::eq(tag, F::Int(ctor.index.to_string())));
                }
                for (index, atom) in args.iter().enumerate() {
                    let value = self.atom(atom, ctx);
                    if let Some(field) = self.field(&x, ctor.index, index)
                        && field.f.sort() == value.f.sort()
                    {
                        ctx.define(F::eq(field.f, value.f));
                    }
                }
            }
            Rhs::Field { value, index, .. } => {
                let record = self.atom(value, ctx);
                if let Some(projection) = self.field(&record, 0, *index)
                    && projection.f.sort() == x.f.sort()
                {
                    ctx.define(F::eq(x.f.clone(), projection.f.clone()));
                    if name.starts_with('%') {
                        self.defs.insert(name.to_owned(), projection.f);
                    }
                }
            }
            // `(a, ...)`: each projection of the binder is a component (`spec/17 §4`).
            Rhs::Tuple(items) => {
                for (index, atom) in items.iter().enumerate() {
                    let value = self.atom(atom, ctx);
                    if let Some(component) = projection(&x, index)
                        && component.f.sort() == value.f.sort()
                    {
                        ctx.define(F::eq(component.f, value.f));
                    }
                }
            }
            Rhs::List(items) => {
                // ADR 0037 rule 2: each function value of a list flows into its element type.
                if let Type::Base {
                    base: Base::Named { args, .. },
                    ..
                } = ty
                    && let Some(element) = args.first()
                    && contains_fn(element)
                {
                    for item in items {
                        let value = self.atom(item, ctx);
                        let who = self.who(&value);
                        self.fn_flow(&value.ty, element, &who, span, ctx);
                    }
                }
                if let Some(len) = self.len(&x) {
                    ctx.define(F::eq(len, F::Int(items.len().to_string())));
                }
            }
            Rhs::Lambda {
                params,
                body,
                function,
            } => {
                let mut inner = ctx.clone();
                for (param, ty) in params {
                    self.bind(&mut inner, param, ty);
                }
                let mut leaves = Vec::new();
                let names = function
                    .as_ref()
                    .map(|_| params.iter().map(|(p, _)| source_name(p)).collect());
                let eta = std::mem::replace(&mut self.eta, names);
                let eta_name = std::mem::replace(
                    &mut self.eta_name,
                    function.as_ref().map(|f| f.name.clone()),
                );
                self.walk(body, &inner, &mut leaves);
                self.eta = eta;
                self.eta_name = eta_name;
                let who = Who {
                    name: function
                        .as_ref()
                        .map_or_else(|| "the lambda".to_owned(), |f| f.name.clone()),
                    params: params.iter().map(|(p, _)| source_name(p)).collect(),
                };
                // The lambda's own type: its parameters, and the result of the function it
                // stands for, or for a lambda of the source the result its binder expects,
                // which its body's paths flow into (`spec/12 §3`, returns).
                let expected = match ty {
                    Type::Fn { result, .. } => Some(result.as_ref()),
                    Type::Base { .. } => None,
                };
                let result = match (function, body.as_ref()) {
                    (Some(_), Expr::Let { ty: result, .. }) => Some(result),
                    (Some(_), _) => None,
                    (None, _) => {
                        if let Some(expected) = expected {
                            self.lambda_result(&inner, &leaves, expected, &who, span);
                        }
                        expected
                    }
                };
                if let (
                    Type::Fn {
                        uses, fails, label, ..
                    },
                    Some(result),
                ) = (ty, result)
                {
                    let own_ty = Type::Fn {
                        params: params.iter().map(|(_, t)| t.clone()).collect(),
                        result: Box::new(result.clone()),
                        uses: uses.clone(),
                        fails: fails.clone(),
                        label: label.clone(),
                    };
                    self.fn_flow(&own_ty, ty, &who, span, ctx);
                }
                if name.starts_with('%') {
                    self.lambdas.insert(name.to_owned(), who);
                }
            }
            Rhs::Checked {
                value,
                formula,
                ctor,
                args,
                arg_spans,
                ..
            } => {
                self.fields(ctor, args, arg_spans, ctx);
                let checked = self.atom(value, ctx);
                let env = Env {
                    vars: &|name| Some(own(ctx, name)),
                    it: Some(checked.clone()),
                    result: None,
                };
                if let Some(fact) = self.term(formula, &env) {
                    ctx.assume(fact.f);
                }
                if checked.f.sort() == x.f.sort() {
                    ctx.define(F::eq(x.f.clone(), checked.f));
                }
            }
            Rhs::Declassify {
                policy,
                arg,
                arg_span,
                ..
            } => {
                let key = (policy.module.as_str(), policy.name.as_str());
                if let Some((from, to)) = self.index.policies.get(&key) {
                    let value = self.atom(arg, ctx);
                    self.refinement_sites(from, &value, *arg_span, ctx);
                    let result = Tv {
                        f: x.f.clone(),
                        ty: (*to).clone(),
                    };
                    self.assume_refinements(ctx, &result);
                }
            }
        }
    }

    /// A call of a function or an operation: refinements of its arguments, its `requires`,
    /// then the refinements of its result and its `ensures` as facts about `x` (T-Call, T-Op).
    fn call(
        &mut self,
        contract: &Contract<'_>,
        values: &[Tv],
        arg_spans: &[Span],
        span: Span,
        x: &Tv,
        ctx: &mut Ctx,
    ) {
        for (((_, param_ty), value), arg_span) in contract.params.iter().zip(values).zip(arg_spans)
        {
            if self.eta.is_none() {
                self.refinement_sites(param_ty, value, *arg_span, ctx);
            }
        }
        let by_name: BTreeMap<&str, Tv> = contract
            .params
            .iter()
            .zip(values)
            .map(|((name, _), value)| (name.as_str(), value.clone()))
            .collect();
        // In the lambda that stands for a function value, every argument spans the value's
        // name, so the goal text names the lambda's parameters instead (R-032).
        let texts: BTreeMap<&str, String> = contract
            .params
            .iter()
            .zip(arg_spans)
            .enumerate()
            .map(|(index, ((name, _), span))| {
                let text = self
                    .eta
                    .as_ref()
                    .and_then(|names| names.get(index).cloned())
                    .unwrap_or_else(|| self.text(*span));
                (name.as_str(), text)
            })
            .collect();
        let vars = |name: &str| by_name.get(name).cloned();
        // Issue #106 (V12-003): the repair of a function value's precondition guards the
        // conjunction of every `requires`, so applying it settles them all, and uses a named
        // helper with a handling clause when the function can fail, since a failing call must
        // be a whole statement (U3013).
        let shown: Vec<String> = contract
            .requires
            .iter()
            .map(|clause| {
                print_term_with(&clause.term, &|term| match term {
                    Term::Var(name) => texts.get(name.as_str()).cloned(),
                    _ => None,
                })
            })
            .collect();
        let all = shown.join(" and ");
        // The label of the parameters the precondition reads, which a guard on it branches on.
        let read = std::cell::RefCell::new(BTreeSet::new());
        for clause in contract.requires {
            print_term_with(&clause.term, &|term| {
                if let Term::Var(name) = term {
                    read.borrow_mut().insert(name.clone());
                }
                None
            });
        }
        let read = read.into_inner();
        let guard_label = contract
            .params
            .iter()
            .filter(|(param, _)| read.contains(param.as_str()))
            .fold(una_core::Label::public(), |label, (_, ty)| {
                label.join(&type_label(ty).concrete())
            });
        let value_repair = self.eta.as_ref().zip(self.eta_name.clone()).map(|(params, name)| {
            let args = params.join(", ");
            let repair = match contract.fails {
                None => format!(
                    "pass a lambda that calls `{name}` only where `{all}` holds, as in `fn({args}) -> if {all} then {name}({args}) else ...`"
                ),
                // A guard on labeled data around a call that can fail reveals that data through
                // the failure (U4002), so no helper checks: the repair says so instead (#126).
                Some(_) if !guard_label.is_public() => format!(
                    "`{name}` can fail, and `{all}` reads `{guard_label}` data, so a guard around the call would reveal that data through its failure (U4002): pass a function whose type has no precondition and that cannot fail on these values, or make `{name}`'s precondition hold for every argument"
                ),
                Some(kind) => {
                    let kind = kind.rsplit('.').next().unwrap_or(kind);
                    let typed: Vec<String> = params
                        .iter()
                        .zip(contract.params)
                        .map(|(param, (_, ty))| format!("{param}: {}", written_type(ty)))
                        .collect();
                    // The types as the value's use instantiates them, labels and refinements
                    // included, so the helper checks once written (P-1, #126).
                    format!(
                        "pass a named function that calls `{name}` only where `{all}` holds: `{name}_where_defined({}) -> {} fails {kind}`, whose body is `if {all}` with `let v = {name}({args})`, `if error pass` and `v` in its block, and a value in its `else` block",
                        typed.join(", "),
                        written_type(contract.instantiated_result.unwrap_or(contract.result))
                    )
                }
            };
            (name, repair)
        });
        for clause in contract.requires {
            let env = Env {
                vars: &vars,
                it: None,
                result: None,
            };
            let goal = self.term(&clause.term, &env);
            let text = print_term_with(&clause.term, &|term| match term {
                Term::Var(name) => texts.get(name.as_str()).cloned(),
                _ => None,
            });
            // Issue #76 (UI-007): in the eta-expansion of a named function passed as a value,
            // the precondition cannot follow the value, since a function type has none; its
            // only repair is a lambda that calls the function where the precondition holds.
            let function_value = value_repair.clone();
            let related = clause.span.map(|span| (span, "requires clause"));
            match function_value {
                Some((name, repair)) => {
                    self.function_value_site(
                        ctx,
                        goal.map(|g| g.f),
                        text,
                        span,
                        related,
                        name,
                        repair,
                    );
                }
                None => self.site(
                    ObligationKind::Precondition,
                    ctx,
                    goal.map(|g| g.f),
                    text,
                    span,
                    related,
                ),
            }
        }
        let result = Tv {
            f: x.f.clone(),
            ty: contract.result.clone(),
        };
        self.assume_refinements(ctx, &result);
        for clause in contract.ensures {
            let env = Env {
                vars: &vars,
                it: None,
                result: Some(x.clone()),
            };
            if let Some(fact) = self.term(&clause.term, &env) {
                ctx.assume(fact.f);
            }
        }
    }

    /// The fields of a constructed value or a failure: refinement obligations at each.
    fn fields(&mut self, ctor: &Ctor, args: &[Atom], arg_spans: &[Span], ctx: &Ctx) {
        let variants = if self.index.prelude_kind(&ctor.kind.module, &ctor.kind.name) {
            None
        } else {
            self.index
                .kinds
                .get(&(ctor.kind.module.as_str(), ctor.kind.name.as_str()))
        };
        let Some(kind) = variants else {
            return;
        };
        let Some(variant) = kind.variants.get(ctor.index) else {
            return;
        };
        let params: Vec<(Name, Type)> = variant.fields.clone();
        for (((_, field_ty), atom), span) in params.iter().zip(args).zip(arg_spans) {
            let value = self.atom(atom, ctx);
            self.refinement_sites(field_ty, &value, *span, ctx);
        }
    }

    /// Arguments into declared parameters: refinement obligations.
    fn arguments(&mut self, params: &[(Name, Type)], args: &[Atom], arg_spans: &[Span], ctx: &Ctx) {
        for (((_, ty), atom), span) in params.iter().zip(args).zip(arg_spans) {
            let value = self.atom(atom, ctx);
            self.refinement_sites(ty, &value, *span, ctx);
        }
    }

    /// A value flowing into `target`: one `refinement` obligation per refinement, at the
    /// value (`spec/12 §3`), and for a function value, the obligations of its function type
    /// (ADR 0037 rule 2).
    fn refinement_sites(&mut self, target: &Type, value: &Tv, span: Span, ctx: &Ctx) {
        if contains_fn(target) {
            let who = self.who(value);
            self.fn_flow(&value.ty, target, &who, span, ctx);
        }
        let value_text = self.text(span);
        for refinement in refinements(target) {
            let env = Env {
                vars: &|_| None,
                it: Some(value.clone()),
                result: None,
            };
            let goal = self.term(&refinement.clause.term, &env);
            let text = instantiate(&refinement.clause.term, &value_text);
            self.site(
                ObligationKind::Refinement,
                ctx,
                goal.map(|g| g.f),
                text,
                span,
                refinement
                    .clause
                    .span
                    .map(|span| (span, clause_role(refinement))),
            );
        }
    }

    // ----- function types (ADR 0037) -----

    /// The values of the normal return paths `leaves` flowing into `target`, a declared result
    /// or a binder, when it holds a function type (ADR 0037 rule 2): at each returned
    /// expression, or at `at`, the expression a binder is bound to.
    fn returned_functions(
        &mut self,
        entry: &Ctx,
        leaves: &[Leaf],
        target: &Type,
        at: Option<Span>,
    ) {
        if !contains_fn(target) {
            return;
        }
        for leaf in leaves {
            let Some(value) = &leaf.value else {
                continue;
            };
            let mut ctx = entry.clone();
            ctx.hyps = leaf.hyps.clone();
            let who = self.who(value);
            self.fn_flow(&value.ty, target, &who, at.unwrap_or(leaf.span), &ctx);
        }
    }

    /// The fields of a constructed value of type `owner` that hold function types, at the
    /// kind's type arguments, the prelude's `Some`, `Ok` and `Err` included (ADR 0037 rule 2).
    /// The fields of a kind without type arguments are [`Walker::fields`]'s.
    fn field_functions(
        &mut self,
        owner: &Type,
        ctor: &Ctor,
        args: &[Atom],
        arg_spans: &[Span],
        ctx: &Ctx,
    ) {
        let generic = match owner {
            Type::Base {
                base: Base::Named { module, name, args },
                ..
            } => self.index.prelude_kind(module, name) || !args.is_empty(),
            Type::Base { .. } | Type::Fn { .. } => false,
        };
        if !generic {
            return;
        }
        for (index, (atom, span)) in args.iter().zip(arg_spans).enumerate() {
            let Some(field) = self.index.field_type(owner, ctor.index, index) else {
                continue;
            };
            if contains_fn(&field) {
                let value = self.atom(atom, ctx);
                let who = self.who(&value);
                self.fn_flow(&value.ty, &field, &who, *span, ctx);
            }
        }
    }

    /// How the obligations of a flowing value name it (ADR 0037 rules 3 and 4): a lambda or
    /// a named function by what it is, another value by its source name, with no names of its
    /// own for its parameters.
    fn who(&self, value: &Tv) -> Who {
        let name = match &value.f {
            F::Const(Const {
                name,
                role: ConstRole::Plain,
                ..
            }) => name.clone(),
            _ => String::new(),
        };
        if let Some(who) = self.lambdas.get(&name) {
            return who.clone();
        }
        Who {
            name: self.show_name(&name),
            params: Vec::new(),
        }
    }

    /// A fresh binder of a function-type flow, which shows as `name` (ADR 0037 rules 5, 9).
    fn fresh_binder(&mut self, name: &str) -> Name {
        self.fresh += 1;
        format!("{name}%f{}", self.fresh)
    }

    /// A value of type `source` flowing into `target` (ADR 0037 rule 2, S-Fun): for function
    /// types, one `refinement` obligation per refinement of a parameter of `source` that the
    /// same parameter of `target` does not carry (contravariant), and per refinement of the
    /// result of `target` that the result of `source` does not carry (covariant), at `span`,
    /// under the path context `ctx`. Function types nested in parameters, results and type
    /// arguments are compared the same way. The binders are fresh (rule 9).
    /// D-153: one obligation per refinement the target position lacks, as for
    /// any flow, and a refinement is carried when the same refined kind or range is.
    /// D-154: a position nested in a parameter or result is named after it
    /// (`parameter arg1 of parameter g of takes_fn`), and a value that is neither a named
    /// function nor a lambda by its source text.
    fn fn_flow(&mut self, source: &Type, target: &Type, who: &Who, span: Span, ctx: &Ctx) {
        match (source, target) {
            (
                Type::Fn {
                    params: from,
                    result: from_result,
                    ..
                },
                Type::Fn {
                    params: to,
                    result: to_result,
                    ..
                },
            ) if from.len() == to.len() => {
                let names: Vec<String> = (0..from.len())
                    .map(|index| {
                        who.params
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| format!("arg{}", index + 1))
                    })
                    .collect();
                let binders: Vec<Name> = names.iter().map(|n| self.fresh_binder(n)).collect();
                // Parameters: the hypotheses are the target's, the goals the source's.
                for (index, ((a, b), (name, binder))) in from
                    .iter()
                    .zip(to)
                    .zip(names.iter().zip(&binders))
                    .enumerate()
                {
                    let mut at = ctx.clone();
                    let value = self.flow_binder(&mut at, binder, b);
                    let position = format!("parameter {name} of {}", who.name);
                    for goal in refinements(a) {
                        if refinements(b).contains(goal) {
                            continue;
                        }
                        let shown = instantiate(&goal.clause.term, name);
                        let repair = format!(
                            "strengthen parameter {} of `{}` to a refined kind that implies `{shown}`",
                            index + 1,
                            show_type(target)
                        );
                        self.flow_obligation(
                            &at,
                            goal,
                            &value,
                            format!("{position}: {shown}"),
                            span,
                            repair,
                        );
                    }
                    let nested = Who {
                        name: position,
                        params: Vec::new(),
                    };
                    self.fn_flow(b, a, &nested, span, ctx);
                }
                // The result: the hypotheses are the target's parameters and the source's
                // result, the goals the target's result.
                let mut at = ctx.clone();
                for (binder, b) in binders.iter().zip(to) {
                    self.flow_binder(&mut at, binder, b);
                }
                let result_binder = self.fresh_binder("result");
                let value = self.flow_binder(&mut at, &result_binder, from_result);
                let position = format!("result of {}", who.name);
                for goal in refinements(to_result) {
                    if refinements(from_result).contains(goal) {
                        continue;
                    }
                    let shown = instantiate(&goal.clause.term, "result");
                    // D-151: rule 6 names the expected type's position for a
                    // parameter; for the result, the position to strengthen is the value's.
                    let repair = format!(
                        "strengthen the result of `{}` to a refined kind that implies `{shown}`",
                        show_type(source)
                    );
                    self.flow_obligation(
                        &at,
                        goal,
                        &value,
                        format!("{position}: {shown}"),
                        span,
                        repair,
                    );
                }
                let nested = Who {
                    name: position,
                    params: Vec::new(),
                };
                self.fn_flow(from_result, to_result, &nested, span, ctx);
            }
            (
                Type::Base {
                    base:
                        Base::Named {
                            module,
                            name,
                            args: from,
                        },
                    ..
                },
                Type::Base {
                    base:
                        Base::Named {
                            module: to_module,
                            name: to_name,
                            args: to,
                        },
                    ..
                },
            ) if module == to_module && name == to_name && from.len() == to.len() => {
                for (a, b) in from.iter().zip(to) {
                    self.fn_flow(a, b, who, span, ctx);
                }
            }
            _ => {}
        }
    }

    /// Brings the fresh binder `binder` of type `ty` into `ctx`, with its refinements as
    /// hypotheses, and returns its value.
    fn flow_binder(&mut self, ctx: &mut Ctx, binder: &str, ty: &Type) -> Tv {
        std::rc::Rc::make_mut(&mut ctx.types).insert(binder.to_owned(), ty.clone());
        let value = own(ctx, binder);
        self.assume_refinements(ctx, &value);
        value
    }

    /// One obligation of a function-type flow: `goal` holds of `value` under `ctx`, with the
    /// repair of ADR 0037 rule 6 only.
    fn flow_obligation(
        &mut self,
        ctx: &Ctx,
        goal: &Refinement,
        value: &Tv,
        text: String,
        span: Span,
        repair: String,
    ) {
        let env = Env {
            vars: &|_| None,
            it: Some(value.clone()),
            result: None,
        };
        let formula = self.term(&goal.clause.term, &env).map(|g| g.f);
        let mut assertions: Vec<F> = ctx.hyps.iter().map(|h| h.f.clone()).collect();
        let mut place = self.place(ObligationKind::Refinement, formula.as_ref());
        place.parameter = None;
        place.over_parameters = false;
        place.ground = false;
        place.flow = Some(repair);
        assertions.push(F::not(formula.unwrap_or(F::Bool(false))));
        let hypotheses = self.shown_hypotheses(&ctx.hyps);
        let related = goal.clause.span.map(|span| (span, clause_role(goal)));
        self.push(
            ObligationKind::Refinement,
            text,
            span,
            related,
            Vec::new(),
            hypotheses,
            assertions,
            ctx,
            place,
            vec![ctx.divisions.to_vec()],
        );
    }

    /// The paths of a lambda's body flowing into the result its binder expects (`spec/12 §3`,
    /// returns): an obligation over every path per refinement, named `result of the lambda`
    /// (ADR 0037 rule 4), and the flows of the function values it returns.
    fn lambda_result(
        &mut self,
        entry: &Ctx,
        leaves: &[Leaf],
        expected: &Type,
        who: &Who,
        span: Span,
    ) {
        self.returned_functions(entry, leaves, expected, None);
        let result = Tv {
            f: constant("result", expected),
            ty: expected.clone(),
        };
        for refinement in refinements(expected) {
            let goal = {
                let env = Env {
                    vars: &|_| None,
                    it: Some(result.clone()),
                    result: Some(result.clone()),
                };
                self.term(&refinement.clause.term, &env)
            };
            let shown = instantiate(&refinement.clause.term, "result");
            let before = self.obligations.len();
            self.returns(
                ObligationKind::Refinement,
                entry,
                leaves,
                &result,
                goal,
                format!("result of {}: {shown}", who.name),
                span,
                refinement
                    .clause
                    .span
                    .map(|span| (span, clause_role(refinement))),
            );
            // D-152: the repair of a lambda's result, which has no written type,
            // is a guard in its body.
            for obligation in self.obligations.iter_mut().skip(before) {
                obligation.site.parameter = None;
                obligation.site.over_parameters = false;
                obligation.site.ground = false;
                obligation.site.flow = Some(format!(
                    "add a guard or an arm so that the failing path of the lambda returns a value for which `{shown}` holds"
                ));
            }
        }
    }

    // ----- values -----

    fn atom(&mut self, atom: &Atom, ctx: &Ctx) -> Tv {
        match atom {
            Atom::Var(name) => own(ctx, name),
            Atom::Int(digits) => Tv {
                f: F::Int(digits.clone()),
                ty: base_type(Base::Int),
            },
            Atom::Bool(value) => Tv {
                f: F::Bool(*value),
                ty: base_type(Base::Bool),
            },
            Atom::Text(text) => text_literal(text),
            Atom::Unit => Tv {
                f: F::Const(Const {
                    name: "unit".to_owned(),
                    sort: UNIT.to_owned(),
                    role: ConstRole::Internal,
                }),
                ty: base_type(Base::Unit),
            },
        }
    }

    /// The tag of a value of a kind, `i!tag!<Sort>`, when its kind is a sum.
    fn tag(&self, value: &Tv) -> Option<F> {
        let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &value.ty
        else {
            return None;
        };
        let names = self.index.variant_names(module, name);
        let record = self
            .index
            .kinds
            .get(&(module.as_str(), name.as_str()))
            .is_some_and(|kind| kind.record);
        if names.is_empty() || record {
            return None;
        }
        Some(tag_of(&value.f, module, name, names.len()))
    }

    /// Field `index` of `variant` of a value: `u!f!<field>!<Sort>` for a record,
    /// `u!f!<field>!<Variant>!<Sort>` for a variant of a sum, so that variants sharing a field
    /// name with different types have distinct functions, and `i!f!<field>!<Sort>` for the
    /// fields of the prelude's kinds (ADR 0038 D8).
    fn field(&self, value: &Tv, variant: usize, index: usize) -> Option<Tv> {
        let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &value.ty
        else {
            return None;
        };
        let field_ty = self.index.field_type(&value.ty, variant, index)?;
        let sort = value.f.sort();
        let symbol = if self.index.prelude_kind(module, name) {
            let field = prelude_variants(name)
                .and_then(|variants| variants.get(variant))
                .and_then(|(_, fields)| fields.get(index))?;
            format!("i!f!{field}!{sort}")
        } else {
            let kind = self.index.kinds.get(&(module.as_str(), name.as_str()))?;
            let declared = kind.variants.get(variant)?;
            let (field, _) = declared.fields.get(index)?;
            if kind.record {
                format!("u!f!{field}!{sort}")
            } else {
                format!("u!f!{field}!{}!{sort}", declared.name)
            }
        };
        let variants = self.index.variant_names(module, name).len();
        Some(Tv {
            f: F::App {
                fun: SmtFun {
                    name: symbol,
                    args: vec![sort],
                    result: self::sort(&field_ty),
                    role: FunRole::Field {
                        module: module.clone(),
                        kind: name.clone(),
                        variant,
                        index,
                        variants,
                    },
                },
                args: vec![value.f.clone()],
            },
            ty: field_ty,
        })
    }

    /// `find` and `contains_text` share an uninterpreted `i!contains!Text` (N-19): one is `Some`
    /// exactly when the other holds (`spec/22 §2` rule 2). Both engines run them over one
    /// search, so the symbol stands for the same relation. A position `find` returns lies in
    /// the text with its match after it (N-21).
    fn text_facts(&mut self, name: &str, values: &[Tv], x: &Tv, ctx: &mut Ctx) {
        let (Some(t), Some(part)) = (values.first(), values.get(1)) else {
            return;
        };
        if t.f.sort() != TEXT || part.f.sort() != TEXT {
            return;
        }
        let contains = F::App {
            fun: SmtFun {
                name: "i!contains!i!Text".to_owned(),
                args: vec![TEXT.to_owned(), TEXT.to_owned()],
                result: "Bool".to_owned(),
                role: FunRole::Other,
            },
            args: vec![t.f.clone(), part.f.clone()],
        };
        match name {
            "contains_text" if x.f.sort() == "Bool" => ctx.define(F::eq(x.f.clone(), contains)),
            "find" => {
                let Some(tag) = self.tag(x) else {
                    return;
                };
                let some = F::eq(tag, F::Int("0".to_owned()));
                ctx.assume(F::eq(some.clone(), contains));
                // N-21: the position of a match lies in the text, with the match after it.
                if let (Some(at), Some(t_len), Some(part_len)) =
                    (self.field(x, 0, 0), self.len(t), self.len(part))
                    && at.f.sort() == "Int"
                {
                    ctx.assume(F::op(
                        "=>",
                        vec![
                            some,
                            F::and(vec![
                                F::op("<=", vec![F::Int("0".to_owned()), at.f.clone()]),
                                F::op("<=", vec![F::op("+", vec![at.f, part_len]), t_len]),
                            ]),
                        ],
                    ));
                }
            }
            _ => {}
        }
    }

    /// `.len` of a text, list or map, `i!len!<Sort>`.
    fn len(&self, value: &Tv) -> Option<F> {
        let sort = value.f.sort();
        let has_len = sort == TEXT || sort.starts_with(LIST) || sort.starts_with(MAP);
        has_len.then(|| len_of(&value.f))
    }

    /// The contract of a map's operation as facts of its result `x` (`spec/21 §4`): `has` and
    /// `get` are uninterpreted, and only the key the operation names is described.
    fn map_facts(&mut self, op: MapOp, values: &[Tv], x: &Tv, ctx: &mut Ctx) {
        let Some(map) = values.first() else {
            return;
        };
        let Some(old_len) = self.len(map) else {
            return;
        };
        let key = values.get(1);
        match op {
            MapOp::Has => {
                if let Some(key) = key {
                    ctx.define(F::eq(x.f.clone(), has_of(&map.f, &key.f)));
                }
            }
            MapOp::Get => {
                let Some(key) = key else {
                    return;
                };
                ctx.define(F::eq(x.f.clone(), get_of(&map.f, &key.f, &x.f.sort())));
                // `result` is `Some` exactly when `m.has(key: k)`; `Some` is variant 0.
                if let Some(tag) = self.tag(x) {
                    ctx.define(F::eq(
                        F::eq(tag, F::Int("0".to_owned())),
                        has_of(&map.f, &key.f),
                    ));
                }
            }
            MapOp::With | MapOp::Without => {
                let (Some(key), Some(new_len)) = (key, self.len(x)) else {
                    return;
                };
                let had = has_of(&map.f, &key.f);
                let one = F::Int("1".to_owned());
                let (with_key, without_key) = if op == MapOp::With {
                    (old_len.clone(), F::op("+", vec![old_len.clone(), one]))
                } else {
                    (F::op("-", vec![old_len.clone(), one]), old_len.clone())
                };
                ctx.assume(F::eq(
                    new_len,
                    F::op("ite", vec![had, with_key, without_key]),
                ));
                let has_now = has_of(&x.f, &key.f);
                if op == MapOp::Without {
                    ctx.assume(F::not(has_now));
                    return;
                }
                ctx.assume(has_now);
                // `m.get(key: k) == Some(v)` after `set` and `with`.
                let (Some(value), Some(option)) = (values.get(2), option_of(&x.ty)) else {
                    return;
                };
                let got = Tv {
                    f: get_of(&x.f, &key.f, &sort(&option)),
                    ty: option,
                };
                if let Some(tag) = self.tag(&got) {
                    ctx.assume(F::eq(tag, F::Int("0".to_owned())));
                }
                if let Some(payload) = self.field(&got, 0, 0)
                    && payload.f.sort() == value.f.sort()
                {
                    ctx.assume(F::eq(payload.f, value.f.clone()));
                }
            }
            MapOp::Keys | MapOp::Values | MapOp::Entries => {
                if let Some(new_len) = self.len(x) {
                    ctx.assume(F::eq(new_len, old_len));
                }
            }
        }
    }

    /// How many tasks a `together for` over `source` runs: the length of its list, or the
    /// number of integers of its range, zero when it is empty.
    fn task_count(&mut self, source: &LoopSource, ctx: &Ctx) -> Option<F> {
        match source {
            LoopSource::List(list) => {
                let list = self.atom(list, ctx);
                self.len(&list)
            }
            LoopSource::Range {
                low,
                high,
                inclusive,
            } => {
                let low = self.atom(low, ctx).f;
                let high = self.atom(high, ctx).f;
                if low.sort() != "Int" || high.sort() != "Int" {
                    return None;
                }
                let (nonempty, count) = if *inclusive {
                    (
                        F::op("<=", vec![low.clone(), high.clone()]),
                        F::op(
                            "+",
                            vec![F::op("-", vec![high, low]), F::Int("1".to_owned())],
                        ),
                    )
                } else {
                    (
                        F::op("<", vec![low.clone(), high.clone()]),
                        F::op("-", vec![high, low]),
                    )
                };
                Some(F::op("ite", vec![nonempty, count, F::Int("0".to_owned())]))
            }
        }
    }

    /// The obligations of a primitive of fixed-width integers (`spec/18 §3`, `§5`): an
    /// `overflow` obligation for arithmetic whose result has a fixed-width type and for a
    /// narrowing conversion, and the precondition `0 <= n and n < width` of a shift. A negated
    /// literal is a literal, which the checker bounds (U2022).
    #[allow(clippy::too_many_arguments)]
    fn fixed_sites(
        &mut self,
        ty: &Type,
        prim: Prim,
        args: &[Atom],
        values: &[Tv],
        arg_spans: &[Span],
        span: Span,
        ctx: &Ctx,
    ) {
        let arg = |i: usize| values.get(i).map(|v| v.f.clone());
        if let Prim::Fixed(op, int) = prim {
            if op.shift()
                && let Some(n) = arg(1)
            {
                let amount = arg_spans.get(1).map_or_else(String::new, |s| self.text(*s));
                let goal = F::and(vec![
                    F::op("<=", vec![F::Int("0".to_owned()), n.clone()]),
                    F::op("<", vec![n, F::Int(int.bits().to_string())]),
                ]);
                let text = format!("0 <= {amount} and {amount} < {}", int.bits());
                self.site(
                    ObligationKind::Precondition,
                    ctx,
                    Some(goal),
                    text,
                    span,
                    None,
                );
            }
            return;
        }
        let Type::Base {
            base: Base::Fixed(int),
            ..
        } = ty
        else {
            return;
        };
        let exact = match prim {
            Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Mod => self.prim(prim, values),
            Prim::Neg if !matches!(args.first(), Some(Atom::Int(_))) => self.prim(prim, values),
            Prim::Convert(Some(_)) => {
                let source = values.first().map(|v| &v.ty);
                let widening = matches!(
                    source,
                    Some(Type::Base { base: Base::Fixed(from), .. }) if from.widens_to(*int)
                );
                if widening { None } else { arg(0) }
            }
            _ => None,
        };
        let Some(exact) = exact else {
            return;
        };
        let goal = in_range(exact, *int);
        let range = format!("{}..{}", int.least(), int.greatest());
        let text = format!("{} in {range}", self.text(span));
        // The operands as `Int`: a literal as written, any other value converted.
        let operand = |i: usize| -> String {
            let written = arg_spans.get(i).map_or_else(String::new, |s| self.text(*s));
            let literal = written
                .strip_prefix('-')
                .unwrap_or(&written)
                .chars()
                .all(|c| c.is_ascii_digit());
            if matches!(args.get(i), Some(Atom::Int(_))) || (literal && !written.is_empty()) {
                written
            } else {
                format!("{written}.to_int()")
            }
        };
        let guard = match prim {
            Prim::Convert(_) => {
                let written = arg_spans
                    .first()
                    .map_or_else(String::new, |s| self.text(*s));
                Some(format!("{written} in {range}"))
            }
            Prim::Neg => Some(format!("-{} in {range}", operand(0))),
            Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Mod => {
                let symbol = match prim {
                    Prim::Add => "+",
                    Prim::Sub => "-",
                    Prim::Mul => "*",
                    Prim::Div => "/",
                    _ => "mod",
                };
                Some(format!("{} {symbol} {} in {range}", operand(0), operand(1)))
            }
            _ => None,
        };
        let mut assertions: Vec<F> = ctx.hyps.iter().map(|h| h.f.clone()).collect();
        let mut place = self.place(ObligationKind::Overflow, Some(&goal));
        place.guard = guard;
        assertions.push(F::not(goal));
        let hypotheses = self.shown_hypotheses(&ctx.hyps);
        self.push(
            ObligationKind::Overflow,
            text,
            span,
            None,
            Vec::new(),
            hypotheses,
            assertions,
            ctx,
            place,
            vec![ctx.divisions.to_vec()],
        );
    }

    /// The contract of a method of a fixed-width type over its result `r` (`spec/18 §5`): the
    /// exact result of the wrapping and saturating methods, and of `wrapping_shift_left` by a
    /// literal; the partial contracts of the bit methods and `shift_right`; and, for every
    /// one, the type's range.
    fn fixed_facts(
        &mut self,
        op: FixedOp,
        int: IntType,
        args: &[Atom],
        values: &[Tv],
        r: &F,
        ctx: &mut Ctx,
    ) {
        let (Some(a), b) = (
            values.first().map(|v| v.f.clone()),
            values.get(1).map(|v| v.f.clone()),
        ) else {
            return;
        };
        ctx.define(in_range(r.clone(), int));
        let unsigned = !int.signed();
        let mut facts = Vec::new();
        match (op, b) {
            (FixedOp::WrappingAdd, Some(b)) => {
                facts.push(F::eq(r.clone(), wrapped(F::op("+", vec![a, b]), int)));
            }
            (FixedOp::WrappingSub, Some(b)) => {
                facts.push(F::eq(r.clone(), wrapped(F::op("-", vec![a, b]), int)));
            }
            (FixedOp::WrappingMul, Some(b)) => {
                facts.push(F::eq(r.clone(), wrapped(arithmetic(BinOp::Mul, a, b), int)));
            }
            (FixedOp::SaturatingAdd, Some(b)) => {
                facts.push(F::eq(r.clone(), clamped(F::op("+", vec![a, b]), int)));
            }
            (FixedOp::SaturatingSub, Some(b)) => {
                facts.push(F::eq(r.clone(), clamped(F::op("-", vec![a, b]), int)));
            }
            (FixedOp::SaturatingMul, Some(b)) => {
                facts.push(F::eq(r.clone(), clamped(arithmetic(BinOp::Mul, a, b), int)));
            }
            (FixedOp::BitAnd, Some(b)) if unsigned => {
                facts.push(F::op("<=", vec![r.clone(), a]));
                facts.push(F::op("<=", vec![r.clone(), b]));
            }
            (FixedOp::BitOr, Some(b)) if unsigned => {
                facts.push(F::op(">=", vec![r.clone(), a]));
                facts.push(F::op(">=", vec![r.clone(), b]));
            }
            (FixedOp::ShiftRight, Some(_)) if unsigned => {
                facts.push(F::op("<=", vec![r.clone(), a]));
            }
            (FixedOp::WrappingShiftLeft, Some(_)) => {
                // Exact only by a literal amount, where `2^n` is a numeral and the product
                // is linear.
                if let Some(Atom::Int(digits)) = args.get(1)
                    && let Ok(n) = digits.parse::<u32>()
                    && n < int.bits()
                {
                    let factor = F::Int((1u128 << n).to_string());
                    facts.push(F::eq(r.clone(), wrapped(F::op("*", vec![a, factor]), int)));
                }
            }
            _ => {}
        }
        for fact in facts {
            ctx.define(fact);
        }
    }

    /// `f`, or the literal that defines it when it is a temporary bound to one, as the Core
    /// IR binds `-3` before `e mod -3`, so that the operation stays linear (#188).
    fn literal(&self, f: F) -> F {
        if let F::Const(constant) = &f
            && let Some(def) = self.defs.get(&constant.name)
            && int_literal(def)
        {
            return def.clone();
        }
        f
    }

    /// A primitive of code as a term, when it is one of the refinement language (`spec/10 §3`:
    /// "RL-expressible results are selfified").
    fn prim(&self, prim: Prim, values: &[Tv]) -> Option<F> {
        let arg = |i: usize| values.get(i).cloned();
        Some(match prim {
            Prim::Add | Prim::Sub => {
                let (a, b) = (arg(0)?.f, arg(1)?.f);
                F::op(if prim == Prim::Add { "+" } else { "-" }, vec![a, b])
            }
            Prim::Mul => arithmetic(BinOp::Mul, self.literal(arg(0)?.f), self.literal(arg(1)?.f)),
            Prim::Div => arithmetic(BinOp::Div, self.literal(arg(0)?.f), self.literal(arg(1)?.f)),
            Prim::Mod => arithmetic(BinOp::Mod, self.literal(arg(0)?.f), self.literal(arg(1)?.f)),
            Prim::Neg => F::op("-", vec![arg(0)?.f]),
            Prim::Not => F::not(arg(0)?.f),
            Prim::Eq | Prim::Ne => {
                let (a, b) = (arg(0)?, arg(1)?);
                let eq = self.equal(&a, &b)?;
                if prim == Prim::Eq { eq } else { F::not(eq) }
            }
            Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge => {
                let (a, b) = (arg(0)?, arg(1)?);
                let op = match prim {
                    Prim::Lt => BinOp::Lt,
                    Prim::Le => BinOp::Le,
                    Prim::Gt => BinOp::Gt,
                    _ => BinOp::Ge,
                };
                self.order(op, &a, &b)?
            }
            Prim::Len => self.len(&arg(0)?)?,
            // `to_int()` on a type parameter under `Integer` is uninterpreted (`spec/20 §5`).
            Prim::Convert(None) if is_param_sort(&arg(0)?.f.sort()) => {
                let value = arg(0)?.f;
                let sort = value.sort();
                F::App {
                    fun: SmtFun {
                        name: format!("i!to_int!{sort}"),
                        args: vec![sort],
                        result: "Int".to_owned(),
                        role: FunRole::Other,
                    },
                    args: vec![value],
                }
            }
            // A conversion keeps the value; a narrowing one has an `overflow` obligation
            // (`spec/18 §4`).
            Prim::Convert(_) => arg(0)?.f,
            // The methods of `spec/18 §5` are known by their contracts, not as terms.
            Prim::Concat | Prim::ToText | Prim::Fixed(..) | Prim::Map(_) => return None,
        })
    }

    /// Equality of two values: of tags for a sum without payloads, of the values otherwise.
    fn equal(&self, a: &Tv, b: &Tv) -> Option<F> {
        if a.f.sort() != b.f.sort() {
            return None;
        }
        if let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &a.ty
            && self.index.payload_free(module, name)
        {
            return Some(F::eq(self.tag(a)?, self.tag(b)?));
        }
        Some(F::eq(a.f.clone(), b.f.clone()))
    }

    /// Order of two values: `Int` natively, a sum without payloads by its tags, `Text` and a
    /// type parameter under `Ord` by uninterpreted relations, with no axioms (`spec/20 §5`).
    fn order(&self, op: BinOp, a: &Tv, b: &Tv) -> Option<F> {
        let symbol = match op {
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            _ => ">=",
        };
        let sort = a.f.sort();
        if sort == "Int" {
            return Some(F::op(symbol, vec![a.f.clone(), b.f.clone()]));
        }
        if sort == TEXT || is_param_sort(&sort) {
            let relation = match op {
                BinOp::Lt => "lt",
                BinOp::Le => "le",
                BinOp::Gt => "gt",
                _ => "ge",
            };
            // `Text` keeps its symbols of `spec/12 §4`, `i!lt!Text` and the others.
            let name = if sort == TEXT {
                format!("i!{relation}!Text")
            } else {
                format!("i!{relation}!{sort}")
            };
            return Some(F::App {
                fun: SmtFun {
                    name,
                    args: vec![sort.clone(), sort.clone()],
                    result: "Bool".to_owned(),
                    role: FunRole::Other,
                },
                args: vec![a.f.clone(), b.f.clone()],
            });
        }
        Some(F::op(symbol, vec![self.tag(a)?, self.tag(b)?]))
    }

    /// A call of a predicate as a term; a reflect predicate's definition is kept for the
    /// queries that use it.
    fn predicate(
        &mut self,
        path: &una_core::Path,
        params: &[(Name, Type)],
        formula: Option<&Term>,
        args: &[Tv],
    ) -> Option<F> {
        let name = format!("u!p!{}", path.qualified());
        let sorts: Vec<Sort> = args.iter().map(|a| a.f.sort()).collect();
        if let Some(formula) = formula
            && !self.definitions.contains_key(&name)
        {
            let locals: Vec<Tv> = params
                .iter()
                .zip(&sorts)
                .map(|((param, ty), sort)| Tv {
                    f: F::Local {
                        name: param.clone(),
                        sort: sort.clone(),
                    },
                    ty: ty.clone(),
                })
                .collect();
            let by_name: BTreeMap<&str, Tv> = params
                .iter()
                .zip(&locals)
                .map(|((param, _), local)| (param.as_str(), local.clone()))
                .collect();
            let vars = |n: &str| by_name.get(n).cloned();
            let env = Env {
                vars: &vars,
                it: None,
                result: None,
            };
            let body = self.term(formula, &env)?;
            self.definitions.insert(
                name.clone(),
                Definition {
                    params: params
                        .iter()
                        .zip(&sorts)
                        .map(|((param, _), sort)| (param.clone(), sort.clone()))
                        .collect(),
                    body: body.f,
                },
            );
        }
        Some(F::App {
            fun: SmtFun {
                name,
                args: sorts,
                result: "Bool".to_owned(),
                role: FunRole::Predicate,
            },
            args: args.iter().map(|a| a.f.clone()).collect(),
        })
    }

    /// A term of the refinement language as a formula; `None` when it uses a value the
    /// encoding does not have.
    fn term(&mut self, term: &Term, env: &Env<'_>) -> Option<Tv> {
        let int = || base_type(Base::Int);
        let boolean = || base_type(Base::Bool);
        Some(match term {
            Term::Var(name) => (env.vars)(name)?,
            Term::It => env.it.clone()?,
            Term::Result => env.result.clone()?,
            Term::Proj { value, index } => {
                let tuple = self.term(value, env)?;
                projection(&tuple, *index)?
            }
            Term::Int(digits) => Tv {
                f: F::Int(digits.clone()),
                ty: int(),
            },
            Term::Bool(value) => Tv {
                f: F::Bool(*value),
                ty: boolean(),
            },
            Term::Text(text) => text_literal(text),
            Term::Unit => return None,
            Term::Ctor(_) => return None,
            Term::Field { value, index, .. } => {
                let value = self.term(value, env)?;
                self.field(&value, 0, *index)?
            }
            Term::Len(value) => {
                let value = self.term(value, env)?;
                Tv {
                    f: self.len(&value)?,
                    ty: int(),
                }
            }
            Term::Has { map, key } => {
                let map = self.term(map, env)?;
                let key = self.term(key, env)?;
                if !map.f.sort().starts_with(MAP) {
                    return None;
                }
                Tv {
                    f: has_of(&map.f, &key.f),
                    ty: boolean(),
                }
            }
            Term::Neg(value) => Tv {
                f: F::op("-", vec![self.term(value, env)?.f]),
                ty: int(),
            },
            Term::Not(value) => Tv {
                f: F::not(self.term(value, env)?.f),
                ty: boolean(),
            },
            Term::Is { value, ctor } => {
                let value = self.term(value, env)?;
                Tv {
                    f: F::eq(self.tag(&value)?, F::Int(ctor.index.to_string())),
                    ty: boolean(),
                }
            }
            Term::In {
                value,
                low,
                high,
                exclusive,
            } => {
                let value = self.term(value, env)?.f;
                let upper = if *exclusive { "<" } else { "<=" };
                Tv {
                    f: F::and(vec![
                        F::op("<=", vec![F::Int(low.clone()), value.clone()]),
                        F::op(upper, vec![value, F::Int(high.clone())]),
                    ]),
                    ty: boolean(),
                }
            }
            Term::Call { predicate, args } => {
                let values: Vec<Tv> = args
                    .iter()
                    .map(|a| self.term(a, env))
                    .collect::<Option<_>>()?;
                let key = (predicate.module.as_str(), predicate.name.as_str());
                let (params, formula) = self.index.predicates.get(&key).copied()?;
                Tv {
                    f: self.predicate(predicate, params, formula, &values)?,
                    ty: boolean(),
                }
            }
            Term::Binary { op, left, right } => self.binary(*op, left, right, env)?,
        })
    }

    fn binary(&mut self, op: BinOp, left: &Term, right: &Term, env: &Env<'_>) -> Option<Tv> {
        let boolean = base_type(Base::Bool);
        // A variant without fields compares by its tag.
        if let (Term::Ctor(ctor), other) | (other, Term::Ctor(ctor)) = (left, right)
            && matches!(op, BinOp::Eq | BinOp::Ne)
        {
            let value = self.term(other, env)?;
            let eq = F::eq(self.tag(&value)?, F::Int(ctor.index.to_string()));
            let f = if op == BinOp::Eq { eq } else { F::not(eq) };
            return Some(Tv { f, ty: boolean });
        }
        if let (Term::Ctor(ctor), _) | (_, Term::Ctor(ctor)) = (left, right)
            && matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
        {
            let k = F::Int(ctor.index.to_string());
            let ctor_on_left = matches!(left, Term::Ctor(_));
            let other = if ctor_on_left { right } else { left };
            let value = self.term(other, env)?;
            let tag = self.tag(&value)?;
            let (l, r) = if ctor_on_left { (k, tag) } else { (tag, k) };
            let symbol = match op {
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                _ => ">=",
            };
            return Some(Tv {
                f: F::op(symbol, vec![l, r]),
                ty: boolean,
            });
        }
        let l = self.term(left, env)?;
        let r = self.term(right, env)?;
        Some(match op {
            BinOp::And | BinOp::Or | BinOp::Implies => Tv {
                f: F::op(
                    match op {
                        BinOp::And => "and",
                        BinOp::Or => "or",
                        _ => "=>",
                    },
                    vec![l.f, r.f],
                ),
                ty: boolean,
            },
            BinOp::Eq => Tv {
                f: self.equal(&l, &r)?,
                ty: boolean,
            },
            BinOp::Ne => Tv {
                f: F::not(self.equal(&l, &r)?),
                ty: boolean,
            },
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => Tv {
                f: self.order(op, &l, &r)?,
                ty: boolean,
            },
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => Tv {
                f: arithmetic(op, l.f, r.f),
                ty: base_type(Base::Int),
            },
        })
    }

    // ----- obligations -----

    /// An obligation at one site: `Π ⊢ goal`.
    fn site(
        &mut self,
        kind: ObligationKind,
        ctx: &Ctx,
        goal: Option<F>,
        text: String,
        span: Span,
        related: Option<(Span, &'static str)>,
    ) {
        let mut assertions: Vec<F> = ctx.hyps.iter().map(|h| h.f.clone()).collect();
        let place = self.place(kind, goal.as_ref());
        // A goal outside the encoding is not assumed proven: its query has no negated goal
        // and stays satisfiable, so the obligation is refuted, never silently proven.
        assertions.push(F::not(goal.unwrap_or(F::Bool(false))));
        let hypotheses = self.shown_hypotheses(&ctx.hyps);
        self.push(
            kind,
            text,
            span,
            related,
            Vec::new(),
            hypotheses,
            assertions,
            ctx,
            place,
            vec![ctx.divisions.to_vec()],
        );
    }

    /// [`Walker::site`] for the precondition of the named function `name` passed as a function
    /// value, whose only repair is `repair` (issue #76).
    #[allow(clippy::too_many_arguments)]
    fn function_value_site(
        &mut self,
        ctx: &Ctx,
        goal: Option<F>,
        text: String,
        span: Span,
        related: Option<(Span, &'static str)>,
        name: String,
        repair: String,
    ) {
        let mut assertions: Vec<F> = ctx.hyps.iter().map(|h| h.f.clone()).collect();
        let mut place = self.place(ObligationKind::Precondition, goal.as_ref());
        place.parameter = None;
        place.over_parameters = false;
        place.flow = Some(repair);
        place.function_value = Some(name);
        assertions.push(F::not(goal.unwrap_or(F::Bool(false))));
        let hypotheses = self.shown_hypotheses(&ctx.hyps);
        self.push(
            ObligationKind::Precondition,
            text,
            span,
            related,
            Vec::new(),
            hypotheses,
            assertions,
            ctx,
            place,
            vec![ctx.divisions.to_vec()],
        );
    }

    /// An obligation over every normal return path: `Π ∧ (path₁ ∨ … ∨ pathₙ) ⊢ goal`, where
    /// each path fixes `result`.
    #[allow(clippy::too_many_arguments)]
    fn returns(
        &mut self,
        kind: ObligationKind,
        entry: &Ctx,
        leaves: &[Leaf],
        result: &Tv,
        goal: Option<Tv>,
        text: String,
        source: Span,
        related: Option<(Span, &'static str)>,
    ) {
        if leaves.is_empty() {
            return;
        }
        let base = entry.hyps.len();
        let mut assertions: Vec<F> = entry.hyps.iter().map(|h| h.f.clone()).collect();
        let mut selectors = Vec::new();
        for (number, leaf) in leaves.iter().enumerate() {
            let selector = path_selector(number + 1);
            assertions.push(F::eq(selector.clone(), self.path(leaf, base, result)));
            selectors.push(selector);
        }
        assertions.push(F::or(selectors));
        let place = self.place(kind, goal.as_ref().map(|g| &g.f));
        assertions.push(F::not(goal.map_or(F::Bool(false), |g| g.f)));
        let hypotheses = self.shown_hypotheses(&entry.hyps);
        let paths = leaves.iter().map(|leaf| leaf.span).collect();
        let mut ctx = entry.clone();
        for leaf in leaves {
            for hyp in leaf.hyps.iter().skip(base) {
                ctx.hyps.push(hyp.clone());
            }
        }
        std::rc::Rc::make_mut(&mut ctx.types).insert("result".to_owned(), result.ty.clone());
        // ADR 0071 D3: a returned value that a loop made fresh, which fixes `result`.
        (self.returned_forgotten, self.returned_unpinned) = self.forgotten(
            leaves
                .iter()
                .filter_map(|leaf| leaf.value.as_ref().map(|v| &v.f)),
        );
        let divisions = leaves.iter().map(|leaf| leaf.divisions.to_vec()).collect();
        self.push(
            kind, text, source, related, paths, hypotheses, assertions, &ctx, place, divisions,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: ObligationKind,
        goal: String,
        source: Span,
        related: Option<(Span, &'static str)>,
        paths: Vec<Span>,
        hypotheses: Vec<String>,
        assertions: Vec<F>,
        ctx: &Ctx,
        site: Site,
        divisions: Vec<Vec<(F, Span)>>,
    ) {
        let mut query = Query {
            assertions,
            definitions: BTreeMap::new(),
        };
        let (facts, listed) = self.ground_facts(&query);
        if !facts.is_empty() {
            let mut all = facts;
            all.append(&mut query.assertions);
            query.assertions = all;
        }
        // The refinements of record fields are listed first: `spec/13 §2` shows them.
        let mut hypotheses = hypotheses;
        let mut shown_facts: Vec<String> = Vec::new();
        for fact in &listed {
            for part in conjuncts(fact) {
                let text = self.show_formula(part);
                if !shown_facts.contains(&text) && !hypotheses.contains(&text) {
                    shown_facts.push(text);
                }
            }
        }
        shown_facts.append(&mut hypotheses);
        let hypotheses = shown_facts;
        self.add_definitions(&mut query);
        let shown = self.shown(&query, ctx);
        let mut forgotten = std::mem::take(&mut self.returned_forgotten);
        let returned_unpinned = std::mem::take(&mut self.returned_unpinned);
        let (shown_forgotten, unpinned) = self.forgotten(shown.iter().map(|item| &item.term));
        // A value from a loop's final value is not fixed by the first iteration, so a model of
        // the second query would be no run either: there is none (ADR 0071 D3).
        let entry = if unpinned || returned_unpinned {
            Vec::new()
        } else {
            ctx.entry.as_ref().clone()
        };
        for name in shown_forgotten {
            if !forgotten.contains(&name) {
                forgotten.push(name);
            }
        }
        let normalized: String = goal.split_whitespace().collect::<Vec<_>>().join(" ");
        let id = obligation_id(self.module, &self.declaration, kind, &normalized);
        self.obligations.push(Obligation {
            id,
            kind,
            module: self.module.to_owned(),
            declaration: self.declaration.clone(),
            goal: normalized,
            source,
            related,
            paths,
            hypotheses,
            query,
            shown,
            site,
            forgotten,
            entry,
            divisions,
        });
    }

    /// Starts a body: the loops of another one made nothing fresh here (ADR 0071 D3).
    fn forget_loops(&mut self) {
        self.loop_fresh.clear();
        self.loop_results.clear();
        self.unpinned.clear();
    }

    /// A binder computed from values a loop made fresh depends on them too (ADR 0071 D3).
    fn taint(&mut self, name: &str, value: &Bound) {
        if self.loop_fresh.is_empty() {
            return;
        }
        let mut used = BTreeSet::new();
        match value {
            Bound::Rhs(rhs) => una_core::rhs_vars(rhs, &mut used),
            Bound::Expr(expr) => una_core::vars(expr, &mut used),
        }
        let sources: BTreeSet<String> = used
            .iter()
            .filter_map(|used| self.loop_fresh.get(used))
            .flatten()
            .cloned()
            .collect();
        if !sources.is_empty() {
            self.loop_fresh.insert(name.to_owned(), sources);
            if used.iter().any(|used| self.unpinned.contains(used)) {
                self.unpinned.insert(name.to_owned());
            }
        }
    }

    /// Whether each state binding of the loop at `span` keeps its length (ADR 0071 D1).
    fn preserved(&mut self, span: Span, state: &[(Name, Type)], body: &Expr) -> Vec<bool> {
        let index = self.index;
        let contract = |path: &Path| {
            index
                .contracts
                .get(&(path.module.as_str(), path.name.as_str()))
                .map(|c| (c.params, c.ensures))
        };
        crate::frame::Frames::new(&contract, &mut self.frames).preserved(span, state, body)
    }

    /// The source names of the loop-fresh values that the terms a counterexample shows
    /// depend on, through the definitions of temporaries (ADR 0071 D3).
    fn forgotten<'t>(&self, terms: impl IntoIterator<Item = &'t F>) -> (Vec<String>, bool) {
        let mut names = Vec::new();
        let mut unpinned = false;
        for term in terms {
            let mut pending: Vec<String> = binders(term).into_iter().collect();
            let mut seen = BTreeSet::new();
            while let Some(name) = pending.pop() {
                if !seen.insert(name.clone()) {
                    continue;
                }
                unpinned |= self.unpinned.contains(&name);
                for source in self.loop_fresh.get(&name).into_iter().flatten() {
                    if !names.contains(source) {
                        names.push(source.clone());
                    }
                }
                if let Some(term) = self.defs.get(&name) {
                    pending.extend(binders(term));
                }
            }
        }
        (names, unpinned)
    }

    /// Where an obligation with `goal` arises (R-021). The free binders of the goal are its
    /// program constants; a temporary that stands for a pure term counts as the binders of
    /// that term, since the goal's text shows the term. D-147: a goal outside the
    /// encoding, and the `false` of an exhaustiveness obligation, are never ground, and the
    /// value is a parameter only when the goal's only constant, before any temporary is
    /// expanded, is a parameter of the enclosing function. A `requires` clause can state the
    /// goal only when its free binders are all parameters of that function; a handler's
    /// operation and an opaque predicate are not functions, so neither repair applies there.
    fn place(&self, kind: ObligationKind, goal: Option<&F>) -> Site {
        let Some(goal) = goal.filter(|_| kind != ObligationKind::Exhaustive) else {
            return Site {
                within: self.within,
                parameter: None,
                over_parameters: false,
                ground: false,
                flow: None,
                function_value: None,
                guard: None,
            };
        };
        let direct = binders(goal);
        let mut free = BTreeSet::new();
        let mut pending: Vec<String> = direct.iter().cloned().collect();
        let mut seen = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            match self.defs.get(&name) {
                Some(term) if name.starts_with('%') => pending.extend(binders(term)),
                _ => {
                    free.insert(name);
                }
            }
        }
        let is_function = matches!(self.within, Within::Function { .. });
        let parameter = match direct.iter().collect::<Vec<_>>().as_slice() {
            [only] if is_function && self.parameters.contains(*only) => Some((*only).clone()),
            _ => None,
        };
        Site {
            within: self.within,
            parameter,
            over_parameters: is_function
                && !free.is_empty()
                && free.iter().all(|name| self.parameters.contains(name)),
            ground: free.is_empty(),
            flow: None,
            function_value: None,
            guard: None,
        }
    }

    /// The facts about the terms of a query that no quantifier states (`spec/12 §2, §4`): lengths are at least 0, tags are in range, and fields satisfy their
    /// declared refinements.
    fn ground_facts(&mut self, query: &Query) -> (Vec<F>, Vec<F>) {
        let mut facts: Vec<F> = Vec::new();
        let mut listed: Vec<F> = Vec::new();
        let mut seen: BTreeSet<F> = BTreeSet::new();
        let mut pending: Vec<F> = query.applications().into_iter().collect();
        let mut rounds = 0;
        while !pending.is_empty() && rounds < 6 {
            rounds += 1;
            let mut next = Vec::new();
            for term in pending {
                if !seen.insert(term.clone()) {
                    continue;
                }
                let mut new = Vec::new();
                match &term {
                    F::App { fun, args } => match &fun.role {
                        FunRole::Len => {
                            new.push(F::op(">=", vec![term.clone(), F::Int("0".to_owned())]));
                        }
                        // A map that has a key is not empty, so an empty one has none.
                        FunRole::Has => {
                            if let Some(map) = args.first() {
                                new.push(F::op(
                                    "=>",
                                    vec![
                                        term.clone(),
                                        F::op(">=", vec![len_of(map), F::Int("1".to_owned())]),
                                    ],
                                ));
                            }
                        }
                        FunRole::Tag { variants, .. } => new.push(F::and(vec![
                            F::op("<=", vec![F::Int("0".to_owned()), term.clone()]),
                            F::op("<", vec![term.clone(), F::Int(variants.to_string())]),
                        ])),
                        FunRole::Field {
                            module,
                            kind,
                            variant,
                            index,
                            variants,
                        } => {
                            let declared = self
                                .index
                                .kinds
                                .get(&(module.as_str(), kind.as_str()))
                                .and_then(|k| k.variants.get(*variant))
                                .and_then(|v| v.fields.get(*index))
                                .map(|(_, ty)| ty.clone());
                            if let Some(declared) = declared {
                                let value = Tv {
                                    f: term.clone(),
                                    ty: declared.clone(),
                                };
                                let owner = args.first().cloned();
                                for refinement in refinements(&declared) {
                                    let env = Env {
                                        vars: &|_| None,
                                        it: Some(value.clone()),
                                        result: None,
                                    };
                                    let Some(fact) = self.term(&refinement.clause.term, &env)
                                    else {
                                        continue;
                                    };
                                    if *variants <= 1 {
                                        listed.push(fact.f.clone());
                                    }
                                    let fact = match (&owner, *variants > 1) {
                                        (Some(owner), true) => {
                                            let tag = tag_of(owner, module, kind, *variants);
                                            F::op(
                                                "=>",
                                                vec![
                                                    F::eq(tag, F::Int(variant.to_string())),
                                                    fact.f,
                                                ],
                                            )
                                        }
                                        _ => fact.f,
                                    };
                                    new.push(fact);
                                }
                            }
                        }
                        FunRole::Predicate | FunRole::Get | FunRole::Other => {}
                    },
                    F::Const(Const {
                        role: ConstRole::Text(text),
                        ..
                    }) => {
                        new.push(F::eq(
                            len_of(&term),
                            F::Int(text.chars().count().to_string()),
                        ));
                    }
                    _ => {}
                }
                for fact in new {
                    let probe = Query {
                        assertions: vec![fact.clone()],
                        definitions: BTreeMap::new(),
                    };
                    next.extend(probe.applications());
                    facts.push(fact);
                }
            }
            pending = next;
        }
        (facts, listed)
    }

    /// Adds the definitions of the reflect predicates the query uses, and of those they use.
    fn add_definitions(&self, query: &mut Query) {
        let mut pending: Vec<String> = Vec::new();
        let collect = |f: &F, out: &mut Vec<String>| {
            let probe = Query {
                assertions: vec![f.clone()],
                definitions: BTreeMap::new(),
            };
            for term in probe.applications() {
                if let F::App { fun, .. } = term
                    && fun.role == FunRole::Predicate
                {
                    out.push(fun.name.clone());
                }
            }
        };
        for assertion in &query.assertions {
            collect(assertion, &mut pending);
        }
        while let Some(name) = pending.pop() {
            if query.definitions.contains_key(&name) {
                continue;
            }
            if let Some(definition) = self.definitions.get(&name) {
                collect(&definition.body, &mut pending);
                query.definitions.insert(name, definition.clone());
            }
        }
    }

    /// The terms a counterexample shows: named binders, `result`, and the fields and lengths
    /// over them, by name.
    fn shown(&self, query: &Query, ctx: &Ctx) -> Vec<Shown> {
        let mut out: BTreeMap<String, Shown> = BTreeMap::new();
        for term in query.applications() {
            match &term {
                F::Const(Const {
                    name,
                    role: ConstRole::Plain,
                    ..
                }) if !name.starts_with('%') => {
                    let Some(ty) = ctx.types.get(name) else {
                        continue;
                    };
                    // A tuple is internal to the desugar of the memory model and has no
                    // source form (`spec/17 §4`).
                    if matches!(
                        ty,
                        Type::Base {
                            base: Base::Tuple(_),
                            ..
                        }
                    ) {
                        continue;
                    }
                    let assigned = self.assigned.as_deref() == Some(name.as_str());
                    let source = if assigned {
                        format!("{} (assigned)", source_name(name))
                    } else {
                        source_name(name)
                    };
                    // Of the binders of one name, the latest rebind shows: the value before
                    // the assignment, when the obligation is about the assigned one.
                    if out
                        .get(&source)
                        .is_some_and(|shown| rebind_number(&shown.term) > rebind_number(&term))
                    {
                        continue;
                    }
                    let value = Tv {
                        f: term.clone(),
                        ty: ty.clone(),
                    };
                    out.insert(
                        source.clone(),
                        Shown {
                            name: source,
                            term: term.clone(),
                            show: self.show_kind(&value),
                        },
                    );
                }
                F::App { fun, args }
                    if matches!(fun.role, FunRole::Field { .. } | FunRole::Len) =>
                {
                    let Some(owner) = args.first().and_then(|a| self.term_name(a)) else {
                        continue;
                    };
                    let (name, ty) = match &fun.role {
                        FunRole::Field {
                            module,
                            kind,
                            variant,
                            index,
                            ..
                        } => {
                            let field = source_symbol(fun);
                            let ty = self
                                .index
                                .kinds
                                .get(&(module.as_str(), kind.as_str()))
                                .and_then(|k| k.variants.get(*variant))
                                .and_then(|v| v.fields.get(*index))
                                .map_or_else(|| base_type(Base::Unknown), |(_, t)| t.clone());
                            (format!("{owner}.{field}"), ty)
                        }
                        _ => (format!("{owner}.len"), base_type(Base::Int)),
                    };
                    let value = Tv {
                        f: term.clone(),
                        ty,
                    };
                    out.insert(
                        name.clone(),
                        Shown {
                            name,
                            term: term.clone(),
                            show: self.show_kind(&value),
                        },
                    );
                }
                _ => {}
            }
        }
        out.into_values().collect()
    }

    /// The source name of a shown term: a named binder or a field of one.
    fn term_name(&self, f: &F) -> Option<String> {
        match f {
            F::Const(Const {
                name,
                role: ConstRole::Plain,
                ..
            }) if !name.starts_with('%') => Some(source_name(name)),
            F::App { fun, args } if matches!(fun.role, FunRole::Field { .. }) => {
                let owner = self.term_name(args.first()?)?;
                Some(format!("{owner}.{}", source_symbol(fun)))
            }
            _ => None,
        }
    }

    fn show_kind(&self, value: &Tv) -> Show {
        let sort = value.f.sort();
        if sort == "Int" || sort == "Bool" {
            return Show::Value;
        }
        if let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &value.ty
        {
            if let Some(tag) = self.tag(value) {
                return Show::Variant {
                    tag,
                    names: self.index.variant_names(module, name),
                };
            }
            return Show::Opaque(name.clone());
        }
        Show::Opaque(type_name(&value.ty))
    }

    /// How a binder shows: a temporary through its definition or its source text.
    fn show_name(&self, name: &str) -> String {
        self.show_binder(name).0
    }

    fn show_binder(&self, name: &str) -> (String, u8) {
        if let Some(def) = self.defs.get(name) {
            return self.show(def);
        }
        match self.temps.get(name) {
            Some(text) => (text.clone(), text_level(text)),
            None => (source_name(name), 1),
        }
    }

    fn show_temp(&mut self, name: &str, atom: &Atom) {
        if name.starts_with('%')
            && let Atom::Var(other) = atom
        {
            let shown = self.show_name(other);
            self.temps.insert(name.to_owned(), shown);
        }
    }

    /// The listed hypotheses: those that are not definitions of temporaries, as text.
    fn shown_hypotheses(&self, hyps: &Hyps) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for hyp in hyps.iter().filter(|h| h.shown) {
            for part in conjuncts(&hyp.f) {
                let text = self.show_formula(part);
                if !out.contains(&text) {
                    out.push(text);
                }
            }
        }
        out
    }

    /// A formula in a readable form of the surface syntax.
    fn show_formula(&self, f: &F) -> String {
        self.show(f).0
    }

    /// A formula as surface syntax, and its level in the precedence table of `spec/01 §7`:
    /// 1 binds tightest, 9 is `implies` and 10 an `if`. An operand shows in parentheses when
    /// its level is above the one its position allows.
    fn show(&self, f: &F) -> (String, u8) {
        let operand = |f: &F, max: u8| self.operand(f, max);
        let binary = |a: &F, op: &str, b: &F, level: u8| self.show_binary(a, op, b, level);
        match f {
            F::Int(digits) => (digits.clone(), if digits.starts_with('-') { 2 } else { 1 }),
            F::Bool(value) => (value.to_string(), 1),
            F::Const(Const { name, role, .. }) => match role {
                ConstRole::Text(text) => (una_core::quote(text), 1),
                _ => self.show_binder(name),
            },
            F::Local { name, .. } => (source_name(name), 1),
            F::App { fun, args } => {
                let first = || args.first().map_or_else(String::new, |a| operand(a, 1));
                let base = source_symbol(fun);
                match &fun.role {
                    FunRole::Field { .. } => (format!("{}.{base}", first()), 1),
                    FunRole::Len => (format!("{}.len", first()), 1),
                    FunRole::Tag { .. } => (format!("tag({})", first()), 1),
                    FunRole::Has | FunRole::Get => {
                        let key = args.get(1).map_or_else(String::new, |a| operand(a, 10));
                        (format!("{}.{base}(key: {key})", first()), 1)
                    }
                    FunRole::Predicate => {
                        let shown: Vec<String> = args.iter().map(|a| operand(a, 10)).collect();
                        let short = base.rsplit('.').next().unwrap_or(base);
                        (format!("{short}({})", shown.join(", ")), 1)
                    }
                    FunRole::Other if base == "contains" => {
                        let shown: Vec<String> = args.iter().map(|a| operand(a, 10)).collect();
                        (format!("contains_text({})", shown.join(", ")), 1)
                    }
                    FunRole::Other => {
                        let (op, level) = match base {
                            "mul" => ("*", 3),
                            "div" => ("/", 3),
                            "mod" => ("mod", 3),
                            "lt" => ("<", 5),
                            "le" => ("<=", 5),
                            "gt" => (">", 5),
                            _ => (">=", 5),
                        };
                        match args.as_slice() {
                            [a, b] => binary(a, op, b, level),
                            _ => (format!("{base}(..)"), 1),
                        }
                    }
                }
            }
            F::Op { op, args } => match (*op, self.resolved(args).as_slice()) {
                ("not", [F::Op { op: "=", args }]) if args.len() == 2 => {
                    match self.resolved(args).as_slice() {
                        [a, b] => self.show_equal(a, b, true),
                        _ => (String::new(), 1),
                    }
                }
                ("not", [a]) => (format!("not {}", operand(a, 6)), 6),
                ("-", [a]) => (format!("-{}", operand(a, 1)), 2),
                ("=", [a, b]) => self.show_equal(a, b, false),
                ("=>", [a, b]) => (format!("{} implies {}", operand(a, 8), operand(b, 9)), 9),
                ("ite", [c, a, F::Bool(false)]) | ("and", [c, a])
                    if let Some(range) = self.show_range(c, a) =>
                {
                    range
                }
                ("ite", [c, a, b]) => match (a, b) {
                    (F::Bool(true), F::Bool(false)) => self.show(c),
                    (F::Bool(false), F::Bool(true)) => (format!("not {}", operand(c, 6)), 6),
                    (_, F::Bool(false)) => (format!("{} and {}", operand(c, 7), operand(a, 7)), 7),
                    (F::Bool(true), _) => (format!("{} or {}", operand(c, 8), operand(b, 8)), 8),
                    (_, F::Bool(true)) => {
                        (format!("{} implies {}", operand(c, 8), operand(a, 9)), 9)
                    }
                    _ => (
                        format!(
                            "if {} then {} else {}",
                            operand(c, 10),
                            operand(a, 9),
                            operand(b, 10)
                        ),
                        10,
                    ),
                },
                ("and" | "or", parts) => {
                    let level = if *op == "and" { 7 } else { 8 };
                    let shown: Vec<String> = parts.iter().map(|p| operand(p, level)).collect();
                    (shown.join(&format!(" {op} ")), level)
                }
                ("+" | "-" | "*" | "div" | "mod" | "<" | "<=" | ">" | ">=", [a, b]) => {
                    let (op, level) = match *op {
                        "+" => ("+", 4),
                        "-" => ("-", 4),
                        "*" => ("*", 3),
                        "div" => ("/", 3),
                        "mod" => ("mod", 3),
                        other => (other, 5),
                    };
                    binary(a, op, b, level)
                }
                (op, parts) => {
                    let shown: Vec<String> = parts.iter().map(|p| operand(p, 4)).collect();
                    (shown.join(&format!(" {op} ")), 5)
                }
            },
        }
    }

    /// The formulas, each temporary replaced by its definition.
    fn resolved<'f>(&'f self, fs: &'f [F]) -> Vec<&'f F> {
        fs.iter().map(|f| self.resolve(f)).collect()
    }

    /// A temporary's definition, followed to a formula that is not one.
    fn resolve<'f>(&'f self, f: &'f F) -> &'f F {
        let mut f = f;
        while let F::Const(Const { name, .. }) = f
            && let Some(def) = self.defs.get(name)
        {
            f = def;
        }
        f
    }

    /// `lo <= x and x <= hi` as `x in lo..hi`, and `x in lo..<hi` with `<`.
    fn show_range(&self, low: &F, high: &F) -> Option<(String, u8)> {
        let (
            F::Op {
                op: "<=",
                args: lower,
            },
            F::Op {
                op: upper,
                args: higher,
            },
        ) = (self.resolve(low), self.resolve(high))
        else {
            return None;
        };
        let (lower, higher) = (self.resolved(lower), self.resolved(higher));
        let ([F::Int(lo), x], [y, F::Int(hi)]) = (lower.as_slice(), higher.as_slice()) else {
            return None;
        };
        let range = match *upper {
            "<=" => "..",
            "<" => "..<",
            _ => return None,
        };
        if x != y {
            return None;
        }
        let shown = self.operand(x, 4);
        Some((format!("{shown} in {lo}{range}{hi}"), 5))
    }

    /// `a == b` or `a != b`; the variant of a value against a literal shows as `is`.
    fn show_equal(&self, a: &F, b: &F, negated: bool) -> (String, u8) {
        if let (
            F::App {
                fun:
                    SmtFun {
                        role: FunRole::Tag { module, kind, .. },
                        ..
                    },
                args,
            },
            F::Int(digits),
        ) = (a, b)
            && let [value] = args.as_slice()
            && let Ok(index) = digits.parse::<usize>()
            && let Some(variant) = self.index.variant_names(module, kind).get(index).cloned()
        {
            let is = format!("{} is {variant}", self.operand(value, 4));
            return if negated {
                (format!("not {is}"), 6)
            } else {
                (is, 5)
            };
        }
        self.show_binary(a, if negated { "!=" } else { "==" }, b, 5)
    }

    /// An operand at a position that allows levels up to `max`, in parentheses above it.
    fn operand(&self, f: &F, max: u8) -> String {
        let (text, level) = self.show(f);
        if level > max {
            format!("({text})")
        } else {
            text
        }
    }

    /// `a op b` at `level`: `*`, `/`, `mod`, `+` and `-` associate to the left, and
    /// comparisons do not chain.
    fn show_binary(&self, a: &F, op: &str, b: &F, level: u8) -> (String, u8) {
        let (left, right) = if level == 5 {
            (4, 4)
        } else {
            (level, level - 1)
        };
        (
            format!("{} {op} {}", self.operand(a, left), self.operand(b, right)),
            level,
        )
    }
}

/// The precedence level of a temporary's source text: 1 when it reads as one operand, with
/// no space outside brackets and text literals; 2 for a negation; 10 otherwise.
fn text_level(text: &str) -> u8 {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for c in text.chars() {
        if quoted {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => return 10,
            _ => {}
        }
    }
    if text.starts_with('-') { 2 } else { 1 }
}

/// The value of a binder in scope.
/// The program binders a term mentions: its constants of role `Plain`.
fn binders(term: &F) -> BTreeSet<String> {
    let mut terms = Vec::new();
    term.visit(&mut terms);
    terms
        .into_iter()
        .filter_map(|t| match t {
            F::Const(Const {
                name,
                role: ConstRole::Plain,
                ..
            }) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn own(ctx: &Ctx, name: &str) -> Tv {
    let ty = ctx
        .types
        .get(name)
        .cloned()
        .unwrap_or_else(|| base_type(Base::Unknown));
    Tv {
        f: constant(name, &ty),
        ty,
    }
}

fn constant(name: &str, ty: &Type) -> F {
    F::Const(Const {
        name: name.to_owned(),
        sort: sort(ty),
        role: ConstRole::Plain,
    })
}

/// A text literal: one constant per distinct text, `i!lit!<hex>` by its bytes (`spec/12 §4`).
fn text_literal(text: &str) -> Tv {
    let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
    Tv {
        f: F::Const(Const {
            name: hex,
            sort: TEXT.to_owned(),
            role: ConstRole::Text(text.to_owned()),
        }),
        ty: base_type(Base::Text),
    }
}

/// The tag of `value`, of the sum at (`module`, `kind`) with `variants` variants,
/// `i!tag!<Sort>`.
fn tag_of(value: &F, module: &str, kind: &str, variants: usize) -> F {
    let sort = value.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!tag!{sort}"),
            args: vec![sort],
            result: "Int".to_owned(),
            role: FunRole::Tag {
                module: module.to_owned(),
                kind: kind.to_owned(),
                variants,
            },
        },
        args: vec![value.clone()],
    }
}

/// Whether `sort` is the sort of a type parameter, `u!Param_<name>` (`spec/12 §4`).
fn is_param_sort(sort: &str) -> bool {
    sort.starts_with("u!Param_")
}

/// `Option[V]` of the prelude, for a map type `Map[K, V]`.
fn option_of(map: &Type) -> Option<Type> {
    let Type::Base {
        base: Base::Named { args, .. },
        ..
    } = map
    else {
        return None;
    };
    Some(Type::Base {
        base: Base::Named {
            module: una_core::PRELUDE_MODULE.to_owned(),
            name: "Option".to_owned(),
            args: vec![args.get(1)?.clone()],
        },
        refinements: Vec::new(),
        label: una_core::Label::public(),
    })
}

/// `m.has(key: k)`, `i!has!<Sort>`, uninterpreted (`spec/21 §4`).
fn has_of(map: &F, key: &F) -> F {
    let sort = map.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!has!{sort}"),
            args: vec![sort, key.sort()],
            result: "Bool".to_owned(),
            role: FunRole::Has,
        },
        args: vec![map.clone(), key.clone()],
    }
}

/// `m.get(key: k)`, `i!get!<Sort>`, uninterpreted, of the option sort `option`.
fn get_of(map: &F, key: &F, option: &Sort) -> F {
    let sort = map.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!get!{sort}"),
            args: vec![sort, key.sort()],
            result: option.clone(),
            role: FunRole::Get,
        },
        args: vec![map.clone(), key.clone()],
    }
}

/// The length of a text, a list or a map, `i!len!<Sort>`.
fn len_of(value: &F) -> F {
    let sort = value.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!len!{sort}"),
            args: vec![sort],
            result: "Int".to_owned(),
            role: FunRole::Len,
        },
        args: vec![value.clone()],
    }
}

/// The source name in a function's symbol: the field of `u!f!<field>!...` and
/// `i!f!<field>!...`, the predicate of `u!p!<qualified name>`, and the operation of
/// `i!<op>!<Sort>`.
fn source_symbol(fun: &SmtFun) -> &str {
    let name = fun.name.as_str();
    match fun.role {
        FunRole::Predicate => name.strip_prefix("u!p!").unwrap_or(name),
        FunRole::Field { .. } => name.split('!').nth(2).unwrap_or(name),
        _ => name.split('!').nth(1).unwrap_or(name),
    }
}

/// Whether `f` is an integer literal, or the negation of one.
fn int_literal(f: &F) -> bool {
    match f {
        F::Int(_) => true,
        F::Op { op: "-", args } => matches!(args.as_slice(), [inner] if int_literal(inner)),
        _ => false,
    }
}

/// Whether `f` is a nonzero integer literal, or the negation of one.
fn nonzero_literal(f: &F) -> bool {
    match f {
        F::Int(digits) => digits.trim_start_matches('-') != "0",
        F::Op { op: "-", args } => matches!(args.as_slice(), [inner] if nonzero_literal(inner)),
        _ => false,
    }
}

/// Component `index` of a tuple value, `i!t!<index>!<Sort>` (`spec/17 §4`), whose tuple sort
/// has a short name (#333).
fn projection(tuple: &Tv, index: usize) -> Option<Tv> {
    let Type::Base {
        base: Base::Tuple(types),
        ..
    } = &tuple.ty
    else {
        return None;
    };
    let ty = types.get(index)?.clone();
    let sort = tuple.f.sort();
    Some(Tv {
        f: F::App {
            fun: SmtFun {
                name: format!("i!t!{index}!{sort}"),
                args: vec![sort],
                result: self::sort(&ty),
                role: FunRole::Other,
            },
            args: vec![tuple.f.clone()],
        },
        ty,
    })
}

/// `least <= value <= greatest` of a fixed-width type (`spec/18 §6`).
fn in_range(value: F, int: IntType) -> F {
    F::and(vec![
        F::op("<=", vec![F::Int(int.least().to_string()), value.clone()]),
        F::op("<=", vec![value, F::Int(int.greatest().to_string())]),
    ])
}

/// `value` reduced into the range of `int` modulo `2^width`: `((value - least) mod 2^w) +
/// least`, linear since `2^w` is a numeral (`spec/18 §5`, `§6`).
fn wrapped(value: F, int: IntType) -> F {
    let least = F::Int(int.least().to_string());
    let modulus = F::Int((1u128 << int.bits()).to_string());
    F::op(
        "+",
        vec![
            F::op("mod", vec![F::op("-", vec![value, least.clone()]), modulus]),
            least,
        ],
    )
}

/// `value` clamped to the range of `int` (`spec/18 §5`).
fn clamped(value: F, int: IntType) -> F {
    let (least, greatest) = (
        F::Int(int.least().to_string()),
        F::Int(int.greatest().to_string()),
    );
    F::op(
        "ite",
        vec![
            F::op("<", vec![value.clone(), least.clone()]),
            least,
            F::op(
                "ite",
                vec![
                    F::op(">", vec![value.clone(), greatest.clone()]),
                    greatest,
                    value,
                ],
            ),
        ],
    )
}

/// Arithmetic in QF_UFLIA: multiplication, division and remainder stay linear only with a
/// literal operand (a nonzero divisor); otherwise they are uninterpreted functions, since the
/// logic has no nonlinear terms (`spec/12 §2`).
fn arithmetic(op: BinOp, left: F, right: F) -> F {
    let (symbol, name, linear) = match op {
        BinOp::Add => ("+", "add", true),
        BinOp::Sub => ("-", "sub", true),
        // A literal, negated or not, keeps a product, a quotient or a remainder linear (#188).
        BinOp::Mul => ("*", "mul", int_literal(&left) || int_literal(&right)),
        BinOp::Div => ("div", "div", nonzero_literal(&right)),
        _ => ("mod", "mod", nonzero_literal(&right)),
    };
    if linear {
        F::op(symbol, vec![left, right])
    } else {
        F::App {
            fun: SmtFun {
                name: format!("i!{name}!Int"),
                args: vec!["Int".to_owned(), "Int".to_owned()],
                result: "Int".to_owned(),
                role: FunRole::Other,
            },
            args: vec![left, right],
        }
    }
}

/// The conjuncts of a formula, for listing.
fn conjuncts(f: &F) -> Vec<&F> {
    match f {
        F::Op { op: "and", args } => args.iter().flat_map(conjuncts).collect(),
        other => vec![other],
    }
}

/// Whether a type holds a function type, at its top or in a type argument (ADR 0037).
fn contains_fn(ty: &Type) -> bool {
    match ty {
        Type::Fn { .. } => true,
        Type::Base {
            base: Base::Named { args, .. },
            ..
        } => args.iter().any(contains_fn),
        Type::Base { .. } => false,
    }
}

/// A type as Una writes it, for repairs: a refined position by its kind or range.
/// The label at the top of `ty`.
fn type_label(ty: &Type) -> &una_core::Label {
    match ty {
        Type::Fn { label, .. } | Type::Base { label, .. } => label,
    }
}

/// `ty` as a header writes it: [`show_type`] with the label of each position that has one
/// above `public`, as `secret Int` or `List[secret[PII] Text]`. A type parameter's symbolic
/// label `ℓ_T` is not written (`spec/07 §7` rule 8).
fn written_type(ty: &Type) -> String {
    let (label, shown) = match ty {
        Type::Fn {
            params,
            result,
            label,
            ..
        } => {
            let params: Vec<String> = params.iter().map(written_type).collect();
            (
                label,
                format!("fn({}) -> {}", params.join(", "), written_type(result)),
            )
        }
        Type::Base {
            base,
            refinements,
            label,
        } => {
            let shown = match (base, refinements.last()) {
                (_, Some(refinement)) if !refinement.name.starts_with("in ") => {
                    refinement.name.clone()
                }
                (Base::Named { name, args, .. }, refinement) if !args.is_empty() => {
                    let args: Vec<String> = args.iter().map(written_type).collect();
                    let plain = format!("{name}[{}]", args.join(", "));
                    match refinement {
                        Some(range) => format!("{plain} {}", range.name),
                        None => plain,
                    }
                }
                _ => show_type(ty),
            };
            (label, shown)
        }
    };
    let concrete = label.concrete();
    if concrete.is_public() {
        shown
    } else {
        format!("{concrete} {shown}")
    }
}

fn show_type(ty: &Type) -> String {
    match ty {
        Type::Fn { params, result, .. } => {
            let params: Vec<String> = params.iter().map(show_type).collect();
            format!("fn({}) -> {}", params.join(", "), show_type(result))
        }
        Type::Base {
            base, refinements, ..
        } => {
            let plain = match base {
                Base::Named { name, args, .. } if !args.is_empty() => {
                    let args: Vec<String> = args.iter().map(show_type).collect();
                    format!("{name}[{}]", args.join(", "))
                }
                _ => type_name(ty),
            };
            match refinements.last() {
                Some(refinement) if refinement.name.starts_with("in ") => {
                    format!("{plain} {}", refinement.name)
                }
                Some(refinement) => refinement.name.clone(),
                None => plain,
            }
        }
    }
}

/// The rebind number of a binder's constant: 0 for `x`, 2 for `x%2`.
fn rebind_number(term: &F) -> usize {
    match term {
        F::Const(Const { name, .. }) => name
            .split_once('%')
            .and_then(|(_, number)| number.parse().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

/// A binder's source name: `x%2` is `x`.
fn source_name(name: &str) -> String {
    match name.split_once('%') {
        Some((source, _)) if !source.is_empty() => source.to_owned(),
        _ => name.to_owned(),
    }
}

/// A refinement's term with `it` written as `value`.
fn instantiate(term: &Term, value: &str) -> String {
    print_term_with(term, &|t| matches!(t, Term::It).then(|| value.to_owned()))
}

/// What declares a refinement, for the related span.
fn clause_role(refinement: &Refinement) -> &'static str {
    if refinement.name.starts_with("in ") {
        "range"
    } else {
        "where clause"
    }
}

/// How a primitive's result shows, from its operands' texts.
fn prim_text(prim: Prim, args: &[String], whole: &str) -> String {
    match (prim, args) {
        (Prim::Len, [a]) => format!("{a}.len"),
        _ => whole.to_owned(),
    }
}

/// `ob-` and the first 12 hex digits of SHA-256 over the module path, the declaration, the
/// kind and the normalized goal (`spec/12 §8`).
fn obligation_id(module: &str, declaration: &str, kind: ObligationKind, goal: &str) -> String {
    let digest =
        Sha256::digest(format!("{module}\n{declaration}\n{}\n{goal}", kind.as_str()).as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("ob-{}", hex.get(..12).unwrap_or(&hex))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(base: Base) -> Type {
        Type::Base {
            base,
            refinements: Vec::new(),
            label: una_core::Label::public(),
        }
    }

    #[test]
    fn a_tuple_sort_has_a_short_name_that_tells_its_components_apart() {
        // #333: the same components give the same name, any difference gives another, and the
        // length does not grow with the number of components.
        let ints = |n: usize| base(Base::Tuple(vec![base(Base::Int); n]));
        let mixed = base(Base::Tuple(vec![base(Base::Int), base(Base::Bool)]));
        let swapped = base(Base::Tuple(vec![base(Base::Bool), base(Base::Int)]));
        assert_eq!(sort(&ints(2)), sort(&ints(2)));
        assert_ne!(sort(&ints(2)), sort(&mixed));
        assert_ne!(sort(&mixed), sort(&swapped));
        assert_ne!(sort(&ints(2)), sort(&ints(3)));
        assert!(sort(&ints(2)).starts_with("i!Tuple2!"));
        assert_eq!(sort(&ints(1000)).len(), "i!Tuple1000!".len() + 32);
        let nested = base(Base::Tuple(vec![ints(2), base(Base::Int)]));
        let flat = base(Base::Tuple(vec![
            base(Base::Int),
            base(Base::Int),
            base(Base::Int),
        ]));
        assert_ne!(sort(&nested), sort(&flat));
    }

    fn int(digits: &str) -> F {
        F::Int(digits.to_owned())
    }

    fn var(name: &str, sort: &str) -> F {
        F::Const(Const {
            name: name.to_owned(),
            sort: sort.to_owned(),
            role: ConstRole::Plain,
        })
    }

    /// A package whose user names include `names`: parameters of `f`, fields of its record
    /// kind and kinds of their own, all of them in its clauses, next to a sum whose variants
    /// share a field name with different types and have a field named `tag`.
    fn namespace_program(names: &[String], span: Span) -> Program {
        let int = || base_type(Base::Int);
        let text = || base_type(Base::Text);
        let named = |name: &str| {
            base_type(Base::Named {
                module: "m".to_owned(),
                name: name.to_owned(),
                args: Vec::new(),
            })
        };
        let option = base_type(Base::Named {
            module: una_core::PRELUDE_MODULE.to_owned(),
            name: "Option".to_owned(),
            args: vec![int()],
        });
        let kind = |name: &str| una_core::Path {
            module: "m".to_owned(),
            name: name.to_owned(),
        };
        let var = |name: &str| Term::Var(name.to_owned());
        let eq = |left: Term, right: Term| Term::Binary {
            op: BinOp::Eq,
            left: Box::new(left),
            right: Box::new(right),
        };
        let clause = |term: Term| Clause {
            term,
            text: String::new(),
            span: Some(span),
        };
        // User names that equal the encoder's names, with and without their prefixes.
        let mut user: Vec<String> = [
            "tag",
            "len",
            "lit",
            "path!1",
            "unit",
            "value",
            "failure",
            "div",
            "mod",
            "mul",
            "lt",
            "result",
            "Text",
            "Unit",
            "Hashed",
            "Fn",
            "Unknown",
            "List_Int",
            "Option_Int",
        ]
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
        user.extend(names.iter().cloned());
        let record_fields: Vec<(String, Type)> = user.iter().map(|n| (n.clone(), int())).collect();
        let mut decls = vec![
            Decl::Data {
                name: "K".to_owned(),
                params: Vec::new(),
                variants: vec![
                    DataVariant {
                        name: "Stamp".to_owned(),
                        fields: vec![("tag".to_owned(), int())],
                    },
                    DataVariant {
                        name: "Label".to_owned(),
                        fields: vec![("size".to_owned(), text())],
                    },
                    DataVariant {
                        name: "Circle".to_owned(),
                        fields: vec![("size".to_owned(), int())],
                    },
                ],
                record: false,
            },
            Decl::Data {
                name: "Rec".to_owned(),
                params: Vec::new(),
                variants: vec![DataVariant {
                    name: "Rec".to_owned(),
                    fields: record_fields,
                }],
                record: true,
            },
        ];
        // Predicates named like the encoder's functions, one opaque and one reflect.
        for (name, formula) in [("tag", None), ("len", Some(eq(var("unit"), var("unit"))))] {
            decls.push(Decl::Pred {
                name: name.to_owned(),
                reflect: formula.is_some(),
                params: vec![("unit".to_owned(), int())],
                body: Expr::Atom {
                    atom: Atom::Bool(true),
                    span,
                },
                formula,
                span,
            });
        }
        // Kinds named like the encoder's sorts.
        for name in ["Text", "Unit", "Hashed", "Fn", "Unknown", "Option"] {
            decls.push(Decl::Data {
                name: name.to_owned(),
                params: Vec::new(),
                variants: vec![DataVariant {
                    name: name.to_owned(),
                    fields: Vec::new(),
                }],
                record: false,
            });
        }
        let mut params: Vec<(Name, Type)> = user.iter().map(|n| (n.clone(), int())).collect();
        params.push(("r".to_owned(), named("Rec")));
        params.push(("k".to_owned(), named("K")));
        params.push(("o".to_owned(), option));
        params.push(("s".to_owned(), text()));
        params.push(("h".to_owned(), base_type(Base::Hashed)));
        for name in ["Text", "Unit", "Hashed", "Fn", "Unknown", "Option"] {
            params.push((format!("v{name}"), named(name)));
        }
        let mut requires: Vec<Term> = user.iter().map(|n| eq(var(n), var(n))).collect();
        for (index, name) in user.iter().enumerate() {
            let field = Term::Field {
                value: Box::new(var("r")),
                kind: kind("Rec"),
                field: name.clone(),
                index,
            };
            requires.push(eq(field.clone(), field));
        }
        for name in ["Text", "Unit", "Hashed", "Fn", "Unknown", "Option"] {
            let value = var(&format!("v{name}"));
            requires.push(Term::Is {
                value: Box::new(value),
                ctor: Ctor {
                    kind: kind(name),
                    name: name.to_owned(),
                    index: 0,
                },
            });
        }
        requires.push(eq(Term::Len(Box::new(var("s"))), Term::Int("3".to_owned())));
        requires.push(eq(var("s"), Term::Text("tag".to_owned())));
        requires.push(Term::Binary {
            op: BinOp::Lt,
            left: Box::new(var("s")),
            right: Box::new(Term::Text("lit".to_owned())),
        });
        requires.push(eq(
            Term::Binary {
                op: BinOp::Mul,
                left: Box::new(var("mul")),
                right: Box::new(var("lt")),
            },
            var("mod"),
        ));
        let some = Ctor {
            kind: una_core::Path {
                module: una_core::PRELUDE_MODULE.to_owned(),
                name: "Option".to_owned(),
            },
            name: "Some".to_owned(),
            index: 0,
        };
        let payload = Term::Field {
            value: Box::new(var("o")),
            kind: some.kind.clone(),
            field: "value".to_owned(),
            index: 0,
        };
        requires.push(Term::Is {
            value: Box::new(var("o")),
            ctor: some,
        });
        requires.push(eq(payload.clone(), payload));
        requires.push(eq(var("h"), var("h")));
        for predicate in ["tag", "len"] {
            requires.push(Term::Call {
                predicate: kind(predicate),
                args: vec![var("unit")],
            });
        }
        let arm = |index: usize, bind: &str, body: Expr| una_core::Arm {
            ctor: Ctor {
                kind: kind("K"),
                name: String::new(),
                index,
            },
            binds: vec![Some(bind.to_owned())],
            body,
        };
        let returned = |atom: Atom| Expr::Atom { atom, span };
        let divide = Expr::Let {
            name: "q".to_owned(),
            ty: int(),
            value: Box::new(Bound::Rhs(Rhs::Prim {
                prim: Prim::Div,
                args: vec![Atom::Var("div".to_owned()), Atom::Var("mod".to_owned())],
                arg_spans: vec![span, span],
                span,
            })),
            body: Box::new(Expr::Let {
                name: "u".to_owned(),
                ty: base_type(Base::Unit),
                value: Box::new(Bound::Rhs(Rhs::Atom(Atom::Unit))),
                body: Box::new(returned(Atom::Var("q".to_owned()))),
                span,
                written: false,
            }),
            span,
            written: false,
        };
        let body = Expr::Case {
            scrutinee: Atom::Var("k".to_owned()),
            arms: vec![
                arm(0, "tag%2", returned(Atom::Var("tag%2".to_owned()))),
                arm(1, "size", returned(Atom::Int("1".to_owned()))),
                arm(2, "size%2", divide),
            ],
            otherwise: None,
        };
        let positive = Term::Binary {
            op: BinOp::Gt,
            left: Box::new(Term::Result),
            right: Box::new(Term::Int("0".to_owned())),
        };
        decls.push(Decl::Fun(Box::new(Fun {
            name: "f".to_owned(),
            type_params: Vec::new(),
            constraints: Vec::new(),
            params,
            result: int(),
            uses: Vec::new(),
            fails: None,
            error: None,
            requires: requires.into_iter().map(clause).collect(),
            ensures: vec![clause(positive)],
            body,
            span,
            public: true,
        })));
        Program {
            modules: vec![una_core::Module {
                path: "m".to_owned(),
                decls,
                examples: Vec::new(),
            }],
            ..Program::default()
        }
    }

    /// The symbols a query declares or defines, in order.
    fn declared(query: &str) -> Vec<String> {
        query
            .lines()
            .filter_map(|line| {
                let rest = line
                    .strip_prefix("(declare-sort ")
                    .or_else(|| line.strip_prefix("(declare-fun "))
                    .or_else(|| line.strip_prefix("(define-fun "))?;
                rest.split(' ')
                    .next()
                    .map(|s| s.trim_matches('|').to_owned())
            })
            .collect()
    }

    #[test]
    fn user_names_never_share_a_symbol_with_the_encoder() {
        // ADR 0038 D8 and `spec/12 §4`: user symbols start with `u!` and the encoder's with
        // `i!`. The first program collects the encoder's symbols; the second declares user
        // names equal to every one of them, and no query declares one symbol twice.
        let mut sources = SourceDb::new();
        let file = sources.add("m.una", b"module m\n");
        let span = Span::new(file, 0, 6);
        let first = namespace_program(&[], span);
        let mut internal: BTreeSet<String> = BTreeSet::new();
        let obligations = super::obligations(&first, &sources);
        assert!(obligations.len() >= 2, "{}", obligations.len());
        for obligation in &obligations {
            for symbol in declared(&obligation.query.text()) {
                if symbol.starts_with("i!") {
                    internal.insert(symbol);
                }
            }
        }
        for expected in [
            "i!Text",
            "i!Hashed",
            "i!Unit",
            "i!Option_Int",
            "i!tag!u!Kind_m_K",
            "i!tag!i!Option_Int",
            "i!f!value!i!Option_Int",
            "i!len!i!Text",
            "i!lt!Text",
            "i!mul!Int",
            "i!div!Int",
            "i!lit!746167",
            "i!v!path!1",
            "i!v!unit",
        ] {
            assert!(internal.contains(expected), "{expected} in {internal:?}");
        }
        let names: Vec<String> = internal.iter().cloned().collect();
        let second = namespace_program(&names, span);
        let obligations = super::obligations(&second, &sources);
        let mut users = 0;
        for obligation in &obligations {
            // No symbol stands for two things: a declaration is emitted once per symbol, so two
            // functions or constants sharing one would be merged silently (R-009, R-019).
            let mut meaning: BTreeMap<String, String> = BTreeMap::new();
            for term in obligation.query.applications() {
                let (symbol, what) = match &term {
                    F::App { fun, .. } => (
                        fun.name.clone(),
                        format!("fun {:?} {:?} {:?}", fun.args, fun.result, fun.role),
                    ),
                    F::Const(c) => (c.symbol(), format!("const {} {:?}", c.sort, c.role)),
                    _ => continue,
                };
                if let Some(first) = meaning.insert(symbol.clone(), what.clone()) {
                    assert_eq!(first, what, "{symbol} stands for two things");
                }
            }
            let text = obligation.query.text();
            let symbols = declared(&text);
            let unique: BTreeSet<&String> = symbols.iter().collect();
            assert_eq!(unique.len(), symbols.len(), "{text}");
            for symbol in &symbols {
                assert!(
                    symbol.starts_with("u!") || symbol.starts_with("i!"),
                    "{symbol} has no namespace in {text}"
                );
            }
            users += symbols
                .iter()
                .filter(|s| {
                    names
                        .iter()
                        .any(|n| s.ends_with(n.as_str()) && s.starts_with("u!"))
                })
                .count();
        }
        // Every internal symbol reappears as a user binder or field, in the user namespace.
        assert!(users >= names.len(), "{users} < {}", names.len());
        // Variants sharing the field `size` with two types have two functions (R-019).
        let all: String = obligations.iter().map(|o| o.query.text()).collect();
        assert!(all.contains("u!f!size!Label!u!Kind_m_K"), "{all}");
        assert!(all.contains("u!f!size!Circle!u!Kind_m_K"), "{all}");
        assert!(all.contains("u!f!tag!Stamp!u!Kind_m_K"), "{all}");
    }

    #[test]
    fn a_kind_the_package_declares_is_never_a_prelude_kind() {
        // R-010: tier 0 rejects a module named `prelude` (U1009); the verifier still takes the
        // variants of a kind the package declares from the declaration.
        let variant = |name: &str| DataVariant {
            name: name.to_owned(),
            fields: Vec::new(),
        };
        let program = Program {
            modules: vec![una_core::Module {
                path: una_core::PRELUDE_MODULE.to_owned(),
                decls: vec![Decl::Data {
                    name: "KeyError".to_owned(),
                    params: Vec::new(),
                    variants: vec![
                        variant("UnknownKey"),
                        variant("Revoked"),
                        variant("Expired"),
                    ],
                    record: false,
                }],
                examples: Vec::new(),
            }],
            ..Program::default()
        };
        let index = Index::new(&program);
        assert_eq!(index.variant_names("prelude", "KeyError").len(), 3);
        assert_eq!(index.variant_names("prelude", "FileError").len(), 3);
        let empty = Program::default();
        let index = Index::new(&empty);
        assert_eq!(
            index.variant_names("prelude", "KeyError"),
            vec!["UnknownKey"]
        );
    }

    #[test]
    fn capabilities_are_looked_up_by_qualified_identity() {
        // ADR 0038 D6: a user capability `Files` is not `std.host.Files`, and two capabilities
        // of one name in two modules keep their own contracts.
        let read = |ensures: Vec<Clause>| CapOp {
            name: "read".to_owned(),
            params: vec![("path".to_owned(), base_type(Base::Text))],
            result: base_type(Base::Text),
            fails: None,
            error: None,
            requires: Vec::new(),
            ensures,
            accepts: None,
        };
        let nonempty = Clause {
            term: Term::Bool(true),
            text: "result.len > 0".to_owned(),
            span: None,
        };
        let files = |ensures| Decl::Cap {
            name: "Files".to_owned(),
            ops: vec![read(ensures)],
        };
        let program = Program {
            modules: vec![
                una_core::Module {
                    path: "aaa".to_owned(),
                    decls: vec![files(vec![nonempty])],
                    examples: Vec::new(),
                },
                una_core::Module {
                    path: "bbb".to_owned(),
                    decls: vec![files(Vec::new())],
                    examples: Vec::new(),
                },
            ],
            library: Vec::new(),
            host: vec![files(Vec::new())],
        };
        let index = Index::new(&program);
        let ensures = |module: &str| {
            let path = una_core::Path {
                module: module.to_owned(),
                name: "Files".to_owned(),
            };
            index.operation(&path, "read").map(|op| op.ensures.len())
        };
        assert_eq!(ensures(una_core::HOST_MODULE), Some(0));
        assert_eq!(ensures("aaa"), Some(1));
        assert_eq!(ensures("bbb"), Some(0));
        assert_eq!(ensures("ccc"), None);
    }

    #[test]
    fn formulas_show_as_source_with_the_parentheses_of_the_precedence_table() {
        // `spec/01 §7`: the hypotheses of `una obligations` read as Una.
        let program = Program::default();
        let index = Index::new(&program);
        let sources = SourceDb::new();
        let mut walker = Walker::new(&index, &sources, "m");
        let (a, b, c) = (var("a", "Int"), var("b", "Int"), var("c", "Int"));
        let low = F::op("<=", vec![int("0"), a.clone()]);
        let high = F::op("<", vec![a.clone(), int("20")]);
        walker
            .defs
            .insert("%t0".to_owned(), F::op("+", vec![a.clone(), int("1")]));
        walker
            .temps
            .insert("%t1".to_owned(), "f(x, \"a b\")".to_owned());
        walker.temps.insert("%t2".to_owned(), "g(x) + 1".to_owned());
        let option = kind_sort("prelude", "Option", &["Int".to_owned()]);
        let tag = F::App {
            fun: SmtFun {
                name: format!("tag!{option}"),
                args: vec![option.clone()],
                result: "Int".to_owned(),
                role: FunRole::Tag {
                    module: "prelude".to_owned(),
                    kind: "Option".to_owned(),
                    variants: 2,
                },
            },
            args: vec![var("o", &option)],
        };
        let cases = [
            (
                F::op("*", vec![F::op("+", vec![a.clone(), b.clone()]), int("2")]),
                "(a + b) * 2",
            ),
            (
                F::op("-", vec![a.clone(), F::op("-", vec![b.clone(), c.clone()])]),
                "a - (b - c)",
            ),
            (
                F::op("-", vec![F::op("-", vec![a.clone(), b.clone()]), c.clone()]),
                "a - b - c",
            ),
            (F::op("-", vec![int("-1")]), "-(-1)"),
            (F::not(F::eq(a.clone(), b.clone())), "a != b"),
            (
                F::not(F::op(
                    "ite",
                    vec![low.clone(), high.clone(), F::Bool(false)],
                )),
                "not a in 0..<20",
            ),
            (
                F::op("ite", vec![low.clone(), F::Bool(true), high.clone()]),
                "0 <= a or a < 20",
            ),
            (
                F::not(F::op("ite", vec![low.clone(), F::Bool(true), high.clone()])),
                "not (0 <= a or a < 20)",
            ),
            (
                F::op(
                    "=>",
                    vec![F::op("=>", vec![low.clone(), high.clone()]), low.clone()],
                ),
                "(0 <= a implies a < 20) implies 0 <= a",
            ),
            (
                F::op(
                    "ite",
                    vec![
                        low.clone(),
                        F::op("+", vec![a.clone(), b.clone()]),
                        c.clone(),
                    ],
                ),
                "if 0 <= a then a + b else c",
            ),
            (F::eq(tag.clone(), int("1")), "o is None"),
            (F::not(F::eq(tag, int("0"))), "not o is Some"),
            (F::op("*", vec![var("%t0", "Int"), int("2")]), "(a + 1) * 2"),
            (
                F::op("*", vec![var("%t1", "Int"), var("%t2", "Int")]),
                "f(x, \"a b\") * (g(x) + 1)",
            ),
        ];
        for (formula, shown) in cases {
            assert_eq!(walker.show_formula(&formula), shown);
        }
    }
}
