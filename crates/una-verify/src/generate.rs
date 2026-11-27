//! Obligation generation (`spec/12 §3`): a walk over Core IR that keeps the path context Π,
//! and builds the query of each obligation (`spec/12 §4`).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use una_core::flow::Variance;
use una_core::{
    Atom, Base, BinOp, BitSetOp, Bound, CapOp, Clause, Ctor, DataVariant, DecimalOp, Decl, Expr,
    FixedOp, FloatOp, Fun, HashMapOp, HashSetOp, IntType, LoopSource, MapOp, Name, Path, Prim,
    PriorityOp, Profile, Program, Refinement, Rhs, SeqOp, SetOp, ShapeFun, Source, Term, Type, Use,
    print_term_with,
};
use una_diag::ObligationKind;
use una_syntax::{Interner, KeywordSet, SourceDb, Span, TokenKind, lex_in};

use crate::run::Problem;
use crate::smt::{
    ARRAY, Combine, Const, ConstRole, Definition, F, FLOAT32, FLOAT64, Fun as SmtFun, FunRole,
    HASHED, LIST, LU, MAP, MATRIX, Query, REPLY_RIGHT, Sort, TENSOR, TEXT, UNIT, kind_sort,
    path_selector,
};
use crate::written::shown as written_text;

/// How the source text of an obligation's goal and hypotheses is normalized: by the edition of
/// the package, whose canonical form decides the layout of that text (`spec/12 §8`, ADR 0125,
/// amendment of 2026-10-25).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceLayout {
    /// Editions 0.1 to 0.3, whose canonical form keeps each logical line without comments on
    /// one line: the goal's white space collapses to single spaces, and the hypotheses are
    /// shown as written, exactly as before soft wrapping.
    #[default]
    Joined,
    /// Edition 0.4, whose canonical form wraps long lines: [`unwrap_layout`] normalizes the
    /// goal and each hypothesis, so wrapping a line changes neither an obligation's id nor its
    /// shown text.
    Wrapped,
}

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
    /// shows, with what each one holds (ADR 0071 D3): a refutation is then unknown.
    pub(crate) forgotten: Vec<(String, Collection)>,
    /// The equalities that put the enclosing loops at their first iteration, for the second
    /// query of a refutation that sets loop-fresh values (ADR 0071 D3).
    pub(crate) entry: Vec<F>,
    /// The divisions by a value that is not a literal that each path evaluates before the
    /// obligation, in the order of `paths` (one entry for an obligation at one site): the
    /// divisor and the division. A model that sets one of them to zero is no run (ADR 0054,
    /// amendment of 2026-10-08).
    pub(crate) divisions: Vec<Vec<(F, Span)>>,
    /// For a `contract_lock` whose locked condition is outside the encoding, that condition:
    /// the obligation is then unknown at the lock, never proven by the missing goal (#638).
    /// The checker rejects such a lock as U7002 first, so the conformance suite and the
    /// corpus cross-check fail on any obligation that sets it: it means the checker admitted
    /// a condition that the encoding does not cover.
    pub unencodable: Option<String>,
    /// The full query of an obligation whose query is the cone of `spec/29 §8` with an `F64`
    /// hypothesis dropped, when a refutation of the cone query is not final: the refutation is
    /// then decided by this one (ADR 0114 rule 8.3, "Order").
    pub(crate) full: Option<Query>,
}

/// What a binding that a loop updates holds, for the reason of an obligation the loop leaves
/// unknown: a loop keeps a list's length when it only `set`s its elements (ADR 0071 D2), and
/// keeps nothing else of a map or another value (#404).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Collection {
    /// A list.
    List,
    /// A map.
    Map,
    /// Any other value.
    Other,
}

impl Collection {
    /// What `ty` holds.
    fn of(ty: &Type) -> Collection {
        match ty {
            Type::Base {
                base: Base::Named { module, name, .. },
                ..
            } if is_prelude(module) && name == "List" => Collection::List,
            Type::Base {
                base: Base::Named { module, name, .. },
                ..
            } if is_prelude(module) && name == "Map" => Collection::Map,
            _ => Collection::Other,
        }
    }
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
    /// For an `overflow` obligation, a guard that is valid code and states its goal: the
    /// operand's range for a conversion, the operation on `Int` for a fixed-width type, and
    /// the operation with its operands as written for `Int` (`spec/18 §3`).
    pub guard: Option<String>,
    /// Whether the obligation is an `overflow` one on `Int` in a `systems` package, whose
    /// repairs are those of ADR 0109 rule 3: a guard, a refinement of the operands, a
    /// fixed-width type, or the wrapping or saturating methods of one.
    pub int: bool,
    /// For an `overflow` obligation over the result of a prelude call that states no bound on
    /// it, as `sum`'s, the callee: a refutation sets that result freely, so it is no run, and
    /// the obligation is unknown (ADR 0109 rule 3).
    pub unbounded: Option<String>,
    /// For an obligation unknown by construction, its reason: no query is sent. The `overflow`
    /// obligation of `abs` or `sum` used as a function value in a `systems` package is one,
    /// since their operands have no bounds at that use (ADR 0109 rule 3).
    pub unknown: Option<String>,
    /// For an `effect_condition` obligation, the conditioned operation and where it is
    /// charged, which decide its message and repairs (ADR 0092 rules 3 and 5).
    pub effect: Option<Effect>,
}

/// The conditioned operation of an `effect_condition` obligation (ADR 0092 rule 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    /// The operation through the caller's binding, as `@catalog.journal`.
    pub operation: String,
    /// The caller's condition on it, as written.
    pub condition: String,
    /// Where the obligation is charged.
    pub at: EffectAt,
    /// At a call or a `with`, the edit that writes the condition on the callee or handler:
    /// its range and replacement, kept when condition (f) holds (ADR 0092, amendment of
    /// 2026-10-14). At a lock, the edit that restores the locked condition in the header, kept
    /// when condition (c) holds (amendment of 2026-10-15). The report applies (h), or (e).
    pub edit: Option<(Span, String)>,
}

/// Where an `effect_condition` obligation is charged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectAt {
    /// The operation itself.
    Operation,
    /// A call of the named function, which receives the binding.
    Call(String),
    /// A `with` that installs the named handler, which uses the binding.
    With(String),
    /// A call of a function value, whose type has no conditions.
    Value,
    /// A `contract_lock` obligation: the header of the named function against its lock in
    /// `una.toml` (ADR 0092 rule 8, as amended).
    Lock(String),
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
    /// The label of its declared type, deeply, joined with its owner's for a field or a
    /// length (ADR 0078).
    pub(crate) label: una_core::Label,
}

/// How a value shows.
#[derive(Clone, Debug)]
pub(crate) enum Show {
    /// An `Int` or a `Bool`, as the solver writes it.
    Value,
    /// A `Decimal[s]`, its coefficient written with s digits after the point (ADR 0120).
    Decimal(u8),
    /// A value of a sum, by the variant its tag names.
    Variant { tag: F, names: Vec<String> },
    /// Another value, as `<Kind>#<n>`.
    Opaque(String),
}

/// The obligations of `program`, in the order of `spec/12 §5`: file, start offset, kind, with
/// the source text of their goals and hypotheses normalized by `layout`, the edition's.
///
/// # Errors
///
/// [`Problem::UnresolvedKind`] when Core IR names a kind that no declaration has: a lookup
/// that finds nothing would drop a fact or skip a check, so generation fails closed (#407).
pub fn obligations(
    program: &Program,
    sources: &SourceDb,
    layout: SourceLayout,
) -> Result<Vec<Obligation>, Problem> {
    let index = Index::new(program);
    let mut all = Vec::new();
    let mut spans: Vec<Span> = program
        .modules
        .iter()
        .flat_map(|module| module.constants.iter().map(|check| check.span))
        .collect();
    spans.sort_by_key(|s| (s.file, s.start, s.end));
    let constant_values: std::rc::Rc<[Span]> = std::rc::Rc::from(spans);
    for module in &program.modules {
        for decl in &module.decls {
            let mut walker = Walker::new(&index, sources, &module.path, layout);
            walker.constant_values = constant_values.clone();
            match decl {
                Decl::Fun(fun) => walker.function(fun),
                Decl::Handler {
                    name,
                    params,
                    capability_path,
                    ops,
                    uses,
                    ..
                } => {
                    for op in ops {
                        walker.declaration = format!("{}.{name}.{}", module.path, op.name);
                        walker.lambdas.clear();
                        // Each operation's definitions are its own (ADR 0114 rule 8.3).
                        walker.let_defs.clear();
                        walker.handler_op(params, uses, capability_path, op);
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
        // ADR 0126 rule 7: the obligations of each module constant's declared type, at its
        // value, with no hypothesis: the value is closed.
        for check in &module.constants {
            let mut walker = Walker::new(&index, sources, &module.path, layout);
            walker.constant_values = constant_values.clone();
            walker.in_constant = true;
            walker.declaration = format!("{}.{}", module.path, check.name);
            walker.lambdas.clear();
            let mut leaves = Vec::new();
            walker.walk(&check.body, &Ctx::default(), &mut leaves);
            all.extend(walker.obligations);
        }
        for example in &module.examples {
            let mut walker = Walker::new(&index, sources, &module.path, layout);
            walker.constant_values = constant_values.clone();
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
    if let Some((module, name)) = index.unresolved().into_iter().next() {
        return Err(Problem::UnresolvedKind { module, name });
    }
    all.sort_by_key(|o| (o.source.file, o.source.start, o.kind));
    Ok(all)
}

/// Whether `span` lies inside one of `values`, the sorted spans of the values of module
/// constants, which do not overlap.
fn inside_constant_value(values: &[Span], span: Span) -> bool {
    let at = values.partition_point(|s| (s.file, s.start) <= (span.file, span.start));
    at.checked_sub(1)
        .and_then(|i| values.get(i))
        .is_some_and(|s| s.file == span.file && s.start <= span.start && span.end <= s.end)
}

/// For each `ensures` clause of a public function, a query without any hypothesis, its
/// parameters and `result` unconstrained: a clause the solver proves this way is trivial, a
/// missing contract (`spec/13 §6`). Their text is normalized by `layout`, the edition's.
pub fn trivial_ensures(
    program: &Program,
    sources: &SourceDb,
    layout: SourceLayout,
) -> Vec<Obligation> {
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
            let mut walker = Walker::new(&index, sources, &module.path, layout);
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
    /// The `uses` of each function and handler, with the conditions on their operations
    /// (ADR 0092).
    uses: BTreeMap<(&'p str, &'p str), &'p [Use]>,
    /// The `uses of` parameters of each function (ADR 0062).
    uses_of: BTreeMap<(&'p str, &'p str), &'p [Name]>,
    /// The functions of the standard library, native and without `uses` of their own: a
    /// parameter of function type is the only way their calls perform effects.
    library: BTreeSet<(&'p str, &'p str)>,
    kinds: BTreeMap<(&'p str, &'p str), Kind<'p>>,
    /// The program's kinds for the variance of refined flows (ADR 0103 rule 4), shared with
    /// the type checker's pending sites.
    flow_kinds: una_core::flow::Kinds<'p>,
    policies: BTreeMap<(&'p str, &'p str), (&'p Type, &'p Type)>,
    /// The `to` type of each policy, by its `via` function.
    vias: BTreeMap<(&'p str, &'p str), Vec<&'p Type>>,
    /// The kinds looked up that resolve to nothing (#407): [`obligations`] stops on them.
    unresolved: RefCell<BTreeSet<(String, String)>>,
    /// The edits of callees' conditions, by site, caller's binding and operation (ADR 0092,
    /// amendment of 2026-10-14).
    condition_edits: BTreeMap<(Span, &'p str, &'p str), &'p una_core::ConditionEdit>,
    /// The package's profile, as the program records it: in `systems`, `Int` arithmetic has
    /// `overflow` obligations (ADR 0109 rules 3 and 6).
    profile: Profile,
    /// Whether a map's update keeps its other keys, in edition `0.4` (ADR 0069, amendment of
    /// 2026-10-19; ADR 0119 V3).
    map_frames: bool,
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

/// The prefix of the sorts of `Stack[T]`, `i!Stack_<T>` (ADR 0119 V1).
const STACK: &str = "i!Stack_";

/// The prefix of the sorts of `Queue[T]`, `i!Queue_<T>` (ADR 0119 V1).
const QUEUE: &str = "i!Queue_";

/// The sort of `BitSet`, `i!BitSet` (ADR 0119 V1): it has no type argument.
const BITSET: &str = "i!BitSet";

/// The prefix of the sorts of `Set[T]`, `i!Set_<T>` (ADR 0119 V1).
const SET: &str = "i!Set_";

/// The prefix of the sorts of `HashMap[K, V]`, `i!HashMap_<K>_<V>` (ADR 0119 V1).
const HASH_MAP: &str = "i!HashMap_";

/// The prefix of the sorts of `HashSet[T]`, `i!HashSet_<T>` (ADR 0119 V1).
const HASH_SET: &str = "i!HashSet_";

/// Whether `sort` is the sort of a collection with a membership `i!has!<Sort>`: a map, and a
/// `Set`, a `HashSet`, a `HashMap` or a `BitSet` (ADR 0119 V1).
fn has_membership(sort: &str) -> bool {
    sort.starts_with(MAP)
        || sort.starts_with(SET)
        || sort.starts_with(HASH_SET)
        || sort.starts_with(HASH_MAP)
        || sort == BITSET
}

/// Whether the members of a collection of `sort` are items, written `has(item:)`, rather than
/// keys, written `has(key:)`: a `Set`, a `HashSet` and a `BitSet` (`spec/28`).
fn has_items(sort: &str) -> bool {
    sort.starts_with(SET) || sort.starts_with(HASH_SET) || sort == BITSET
}

/// `Option[T]` of the prelude.
fn option_type(item: Type) -> Type {
    Type::Base {
        base: Base::Named {
            module: una_core::PRELUDE_MODULE.to_owned(),
            name: "Option".to_owned(),
            args: vec![item],
        },
        refinements: Vec::new(),
        label: una_core::Label::public(),
    }
}

/// The item type of a `Set[T]`, a `HashSet[T]` or a `BitSet` (`Int`) of `std.collections`,
/// or the key type of a `HashMap[K, V]`, refinements included; `None` for any other type.
fn member_type(ty: &Type) -> Option<Type> {
    match ty {
        Type::Base {
            base: Base::Named { module, name, args },
            ..
        } if module == una_core::COLLECTIONS_MODULE => match (name.as_str(), args.as_slice()) {
            ("Set" | "HashSet", [item]) | ("HashMap", [item, _]) => Some(item.clone()),
            ("BitSet", []) => Some(base_type(Base::Int)),
            _ => None,
        },
        _ => None,
    }
}

/// The element type of a `Stack[T]` or a `Queue[T]` of `std.collections`, refinements
/// included; `None` for any other type.
fn seq_element(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Base {
            base: Base::Named { module, name, args },
            ..
        } if module == una_core::COLLECTIONS_MODULE
            && (name == "Stack" || name == "Queue")
            && args.len() == 1 =>
        {
            args.first()
        }
        _ => None,
    }
}

/// The prefix of the sorts of `PriorityQueue[T, P]`, `i!PriorityQueue_<T>_<P>` (ADR 0119 V1).
const PRIORITY_QUEUE: &str = "i!PriorityQueue_";

/// The item and priority types of a `PriorityQueue[T, P]` of `std.collections`, refinements
/// included; `None` for any other type.
fn priority_positions(ty: &Type) -> Option<(&Type, &Type)> {
    match ty {
        Type::Base {
            base: Base::Named { module, name, args },
            ..
        } if module == una_core::COLLECTIONS_MODULE && name == "PriorityQueue" => {
            match args.as_slice() {
                [item, priority] => Some((item, priority)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether (`module`, `name`) is a collection type of `std.collections` (ADR 0119), which Core
/// does not declare: its sort is the encoder's, as a list's is (V1).
fn is_collection(module: &str, name: &str) -> bool {
    module == una_core::COLLECTIONS_MODULE && una_core::Coll::from_name(name).is_some()
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
            uses: BTreeMap::new(),
            uses_of: BTreeMap::new(),
            library: BTreeSet::new(),
            kinds: BTreeMap::new(),
            flow_kinds: una_core::flow::Kinds::new(program),
            policies: BTreeMap::new(),
            vias: BTreeMap::new(),
            unresolved: RefCell::new(BTreeSet::new()),
            map_frames: program.map_frames,
            condition_edits: program
                .condition_edits
                .iter()
                .map(|edit| ((edit.site, edit.binding.as_str(), edit.op.as_str()), edit))
                .collect(),
            profile: program.profile,
        };
        for signature in &program.library {
            index
                .library
                .insert((signature.path.module.as_str(), signature.path.name.as_str()));
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
                        index.uses.insert((path, fun.name.as_str()), &fun.uses);
                        index
                            .uses_of
                            .insert((path, fun.name.as_str()), &fun.uses_of);
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
                    Decl::Handler {
                        name, params, uses, ..
                    } => {
                        index.handlers.insert((path, name.as_str()), params);
                        index.uses.insert((path, name.as_str()), uses);
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
    /// declares never is, whatever its module is called. Every lookup of a kind starts here,
    /// so a kind that resolves to nothing is recorded (#407).
    fn prelude_kind(&self, module: &str, name: &str) -> bool {
        if self.kinds.contains_key(&(module, name)) {
            return false;
        }
        let collection = is_collection(module, name);
        let prelude = is_prelude(module) || collection;
        // The kinds Core does not declare: `prelude_variants` has the variants of some, and
        // the verifier does not look into the others' values, nor into a collection's.
        if !(collection || (prelude && una_core::PRELUDE_ENGINE_KINDS.contains(&name))) {
            self.unresolved
                .borrow_mut()
                .insert((module.to_owned(), name.to_owned()));
        }
        prelude
    }

    /// The kinds looked up that resolve to nothing, in order.
    fn unresolved(&self) -> Vec<(String, String)> {
        self.unresolved.borrow().iter().cloned().collect()
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

/// `ty` with every refinement removed, at every position: what is known of a value whose
/// type was not found (ADR 0103 rule 4).
fn unrefined(ty: &Type) -> Type {
    match ty {
        Type::Base { base, label, .. } => Type::Base {
            base: match base {
                Base::Named { module, name, args } => Base::Named {
                    module: module.clone(),
                    name: name.clone(),
                    args: args.iter().map(unrefined).collect(),
                },
                Base::Tuple(types) => Base::Tuple(types.iter().map(unrefined).collect()),
                other => other.clone(),
            },
            refinements: Vec::new(),
            label: label.clone(),
        },
        Type::Fn {
            params,
            result,
            uses,
            fails,
            label,
        } => Type::Fn {
            params: params.iter().map(unrefined).collect(),
            result: Box::new(unrefined(result)),
            uses: uses.clone(),
            fails: fails.clone(),
            label: label.clone(),
        },
    }
}

/// Whether a refinement is at some position of `ty`, its top included (ADR 0103).
fn has_refinements(ty: &Type) -> bool {
    match ty {
        Type::Base {
            base: Base::Named { args, .. },
            refinements,
            ..
        } => !refinements.is_empty() || args.iter().any(has_refinements),
        Type::Base { refinements, .. } => !refinements.is_empty(),
        Type::Fn { params, result, .. } => {
            params.iter().any(has_refinements) || has_refinements(result)
        }
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
            Base::ReplyRight => REPLY_RIGHT.to_owned(),
            // `(_ FloatingPoint 11 53)`: the encoding is the run-time semantics (`spec/29 §8`).
            Base::F64 => FLOAT64.to_owned(),
            // A decimal is its coefficient, an `Int` (ADR 0120 V2): its queries stay linear.
            Base::Decimal(_) => "Int".to_owned(),
            // `(_ FloatingPoint 8 24)` (ADR 0120 V1).
            Base::F32 => FLOAT32.to_owned(),
            Base::Named { module, name, args } => {
                let args: Vec<Sort> = args.iter().map(sort).collect();
                if is_prelude(module) || is_collection(module, name) {
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

/// `RNE`, the rounding mode of every operation of `F64` (ADR 0114 rule 3).
fn rne() -> F {
    F::op("RNE", Vec::new())
}

/// A binary floating-point format of the encoding: `F64` or `F32` (ADR 0120 V1).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fmt {
    F64,
    F32,
}

impl Fmt {
    /// The SMT sort.
    fn sort(self) -> &'static str {
        match self {
            Fmt::F64 => FLOAT64,
            Fmt::F32 => FLOAT32,
        }
    }

    /// The defined function of `<` on the format.
    fn lt(self) -> &'static str {
        match self {
            Fmt::F64 => "i!lt!F64",
            Fmt::F32 => "i!lt!F32",
        }
    }

    /// The defined function of `<=` on the format.
    fn le(self) -> &'static str {
        match self {
            Fmt::F64 => "i!le!F64",
            Fmt::F32 => "i!le!F32",
        }
    }

    /// The conversion to the format, signed or unsigned.
    fn to_fp(self, signed: bool) -> &'static str {
        match (self, signed) {
            (Fmt::F64, true) => "(_ to_fp 11 53)",
            (Fmt::F64, false) => "(_ to_fp_unsigned 11 53)",
            (Fmt::F32, true) => "(_ to_fp 8 24)",
            (Fmt::F32, false) => "(_ to_fp_unsigned 8 24)",
        }
    }

    /// The uninterpreted conversion of an `Int` beyond 64 bits.
    fn beyond(self) -> &'static str {
        match self {
            Fmt::F64 => "i!to_f64!Int",
            Fmt::F32 => "i!to_f32!Int",
        }
    }
}

/// The defined functions `i!lt!F64`, `i!le!F64`, `i!lt!F32` and `i!le!F32`: IEEE totalOrder on
/// the canonical values of each format, which the FloatingPoint theory's one NaN and two zeros
/// are (ADR 0114 rule 8.1, ADR 0120 V1). A query holds only those it uses.
fn float_order_definitions() -> BTreeMap<String, Definition> {
    let mut definitions = BTreeMap::new();
    for fmt in [Fmt::F64, Fmt::F32] {
        let local = |name: &str| F::Local {
            name: name.to_owned(),
            sort: fmt.sort().to_owned(),
        };
        let (a, b) = (local("a"), local("b"));
        let lt = F::and(vec![
            F::not(F::op("fp.isNaN", vec![a.clone()])),
            F::or(vec![
                F::op("fp.isNaN", vec![b.clone()]),
                F::op("fp.lt", vec![a.clone(), b.clone()]),
                F::and(vec![
                    F::op("fp.isZero", vec![a.clone()]),
                    F::op("fp.isZero", vec![b.clone()]),
                    F::op("fp.isNegative", vec![a.clone()]),
                    F::op("fp.isPositive", vec![b.clone()]),
                ]),
            ]),
        ]);
        let params = vec![
            ("a".to_owned(), fmt.sort().to_owned()),
            ("b".to_owned(), fmt.sort().to_owned()),
        ];
        let le = F::or(vec![
            float_order(fmt, fmt.lt(), a.clone(), b.clone()),
            F::eq(a, b),
        ]);
        definitions.insert(
            fmt.lt().to_owned(),
            Definition {
                params: params.clone(),
                body: lt,
            },
        );
        definitions.insert(fmt.le().to_owned(), Definition { params, body: le });
    }
    definitions
}

/// An application of the order function `name` of the format `fmt`.
fn float_order(fmt: Fmt, name: &str, a: F, b: F) -> F {
    F::App {
        fun: SmtFun {
            name: name.to_owned(),
            args: vec![fmt.sort().to_owned(), fmt.sort().to_owned()],
            result: "Bool".to_owned(),
            role: FunRole::Predicate,
        },
        args: vec![a, b],
    }
}

/// The term of an operation of `F64` of the refinement language on `args`, the receiver first
/// (ADR 0114 rule 8.1); `None` for a code-only method, whose result is a fresh binder.
fn float_term(fmt: Fmt, op: FloatOp, args: &[F]) -> Option<F> {
    let arg = |i: usize| args.get(i).cloned();
    let rounded =
        |name: &'static str| -> Option<F> { Some(F::op(name, vec![rne(), arg(0)?, arg(1)?])) };
    let unary = |name: &'static str| -> Option<F> { Some(F::op(name, vec![arg(0)?])) };
    let binary = |name: &'static str| -> Option<F> { Some(F::op(name, vec![arg(0)?, arg(1)?])) };
    Some(match op {
        FloatOp::Add => rounded("fp.add")?,
        FloatOp::Sub => rounded("fp.sub")?,
        FloatOp::Mul => rounded("fp.mul")?,
        FloatOp::Div => rounded("fp.div")?,
        FloatOp::Neg => unary("fp.neg")?,
        FloatOp::Abs => unary("fp.abs")?,
        FloatOp::Sqrt => F::op("fp.sqrt", vec![rne(), arg(0)?]),
        // SMT `=` on `Float64` is identity, exactly `==` (ADR 0114 rule 5.1).
        FloatOp::Eq => F::eq(arg(0)?, arg(1)?),
        FloatOp::Ne => F::not(F::eq(arg(0)?, arg(1)?)),
        FloatOp::Lt => float_order(fmt, fmt.lt(), arg(0)?, arg(1)?),
        FloatOp::Le => float_order(fmt, fmt.le(), arg(0)?, arg(1)?),
        FloatOp::Gt => float_order(fmt, fmt.lt(), arg(1)?, arg(0)?),
        FloatOp::Ge => float_order(fmt, fmt.le(), arg(1)?, arg(0)?),
        // The conversions between the formats, `convertFormat` in RNE (ADR 0120 V1).
        FloatOp::ToF32 if fmt == Fmt::F64 => F::op(Fmt::F32.to_fp(true), vec![rne(), arg(0)?]),
        FloatOp::ToF64 if fmt == Fmt::F32 => F::op(Fmt::F64.to_fp(true), vec![rne(), arg(0)?]),
        FloatOp::ToF32 | FloatOp::ToF64 => return None,
        FloatOp::IeeeEq => binary("fp.eq")?,
        FloatOp::IeeeNe => F::not(binary("fp.eq")?),
        FloatOp::IeeeLt => binary("fp.lt")?,
        FloatOp::IeeeLe => binary("fp.leq")?,
        FloatOp::IeeeGt => binary("fp.gt")?,
        FloatOp::IeeeGe => binary("fp.geq")?,
        FloatOp::IsNan => unary("fp.isNaN")?,
        FloatOp::IsFinite => F::not(F::or(vec![unary("fp.isNaN")?, unary("fp.isInfinite")?])),
        FloatOp::IsInfinite => unary("fp.isInfinite")?,
        FloatOp::IsSignNegative => {
            F::and(vec![unary("fp.isNegative")?, F::not(unary("fp.isNaN")?)])
        }
        FloatOp::IsZero => unary("fp.isZero")?,
        FloatOp::IsPositiveZero => F::and(vec![unary("fp.isZero")?, unary("fp.isPositive")?]),
        FloatOp::IsNegativeZero => F::and(vec![unary("fp.isZero")?, unary("fp.isNegative")?]),
        FloatOp::Fma
        | FloatOp::Rem
        | FloatOp::Floor
        | FloatOp::Ceil
        | FloatOp::Trunc
        | FloatOp::RoundHalfEven
        | FloatOp::RoundHalfAway
        | FloatOp::CopySign
        | FloatOp::IeeeMin
        | FloatOp::IeeeMax
        | FloatOp::Clamp
        | FloatOp::ToBits
        | FloatOp::ToIntExact
        | FloatOp::FromInt
        | FloatOp::ToText => return None,
    })
}

/// The fixed-width type of an integer value, `None` for `Int`.
fn integer_type(ty: &Type) -> Option<IntType> {
    match ty {
        Type::Base {
            base: Base::Fixed(int),
            ..
        } => Some(*int),
        _ => None,
    }
}

/// `n.to_f64()` (ADR 0114 rule 8.1) or `n.to_f32()` (ADR 0120 V1): through a bit-vector of
/// the integer type's own width, signed or unsigned, exact for every value of the type, and
/// rounded once to the format; for an `Int`, interpreted for `-2^63 <= n < 2^63` and the
/// uninterpreted `i!to_f64!Int` or `i!to_f32!Int` beyond.
///
/// The narrow form applies only where `n` lies in its type's range: `int2bv w` takes
/// `n mod 2^w`, and a term the language computes exactly (the body of a `reflect` predicate, a
/// clause, an argument of `Int` given to a fixed-width parameter, `spec/18 §2` rule 4) may lie
/// outside it although its static type is fixed-width. So the term is
/// `(ite (in range) <narrow> <the Int form>)`: exact for every integer, and the narrow form, the
/// one the solver decides fast, wherever the range holds.
///
/// `guarded` is false only for a conversion in code (`FloatOp::FromInt` as a primitive), whose
/// operand is a value of its type: a parameter, a field, or the result of an operation with an
/// `overflow` obligation (`spec/18 §3`), so it lies in the range by construction, and the
/// narrow form is exact unguarded (a program whose obligation fails is rejected).
fn to_float(fmt: Fmt, n: F, from: Option<IntType>, guarded: bool) -> Option<F> {
    let through = |width: u32, signed: bool, n: F| -> F {
        let int2bv: &'static str = match width {
            8 => "(_ int2bv 8)",
            16 => "(_ int2bv 16)",
            32 => "(_ int2bv 32)",
            _ => "(_ int2bv 64)",
        };
        F::op(fmt.to_fp(signed), vec![rne(), F::op(int2bv, vec![n])])
    };
    let range = |low: i128, high: i128, n: &F| {
        F::and(vec![
            F::op("<=", vec![F::Int(low.to_string()), n.clone()]),
            F::op("<", vec![n.clone(), F::Int(high.to_string())]),
        ])
    };
    let int_form = |n: F| -> F {
        let beyond = F::App {
            fun: SmtFun {
                name: fmt.beyond().to_owned(),
                args: vec!["Int".to_owned()],
                result: fmt.sort().to_owned(),
                role: FunRole::Other,
            },
            args: vec![n.clone()],
        };
        let (low, high) = (-(1_i128 << 63), 1_i128 << 63);
        F::op(
            "ite",
            vec![range(low, high, &n), through(64, true, n.clone()), beyond],
        )
    };
    Some(match from {
        Some(int) if !guarded => through(int.bits(), int.signed(), n),
        Some(int) => {
            let width = int.bits();
            let (low, high) = if int.signed() {
                (-(1_i128 << (width - 1)), 1_i128 << (width - 1))
            } else {
                (0, 1_i128 << width)
            };
            F::op(
                "ite",
                vec![
                    range(low, high, &n),
                    through(width, int.signed(), n.clone()),
                    int_form(n),
                ],
            )
        }
        None => int_form(n),
    })
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
            Base::ReplyRight => "ReplyRight".to_owned(),
            Base::F64 => "F64".to_owned(),
            Base::F32 => "F32".to_owned(),
            Base::Decimal(scale) => format!("Decimal[{scale}]"),
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

/// The measure of a `for until` loop whose body is walked (ADR 0116 V1): the clause, its value
/// at the start of the iteration, the loop's state binders, and where `decreases` is written.
#[derive(Clone)]
struct Measure {
    clause: una_core::Clause,
    start: F,
    state: Vec<Name>,
    decreases: Span,
}

/// Whether `expr`, a loop's body, holds a `stop` of that loop: one outside the loops in it.
fn stops(expr: &Expr) -> bool {
    match expr {
        Expr::Exit { tag, .. } => *tag == una_core::ExitTag::Stop,
        Expr::Let { value, body, .. } => {
            matches!(value.as_ref(), Bound::Expr(inner) if stops(inner)) || stops(body)
        }
        Expr::LetTuple { body, .. }
        | Expr::With { body, .. }
        | Expr::Attempt { body, .. }
        | Expr::Within { body, .. }
        | Expr::Tasks { body, .. }
        | Expr::Quiet { body, .. } => stops(body),
        Expr::If {
            then, otherwise, ..
        } => stops(then) || stops(otherwise),
        Expr::Case {
            arms, otherwise, ..
        } => arms.iter().any(|arm| stops(&arm.body)) || otherwise.as_deref().is_some_and(stops),
        Expr::Handle { body, handler, .. } => stops(body) || stops(handler),
        Expr::Atom { .. }
        | Expr::Fail { .. }
        | Expr::Unreachable { .. }
        | Expr::Loop { .. }
        | Expr::Together { .. }
        | Expr::TogetherFor { .. }
        | Expr::FirstOf { .. }
        | Expr::Start { .. } => false,
    }
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
    /// (ADR 0071 D3). Shared until changed, as `types`.
    entry: std::rc::Rc<Vec<F>>,
    /// The divisions by a value that is not a literal evaluated on this path, with their
    /// divisors (ADR 0054, amendment of 2026-10-08). Shared until changed, as `types`.
    divisions: std::rc::Rc<Vec<(F, Span)>>,
    /// The capability references in scope, innermost last: the binding of the declaration
    /// walked that each one holds, or `None` for one a `with` binds to a handler (ADR 0092
    /// rule 3). Shared until changed, as `types`.
    caps: std::rc::Rc<Vec<(Name, Option<Name>)>>,
}

impl Ctx {
    /// The binding of the declaration walked that `cap` holds here, if any.
    fn binding(&self, cap: &str) -> Option<Name> {
        self.caps
            .iter()
            .rev()
            .find(|(name, _)| name.as_str() == cap)
            .and_then(|(_, binding)| binding.clone())
    }

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
    /// The spans of the values of the program's module constants, sorted, and whether this
    /// walk is a constant's own check (ADR 0126 rules 7 and 8): outside it, an obligation at a
    /// position inside a constant's value is the check's, generated there once.
    constant_values: std::rc::Rc<[Span]>,
    in_constant: bool,
    sources: &'i SourceDb,
    module: &'p str,
    /// How the text of goals and hypotheses is normalized, by the package's edition.
    layout: SourceLayout,
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
    /// The element type of the list that the `together for` being walked makes, from the
    /// binder of its `let`: each task's value flows into it (ADR 0103 rule 3).
    together_element: Option<Type>,
    /// The type of the value of the `first of` being walked, when it is bound by a `let`:
    /// each branch's value flows into it (`spec/27 §6`, as ADR 0103 rule 3 for `together for`).
    first_of_type: Option<Type>,
    /// The name of the named function whose eta-expansion is walked (issue #76).
    eta_name: Option<String>,
    /// The binder that an assignment's refinement obligation is about, which a counterexample
    /// shows as `x (assigned)` (#277).
    assigned: Option<String>,
    /// Whether each state binding of each loop met so far keeps its length (ADR 0071 D1).
    frames: BTreeMap<Span, Vec<bool>>,
    /// The values that loops make fresh, by binder, with their source names (ADR 0071 D3).
    loop_fresh: BTreeMap<String, BTreeSet<String>>,
    /// For each loop whose body is walked, innermost last, its measure when it is a `for
    /// until`: what each `skip` must decrease (ADR 0116 V1).
    measures: Vec<Option<Measure>>,
    /// What each source name of [`Walker::loop_fresh`] holds (#404).
    loop_collections: BTreeMap<String, Collection>,
    /// The binders of loops' final values, with the source names of their components.
    loop_results: BTreeMap<Name, Vec<String>>,
    /// The loop-fresh binders that come from a loop's final value, which no equality of an
    /// enclosing loop's first iteration fixes (ADR 0071 D3).
    unpinned: BTreeSet<String>,
    /// Whether the paths of the return obligation being pushed return such a value.
    returned_unpinned: bool,
    /// The loop-fresh values that the paths of the return obligation being pushed return.
    returned_forgotten: Vec<(String, Collection)>,
    /// The conditions on the operations of the bindings of the declaration walked, by binding
    /// and operation (ADR 0092).
    conditions: BTreeMap<(Name, String), &'p Clause>,
    /// The `uses of` parameters of the function walked (ADR 0062).
    uses_of: BTreeSet<Name>,
    /// The binders bound to a lambda, which is charged where it is defined (ADR 0092 rule 3).
    lambda_binders: BTreeSet<Name>,
    /// The selfified `let`s whose value has an `F64` term, each `y = t`: the definitions of
    /// ADR 0114 rule 8.3, which the cone may drop and a refutation survives.
    let_defs: BTreeSet<F>,
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
    fn new(
        index: &'i Index<'p>,
        sources: &'i SourceDb,
        module: &'p str,
        layout: SourceLayout,
    ) -> Self {
        Walker {
            index,
            constant_values: std::rc::Rc::from(Vec::new()),
            in_constant: false,
            sources,
            module,
            layout,
            declaration: String::new(),
            obligations: Vec::new(),
            definitions: float_order_definitions(),
            temps: BTreeMap::new(),
            defs: BTreeMap::new(),
            within: Within::Other,
            parameters: BTreeSet::new(),
            lambdas: BTreeMap::new(),
            fresh: 0,
            eta: None,
            together_element: None,
            first_of_type: None,
            eta_name: None,
            assigned: None,
            frames: BTreeMap::new(),
            loop_fresh: BTreeMap::new(),
            measures: Vec::new(),
            loop_collections: BTreeMap::new(),
            loop_results: BTreeMap::new(),
            returned_forgotten: Vec::new(),
            conditions: BTreeMap::new(),
            uses_of: BTreeSet::new(),
            lambda_binders: BTreeSet::new(),
            unpinned: BTreeSet::new(),
            returned_unpinned: false,
            let_defs: BTreeSet::new(),
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
        self.let_defs.clear();
        self.within = Within::Function { public: fun.public };
        self.parameters = fun.params.iter().map(|(name, _)| name.clone()).collect();
        let mut ctx = Ctx::default();
        for (name, ty) in &fun.params {
            self.bind(&mut ctx, name, ty);
        }
        self.own_bindings(&mut ctx, &fun.uses);
        self.uses_of = fun.uses_of.iter().cloned().collect();
        self.lambda_binders.clear();
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
        // ADR 0092 rule 8, as amended: the header's condition on each locked operation implies
        // the lock's, over the parameters on entry.
        for lock in &fun.locks {
            self.contract_lock(&ctx, &fun.name, lock);
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
        let text = instantiate(&refinement.clause, "result");
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
        handler_uses: &'p [Use],
        capability: &una_core::Path,
        op: &'p una_core::HandlerOp,
    ) {
        let mut ctx = Ctx::default();
        for (name, ty) in handler_params {
            self.bind(&mut ctx, name, ty);
        }
        self.own_bindings(&mut ctx, handler_uses);
        self.uses_of.clear();
        self.lambda_binders.clear();
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
            let text = instantiate(&refinement.clause, "result");
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
        // ADR 0103 rule 6: the refinements of its nested positions, of the terms that project
        // them, under the tags of their variants.
        self.assume_projections(ctx, value, &[], 8);
    }

    /// The facts on access of ADR 0103 rule 6 for the fields of `value` whose instantiated
    /// type is refined: each refinement of a field, applied to the field's term, under `guard`
    /// and, for a sum, the tag of the field's variant. Bounded by `depth`, so a recursive kind
    /// adds a bounded number of facts; linear in the fields of the kinds it reaches.
    fn assume_projections(&mut self, ctx: &mut Ctx, value: &Tv, guard: &[F], depth: usize) {
        let Type::Base {
            base: Base::Named { module, name, args },
            ..
        } = &value.ty
        else {
            return;
        };
        if depth == 0 || !args.iter().any(has_refinements) {
            return;
        }
        let variants = self.index.variant_names(module, name).len().max(1);
        let tag = if variants > 1 { self.tag(value) } else { None };
        for variant in 0..variants {
            let mut guarded = guard.to_vec();
            if let Some(tag) = &tag {
                guarded.push(F::eq(tag.clone(), F::Int(variant.to_string())));
            }
            for index in 0.. {
                let Some(field) = self.field(value, variant, index) else {
                    break;
                };
                // A field of a generic kind is encoded at its parameter's sort, which a
                // refinement of the argument does not speak of: no fact about it or below it
                // (sound, incomplete).
                if !has_refinements(&field.ty) || field.f.sort() != sort(&field.ty) {
                    continue;
                }
                for refinement in refinements(&field.ty) {
                    let env = Env {
                        vars: &|_| None,
                        it: Some(field.clone()),
                        result: None,
                    };
                    if let Some(f) = self.term(&refinement.clause.term, &env) {
                        let fact = if guarded.is_empty() {
                            f.f
                        } else {
                            F::op("=>", vec![F::and(guarded.clone()), f.f])
                        };
                        ctx.define(fact);
                    }
                }
                self.assume_projections(ctx, &field, &guarded, depth - 1);
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
                    on_stop,
                    span,
                } => {
                    // `on stop` runs with the state fresh at its declared types (ADR 0116 V3):
                    // its leaves give no fact, as the loop's value gives none.
                    if let Some(on_stop) = on_stop {
                        let mut stop_ctx = ctx.clone();
                        for (name, ty) in &on_stop.state {
                            self.bind(&mut stop_ctx, name, ty);
                        }
                        let mut stop_leaves = Vec::new();
                        self.walk(&on_stop.body, &stop_ctx, &mut stop_leaves);
                    }
                    // `for ever within d` (`spec/26 §7`, ADR 0116 rule 11): `d >= 1`, a
                    // `refinement` obligation at `d`, in the context before the loop.
                    if let LoopSource::Ever {
                        within: Some((deadline, deadline_span)),
                    } = source.as_ref()
                    {
                        let value = self.atom(deadline, &ctx);
                        let goal = (value.f.sort() == "Int")
                            .then(|| F::op(">=", vec![value.f, F::Int("1".to_owned())]));
                        let text = format!("{} >= 1", self.text(*deadline_span));
                        self.site(
                            ObligationKind::Refinement,
                            &ctx,
                            goal,
                            text,
                            *deadline_span,
                            None,
                        );
                    }
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
                        self.loop_collections
                            .insert(source_name(name), Collection::of(ty));
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
                        // The shape frame of ADR 0120 V4: a matrix's rows and columns too.
                        let (now, then) = (self.shape_terms(&x), self.shape_terms(&entry));
                        if now.len() == then.len() {
                            for (now, then) in now.into_iter().zip(then) {
                                body_ctx.assume(F::eq(now, then));
                            }
                        }
                    }
                    // `for until c decreases m` (ADR 0116 V1): `c` is false in the body when it
                    // is in the refinement language, and `m` is at least 0 there.
                    let measure = match source.as_ref() {
                        LoopSource::Until {
                            holds,
                            measure,
                            decreases,
                            ..
                        } => {
                            let lookup = |name: &str| {
                                body_ctx
                                    .types
                                    .contains_key(name)
                                    .then(|| own(&body_ctx, name))
                            };
                            let env = Env {
                                vars: &lookup,
                                it: None,
                                result: None,
                            };
                            let holds = holds
                                .as_ref()
                                .and_then(|holds| self.term(&holds.term, &env));
                            let start = self.term(&measure.term, &env);
                            if let Some(holds) = holds {
                                body_ctx.assume(F::not(holds.f));
                            }
                            let goal = start
                                .as_ref()
                                .map(|m| F::op(">=", vec![m.f.clone(), F::Int("0".to_owned())]));
                            self.site(
                                ObligationKind::DecreasesBound,
                                &body_ctx,
                                goal,
                                measure.text.clone(),
                                *decreases,
                                None,
                            );
                            start.map(|start| Measure {
                                clause: measure.as_ref().clone(),
                                start: start.f,
                                state: state.iter().map(|(name, _)| name.clone()).collect(),
                                decreases: *decreases,
                            })
                        }
                        _ => None,
                    };
                    self.measures.push(measure.clone());
                    let mut body_leaves = Vec::new();
                    self.walk(body, &body_ctx, &mut body_leaves);
                    self.measures.pop();
                    // Each path that ends the body goes on to the next iteration: `m` is less
                    // there than at the start (`decreases_step`).
                    if let Some(measure) = &measure {
                        for leaf in &body_leaves {
                            let Some(tuple) = &leaf.value else {
                                continue;
                            };
                            let mut leaf_ctx = body_ctx.clone();
                            leaf_ctx.hyps = leaf.hyps.clone();
                            leaf_ctx.divisions = leaf.divisions.clone();
                            let components: Vec<Option<Tv>> = (0..measure.state.len())
                                .map(|index| projection(tuple, index))
                                .collect();
                            self.decreases_step(measure, &leaf_ctx, &components);
                        }
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
                    let element = self.together_element.take();
                    let mut task_leaves = Vec::new();
                    self.walk(task, &task_ctx, &mut task_leaves);
                    // ADR 0103 rule 3: each task's value is an element of the list.
                    if let Some(element) = element.filter(has_refinements) {
                        for leaf in &task_leaves {
                            let Some(value) = &leaf.value else {
                                continue;
                            };
                            let mut leaf_ctx = task_ctx.clone();
                            leaf_ctx.hyps = leaf.hyps.clone();
                            self.refined_flow(&element, value, leaf.span, &leaf_ctx);
                        }
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
                // `first_of(e₁ | ... | eₙ)` (`spec/27 §6`, ADR 0117 Verification): each branch
                // is verified alone, in the context before the block, and each of its values
                // flows into the type of the result, which carries the facts of that type only:
                // which branch wins is timing.
                Expr::FirstOf { tasks, span } => {
                    let result = self.first_of_type.take().filter(has_refinements);
                    for task in tasks {
                        let mut task_leaves = Vec::new();
                        self.walk(task, &ctx, &mut task_leaves);
                        let Some(result) = &result else {
                            continue;
                        };
                        for leaf in &task_leaves {
                            let Some(value) = &leaf.value else {
                                continue;
                            };
                            let mut leaf_ctx = ctx.clone();
                            leaf_ctx.hyps = leaf.hyps.clone();
                            self.refined_flow(result, value, leaf.span, &leaf_ctx);
                        }
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
                // `tasks(b) e` (`spec/27 §7`, ADR 0117 Verification): `b >= 1`, a `refinement`
                // obligation at `b`; the block runs in the current task, so it is walked as a
                // block, and its `start`s change no outer binding.
                Expr::Tasks { bound, body, .. } => {
                    if let Some((bound, bound_span)) = bound {
                        let value = self.atom(bound, &ctx);
                        let goal = (value.f.sort() == "Int")
                            .then(|| F::op(">=", vec![value.f, F::Int("1".to_owned())]));
                        let text = format!("{} >= 1", self.text(*bound_span));
                        self.site(
                            ObligationKind::Refinement,
                            &ctx,
                            goal,
                            text,
                            *bound_span,
                            None,
                        );
                    }
                    expr = body;
                }
                // `start e`: `e` is verified alone, in the context of the `start`, as a
                // `together` task is; its value is `unit`, and it gives no fact.
                // `quiet e` is `e` for verification (S-39): only the trace differs.
                Expr::Quiet { body, .. } => expr = body,
                Expr::Start { task, span, .. } => {
                    let mut task_leaves = Vec::new();
                    self.walk(task, &ctx, &mut task_leaves);
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
                    // A `quiet` expression is its body to the verifier (S-39): the facts of a
                    // task construct it marks are those of the construct.
                    let bound = match value.as_ref() {
                        Bound::Rhs(rhs) => Err(rhs),
                        Bound::Expr(inner) => {
                            let mut inner = inner;
                            while let Expr::Quiet { body, .. } = inner {
                                inner = body;
                            }
                            Ok(inner)
                        }
                    };
                    match bound {
                        Err(rhs) => self.rhs(name, ty, rhs, *span, &mut ctx),
                        // The tuple of a `together` block (`spec/19 §4`): each task is checked
                        // in the context before the block, with none of its siblings' facts,
                        // and each component of the tuple is the value of one of its task's
                        // paths, so each branch's binding carries its facts after the join.
                        Ok(Expr::Together { tasks, .. }) => {
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
                        Ok(inner) => {
                            if let Expr::TogetherFor { .. } = inner
                                && let Type::Base {
                                    base: Base::Named { args, .. },
                                    ..
                                } = ty
                            {
                                self.together_element = args.first().cloned();
                            }
                            if let Expr::FirstOf { .. } = inner {
                                self.first_of_type = Some(ty.clone());
                            }
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
                                source,
                                state,
                                init,
                                body,
                                span: loop_span,
                                ..
                            } = inner
                            {
                                // ADR 0116 V3: after a `for until` with no `stop`, `c` holds.
                                if let LoopSource::Until {
                                    holds: Some(holds), ..
                                } = source.as_ref()
                                    && !stops(body)
                                {
                                    let lookup = |name: &str| match state
                                        .iter()
                                        .position(|(state, _)| state == name)
                                    {
                                        Some(index) => projection(&x, index),
                                        None => {
                                            ctx.types.contains_key(name).then(|| own(&ctx, name))
                                        }
                                    };
                                    let env = Env {
                                        vars: &lookup,
                                        it: None,
                                        result: None,
                                    };
                                    if let Some(holds) = self.term(&holds.term, &env) {
                                        ctx.assume(holds.f);
                                    }
                                }
                                let preserved = self.preserved(*loop_span, state, body);
                                let sources = state.iter().map(|(name, _)| source_name(name));
                                self.loop_results.insert(name.clone(), sources.collect());
                                for (name, ty) in state {
                                    self.loop_collections
                                        .insert(source_name(name), Collection::of(ty));
                                }
                                for (index, ((_, _), atom)) in state.iter().zip(init).enumerate() {
                                    let Some(component) = projection(&x, index) else {
                                        continue;
                                    };
                                    if !preserved.get(index).copied().unwrap_or(false) {
                                        continue;
                                    }
                                    let entry = self.atom(atom, &ctx);
                                    let (now, then) =
                                        (self.shape_terms(&component), self.shape_terms(&entry));
                                    if now.len() == then.len() {
                                        for (now, then) in now.into_iter().zip(then) {
                                            ctx.define(F::eq(now, then));
                                        }
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
                            let text = instantiate(&refinement.clause, &value_text);
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
                    self.declared_fields(ctor, args, arg_spans, &ctx);
                    return;
                }
                // `skip` or `stop`: the path leaves the body with the state, whose refinements
                // its rebinds already proved (ADR 0116 V2); the loop's value says nothing but
                // their types.
                Expr::Exit { tag, state, .. } => {
                    if *tag == una_core::ExitTag::Next
                        && let Some(Some(measure)) = self.measures.last().cloned()
                    {
                        let components: Vec<Option<Tv>> = state
                            .iter()
                            .map(|atom| Some(self.atom(atom, &ctx)))
                            .collect();
                        self.decreases_step(&measure, &ctx, &components);
                    }
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
                // `e within d` (`spec/27 §5`, ADR 0117 rule 5): `d >= 1`, a `refinement`
                // obligation at `d`; on success the facts of `e` hold as without the deadline,
                // and a time-out carries none, as any failure.
                Expr::Within {
                    deadline,
                    deadline_span,
                    body,
                    ..
                } => {
                    let value = self.atom(deadline, &ctx);
                    let goal = (value.f.sort() == "Int")
                        .then(|| F::op(">=", vec![value.f, F::Int("1".to_owned())]));
                    let text = format!("{} >= 1", self.text(*deadline_span));
                    self.site(
                        ObligationKind::Refinement,
                        &ctx,
                        goal,
                        text,
                        *deadline_span,
                        None,
                    );
                    expr = body;
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
                Expr::With { cap, source, body } => {
                    let holds = match source {
                        Source::Rebind(other) => ctx.binding(other),
                        Source::Handler {
                            handler,
                            args,
                            arg_spans,
                            span,
                            ..
                        } => {
                            let key = (handler.module.as_str(), handler.name.as_str());
                            if let Some(params) = self.index.handlers.get(&key).copied() {
                                self.arguments(params, args, arg_spans, &ctx);
                                // ADR 0092 rule 3: the handler's bindings receive the
                                // context's, and it is charged where it is installed.
                                let values: Vec<Tv> =
                                    args.iter().map(|a| self.atom(a, &ctx)).collect();
                                self.effect_call(
                                    &ctx,
                                    key,
                                    params,
                                    &values,
                                    *span,
                                    &EffectAt::With(handler.name.clone()),
                                );
                            } else if key == (una_core::LIMIT_MODULE, una_core::LIMITER) {
                                // `limiter(at_most, per, waiting)` of `std.limit` (ADR 0117
                                // rule 9, `spec/27 §10`): the engines provide it, so Core
                                // declares no handler; its installation carries
                                // `at_most >= 1`, `per >= 1` and `waiting >= 0`, a
                                // `refinement` obligation at each argument.
                                let bounds = ["1", "1", "0"];
                                for ((arg, arg_span), bound) in
                                    args.iter().zip(arg_spans).zip(bounds)
                                {
                                    let value = self.atom(arg, &ctx);
                                    let goal = (value.f.sort() == "Int").then(|| {
                                        F::op(">=", vec![value.f, F::Int(bound.to_owned())])
                                    });
                                    let text = format!("{} >= {bound}", self.text(*arg_span));
                                    self.site(
                                        ObligationKind::Refinement,
                                        &ctx,
                                        goal,
                                        text,
                                        *arg_span,
                                        None,
                                    );
                                }
                            }
                            None
                        }
                    };
                    std::rc::Rc::make_mut(&mut ctx.caps).push((cap.clone(), holds));
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
        // ADR 0103 rule 4: the value flows into the binder's nested positions, as an annotated
        // `let`, an assignment or an `edit` argument's rebind binds it. A binder at the value's
        // own type gives none. A list literal's elements are its own flows (rule 3). The
        // refinements of either side count: a contravariant or invariant position proves the
        // value's.
        if let Some(source) = self.rhs_type(rhs, ty, ctx)
            && (has_refinements(ty) || has_refinements(&source))
        {
            let what = self.text(span);
            self.nested_flows(&source, ty, span, &what);
        }
        std::rc::Rc::make_mut(&mut ctx.types).insert(name.to_owned(), ty.clone());
        let x = own(ctx, name);
        match rhs {
            Rhs::Atom(atom) => {
                // ADR 0109 rule 3: an `Int` literal outside the tagged range, which
                // elaboration binds at its span in a `systems` package.
                self.literal_site(ty, atom, span, ctx);
                let value = self.atom(atom, ctx);
                if value.f.sort() == x.f.sort() {
                    let definition = F::eq(x.f.clone(), value.f.clone());
                    if definition.mentions_float() {
                        self.let_defs.insert(definition.clone());
                    }
                    ctx.define(definition);
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
                self.decimal_sites(*prim, args, &values, arg_spans, *span, ctx);
                self.fixed_sites(ty, *prim, args, &values, arg_spans, *span, ctx);
                if let Prim::Fixed(op, int) = prim {
                    self.fixed_facts(*op, *int, args, &values, &x.f, ctx);
                }
                if let Prim::Map(op) = prim {
                    // ADR 0103 rules 3 and 5: `with` and `set` store the key and the value at
                    // the map's types; a lookup stores nothing, so its key is at its base.
                    if *op == MapOp::With
                        && let Some(Type::Base {
                            base: Base::Named { args: kv, .. },
                            ..
                        }) = values.first().map(|map| &map.ty)
                    {
                        let kv = kv.clone();
                        for (index, target) in kv.iter().enumerate() {
                            if let Some(value) = values.get(index + 1)
                                && has_refinements(target)
                            {
                                let at = arg_spans.get(index + 1).copied().unwrap_or(*span);
                                self.refined_flow(target, value, at, ctx);
                            }
                        }
                    }
                    self.map_facts(*op, &values, &x, ctx);
                }
                if let Prim::Seq(op) = prim {
                    // ADR 0119 rule 1 and ADR 0103 rule 3: a push stores its item at the
                    // element type, a flow site with its `refinement` obligations; the other
                    // operations store nothing.
                    if matches!(op, SeqOp::PushFront | SeqOp::PushBack)
                        && let Some(Type::Base {
                            base: Base::Named { args: element, .. },
                            ..
                        }) = values.first().map(|seq| &seq.ty)
                        && let (Some(target), Some(value)) =
                            (element.first().cloned(), values.get(1))
                        && has_refinements(&target)
                    {
                        let at = arg_spans.get(1).copied().unwrap_or(*span);
                        self.refined_flow(&target, value, at, ctx);
                    }
                    self.seq_facts(*op, &values, &x, ctx);
                }
                if let Prim::Priority(op) = prim {
                    // ADR 0119 rule 1 and ADR 0103 rule 3: a push stores its item at `T` and its
                    // priority at `P`, each a flow site with its `refinement` obligations; the
                    // other operations store nothing.
                    if *op == PriorityOp::Push
                        && let Some(Type::Base {
                            base:
                                Base::Named {
                                    args: positions, ..
                                },
                            ..
                        }) = values.first().map(|queue| &queue.ty)
                    {
                        let positions = positions.clone();
                        for (index, target) in positions.iter().enumerate() {
                            if let Some(value) = values.get(index + 1)
                                && has_refinements(target)
                            {
                                let at = arg_spans.get(index + 1).copied().unwrap_or(*span);
                                self.refined_flow(target, value, at, ctx);
                            }
                        }
                    }
                    self.priority_facts(*op, &values, &x, ctx);
                }
                // ADR 0119 rule 1 and ADR 0103 rule 3: `add` and `with` store their item at
                // the set's item type, a flow site with its `refinement` obligations; a lookup,
                // `has`, `remove` and `without`, stores nothing.
                if *prim == Prim::Set(SetOp::With)
                    && let Some(Type::Base {
                        base: Base::Named { args: item, .. },
                        ..
                    }) = values.first().map(|set| &set.ty)
                    && let (Some(target), Some(value)) = (item.first().cloned(), values.get(1))
                    && has_refinements(&target)
                {
                    let at = arg_spans.get(1).copied().unwrap_or(*span);
                    self.refined_flow(&target, value, at, ctx);
                }
                // `union` stores the other set's items at the receiver's item type: the other
                // set flows into the receiver's type, with the obligations of its nested
                // positions (ADR 0103 rule 4). The other set operations keep only the
                // receiver's items, so their other set stores nothing.
                if *prim == Prim::Set(SetOp::Union)
                    && let (Some(set), Some(other)) = (values.first(), values.get(1))
                    && let Type::Base {
                        base: base @ Base::Named { .. },
                        label,
                        ..
                    } = &set.ty
                    && has_refinements(&set.ty)
                {
                    let target = Type::Base {
                        base: base.clone(),
                        refinements: Vec::new(),
                        label: label.clone(),
                    };
                    let at = arg_spans.get(1).copied().unwrap_or(*span);
                    self.refined_flow(&target, other, at, ctx);
                }
                // ADR 0119 rule 1 and ADR 0103 rule 3: a hash map's `set`, `with` and the
                // `with` of `update` store the key and the value at the map's types, and a hash
                // set's `add` and `with` store the item, each a flow site with its `refinement`
                // obligations; a lookup stores nothing.
                if matches!(
                    prim,
                    Prim::HashMap(HashMapOp::With) | Prim::HashSet(HashSetOp::With)
                ) && let Some(Type::Base {
                    base: Base::Named { args: stored, .. },
                    ..
                }) = values.first().map(|table| &table.ty)
                {
                    let stored = stored.clone();
                    for (index, target) in stored.iter().enumerate() {
                        if let Some(value) = values.get(index + 1)
                            && has_refinements(target)
                        {
                            let at = arg_spans.get(index + 1).copied().unwrap_or(*span);
                            self.refined_flow(target, value, at, ctx);
                        }
                    }
                }
                // A hash set's `union` stores the other set's items at the receiver's item
                // type, as a set's does.
                if *prim == Prim::HashSet(HashSetOp::Union)
                    && let (Some(set), Some(other)) = (values.first(), values.get(1))
                    && let Type::Base {
                        base: base @ Base::Named { .. },
                        label,
                        ..
                    } = &set.ty
                    && has_refinements(&set.ty)
                {
                    let target = Type::Base {
                        base: base.clone(),
                        refinements: Vec::new(),
                        label: label.clone(),
                    };
                    let at = arg_spans.get(1).copied().unwrap_or(*span);
                    self.refined_flow(&target, other, at, ctx);
                }
                // ADR 0119 V2 to V4: the facts of an operation of a set, a hash set or a hash
                // map.
                if matches!(prim, Prim::Set(_) | Prim::HashSet(_) | Prim::HashMap(_)) {
                    self.table_facts(*prim, &values, &x, ctx);
                }
                // ADR 0119 rule 14 and V2: the range and size preconditions of a bit set, then
                // the facts of its operation.
                if let Prim::BitSet(op) = prim {
                    self.bit_set_sites(*op, &values, arg_spans, *span, ctx);
                    self.bit_set_facts(*op, &values, &x, ctx);
                }
                if let Some(value) = self.prim(*prim, &values)
                    && value.sort() == x.f.sort()
                {
                    let definition = F::eq(x.f.clone(), value.clone());
                    if definition.mentions_float() {
                        self.let_defs.insert(definition.clone());
                    }
                    ctx.define(definition);
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
                // ADR 0092 rule 3: the callee's bindings receive the caller's of the same
                // name, and promise their own conditions.
                if let Some(contract) = self.index.contracts.get(&key) {
                    self.effect_call(
                        ctx,
                        key,
                        contract.params,
                        &values,
                        *span,
                        &EffectAt::Call(function.name.clone()),
                    );
                    let library = self.index.library.contains(&key);
                    let of = self.index.uses_of.get(&key).copied().unwrap_or(&[]);
                    for ((param, _), (atom, value)) in
                        contract.params.iter().zip(args.iter().zip(&values))
                    {
                        // A `uses of` parameter of the function walked is charged wherever it
                        // is passed, since its argument may run any of the bindings.
                        let held = matches!(atom, Atom::Var(name) if self.uses_of.contains(name));
                        if library || of.contains(param) || held {
                            self.passed_function(ctx, atom, value, *span);
                        }
                    }
                }
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
                    // ADR 0103 rule 6: a generic callee's result at refined type arguments
                    // carries their refinements. It returns only values it was given at those
                    // types, each a flow with its obligation, as `fold`'s accumulator is
                    // `initial` or a result of `step`.
                    if !contract.type_params.is_empty() && has_refinements(&result) {
                        let value = Tv {
                            f: x.f.clone(),
                            ty: result.clone(),
                        };
                        self.assume_refinements(ctx, &value);
                    }
                    if is_prelude(&function.module) {
                        self.text_facts(&function.name, &values, &x, ctx);
                        self.dense_facts(&function.name, &values, &x, ctx);
                        // ADR 0109 rule 3: after the callee's `ensures`, which bound `abs`.
                        self.prelude_site(&function.name, &x, arg_spans, *span, ctx);
                    }
                    // ADR 0119 rule 14: `empty_bit_set(size: n)` ensures `result.size == n`
                    // and `result.len == 0`, which its declaration cannot write until V5: facts
                    // after the call's precondition, as its `ensures` would be.
                    if key == (una_core::COLLECTIONS_MODULE, "empty_bit_set")
                        && x.f.sort() == BITSET
                        && let Some(size) = values.first()
                    {
                        ctx.assume(F::eq(size_of(&x.f), size.f.clone()));
                        ctx.assume(F::eq(len_of(&x.f), F::Int("0".to_owned())));
                    }
                    // ADR 0119 rules 5 to 13 and 16: each empty collection `ensures result.len
                    // == 0`, and a conversion to a set or a table `ensures result.len <=
                    // xs.len`, which the declarations do not write.
                    if function.module == una_core::COLLECTIONS_MODULE
                        && let Some(len) = self.len(&x)
                    {
                        match function.name.as_str() {
                            "empty_stack" | "empty_queue" | "empty_set" | "empty_hash_map"
                            | "empty_hash_set" | "lowest_first" | "highest_first" => {
                                ctx.assume(F::eq(len, F::Int("0".to_owned())));
                            }
                            "to_set" | "to_hash_set" | "to_hash_map" => {
                                if let Some(list) = values.first().and_then(|xs| self.len(xs)) {
                                    ctx.assume(F::op("<=", vec![len, list]));
                                }
                            }
                            _ => {}
                        }
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
                span,
            } => {
                let callee_atom = callee;
                let callee = self.atom(callee, ctx);
                // ADR 0092 rule 3, as amended: a call of a function value performs its type's
                // `uses` through the caller's bindings, and a function type has no conditions.
                // A `uses of` parameter's argument runs through any of the bindings.
                if let Atom::Var(name) = callee_atom
                    && self.uses_of.contains(name)
                {
                    self.every_condition(ctx, *span, &EffectAt::Value);
                }
                if let Type::Fn { uses, .. } = &callee.ty {
                    for usage in uses {
                        self.effect_sites(
                            ctx,
                            &usage.cap,
                            &usage.ops,
                            &|_, _| None,
                            *span,
                            &EffectAt::Value,
                        );
                    }
                }
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
                cap,
                capability_path,
                op,
                args,
                arg_spans,
                span,
                ..
            } => {
                let values: Vec<Tv> = args.iter().map(|a| self.atom(a, ctx)).collect();
                // ADR 0092 rule 3: the operation runs where its binding's condition holds.
                self.effect_sites(
                    ctx,
                    cap,
                    std::slice::from_ref(op),
                    &|_, _| None,
                    *span,
                    &EffectAt::Operation,
                );
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
                self.fields(&x.ty, ctor, args, arg_spans, ctx);
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
                // ADR 0103 rule 3: each element flows into a refined element type.
                if let Type::Base {
                    base: Base::Named { args, .. },
                    ..
                } = ty
                    && let Some(element) = args.first()
                    && has_refinements(element)
                {
                    for item in items {
                        let value = self.atom(item, ctx);
                        self.refined_flow(element, &value, span, ctx);
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
                // ADR 0109 rule 3: `abs` or `sum` of the prelude used as a value.
                if let Some(path) = function {
                    self.prelude_value_site(path, span, ctx);
                }
                self.lambda_binders.insert(name.to_owned());
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
                self.declared_fields(ctor, args, arg_spans, ctx);
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
                via,
                arg,
                arg_span,
                span,
            } => {
                // ADR 0092 rule 3: a policy uses as its `via` function does, so the call runs
                // the `via` with the caller's bindings: a call of it that passes them.
                let via_key = (via.module.as_str(), via.name.as_str());
                if let Some(contract) = self.index.contracts.get(&via_key) {
                    let value = self.atom(arg, ctx);
                    self.effect_call(
                        ctx,
                        via_key,
                        contract.params,
                        std::slice::from_ref(&value),
                        *span,
                        &EffectAt::Call(via.name.clone()),
                    );
                }
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
                written_text(clause, &|term| match term {
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
            let text = written_text(clause, &|term| match term {
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

    /// The arguments of a construction of `owner` into its fields, at the kind's type
    /// arguments: one `refinement` obligation per refinement of the field type, and for a
    /// kind without type arguments, the flows of its function fields too; a generic kind's,
    /// the prelude's `Some`, `Ok` and `Err` included, are [`Walker::field_functions`]'s. A
    /// field at a refined type argument, as `Box(value: n)` at `Box[Pos]`, is an obligation
    /// (ADR 0103 rule 3).
    fn fields(&mut self, owner: &Type, ctor: &Ctor, args: &[Atom], arg_spans: &[Span], ctx: &Ctx) {
        let generic = match owner {
            Type::Base {
                base: Base::Named { module, name, args },
                ..
            } => self.index.prelude_kind(module, name) || !args.is_empty(),
            Type::Base { .. } | Type::Fn { .. } => false,
        };
        for (index, (atom, span)) in args.iter().zip(arg_spans).enumerate() {
            let Some(field_ty) = self.index.field_type(owner, ctor.index, index) else {
                continue;
            };
            let value = self.atom(atom, ctx);
            if generic {
                self.refined_flow(&field_ty, &value, *span, ctx);
            } else {
                self.refinement_sites(&field_ty, &value, *span, ctx);
            }
        }
    }

    /// The fields of a failure, a kind without type arguments: refinement obligations at
    /// each, at the declared field types.
    fn declared_fields(&mut self, ctor: &Ctor, args: &[Atom], arg_spans: &[Span], ctx: &Ctx) {
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

    /// The type of the value that `rhs` makes, before it is bound, for the flow into the
    /// binder's nested positions (ADR 0103 rule 4). Total over the forms of a right-hand side,
    /// with no catch-all: a new form does not compile until it says what it makes. A form
    /// whose type cannot be found gives the binder's type with every refinement removed, so
    /// each refined nested position of the binder is an obligation with no fact behind it,
    /// refuted or unknown, never skipped. `None` only for the forms whose flows into the
    /// binder are their own, each named below.
    fn rhs_type(&mut self, rhs: &Rhs, binder: &Type, ctx: &Ctx) -> Option<Type> {
        let unknown = || unrefined(binder);
        match rhs {
            Rhs::Atom(atom) => Some(self.atom(atom, ctx).ty),
            Rhs::Tuple(items) => Some(Type::Base {
                base: Base::Tuple(items.iter().map(|item| self.atom(item, ctx).ty).collect()),
                refinements: Vec::new(),
                label: type_label(binder).clone(),
            }),
            Rhs::Prim { prim, args, .. } => Some(match prim {
                Prim::Map(op) => {
                    let receiver = args.first().map(|map| self.atom(map, ctx).ty);
                    match receiver {
                        Some(Type::Base {
                            base: Base::Named { module, name, args },
                            ..
                        }) if is_prelude(&module) && name == "Map" && args.len() == 2 => {
                            let named = |name: &str, args: Vec<Type>| Type::Base {
                                base: Base::Named {
                                    module: module.clone(),
                                    name: name.to_owned(),
                                    args,
                                },
                                refinements: Vec::new(),
                                label: type_label(binder).clone(),
                            };
                            let Ok([key, value]) = <[Type; 2]>::try_from(args) else {
                                return Some(unknown());
                            };
                            match op {
                                MapOp::Has => unknown(),
                                MapOp::Get => named("Option", vec![value]),
                                MapOp::With | MapOp::Without => named("Map", vec![key, value]),
                                MapOp::Keys => named("List", vec![key]),
                                MapOp::Values => named("List", vec![value]),
                                MapOp::Entries => {
                                    named("List", vec![named("Entry", vec![key, value])])
                                }
                            }
                        }
                        _ => unknown(),
                    }
                }
                // ADR 0119 V6: what a priority queue gives back carries its item or priority
                // position's refinements, as an element of a list does (ADR 0103 rule 6), and an
                // update gives a queue of the receiver's type.
                Prim::Priority(op) => {
                    let receiver = args.first().map(|queue| self.atom(queue, ctx).ty);
                    let positions = receiver
                        .as_ref()
                        .and_then(priority_positions)
                        .map(|(item, priority)| (item.clone(), priority.clone()));
                    match (receiver, positions) {
                        (Some(queue), Some((item, priority))) => {
                            let wrapped = |module: &str, name: &str, args: Vec<Type>| Type::Base {
                                base: Base::Named {
                                    module: module.to_owned(),
                                    name: name.to_owned(),
                                    args,
                                },
                                refinements: Vec::new(),
                                label: type_label(binder).clone(),
                            };
                            let prelude = una_core::PRELUDE_MODULE;
                            match op {
                                PriorityOp::Peek => wrapped(prelude, "Option", vec![item]),
                                PriorityOp::PeekPriority => {
                                    wrapped(prelude, "Option", vec![priority])
                                }
                                PriorityOp::PeekRanked => {
                                    let ranked = Type::Base {
                                        base: Base::Named {
                                            module: una_core::COLLECTIONS_MODULE.to_owned(),
                                            name: "Ranked".to_owned(),
                                            args: vec![item, priority],
                                        },
                                        refinements: Vec::new(),
                                        label: type_label(binder).clone(),
                                    };
                                    wrapped(prelude, "Option", vec![ranked])
                                }
                                PriorityOp::Items => wrapped(prelude, "List", vec![item]),
                                PriorityOp::Push | PriorityOp::Pop => queue,
                            }
                        }
                        _ => unknown(),
                    }
                }
                // Scalars: their binder has no nested position.
                Prim::Add
                | Prim::Sub
                | Prim::Mul
                | Prim::Div
                | Prim::Mod
                | Prim::Neg
                | Prim::Eq
                | Prim::Ne
                | Prim::Lt
                | Prim::Le
                | Prim::Gt
                | Prim::Ge
                | Prim::Not
                | Prim::Len
                | Prim::Concat
                | Prim::ToText
                | Prim::Convert(_)
                | Prim::Fixed(..)
                | Prim::Float(_)
                | Prim::Shape(_)
                | Prim::Float32(_)
                // A decimal's `Option` results have no fact behind their nested positions.
                | Prim::Decimal(_)
                // A bit set's items are `Int`, with no refinement: nothing it gives back has a
                // refined nested position.
                | Prim::BitSet(_) => unknown(),
                // ADR 0119 V6: what a `Stack` or a `Queue` gives back carries its element
                // position's refinements, as an element of a list does (ADR 0103 rule 6), and an
                // update gives a sequence of the receiver's type.
                Prim::Seq(op) => {
                    let receiver = args.first().map(|seq| self.atom(seq, ctx).ty);
                    let element = receiver.as_ref().and_then(seq_element).cloned();
                    match (receiver, element) {
                        (Some(sequence), Some(element)) => {
                            let wrapped = |name: &str| Type::Base {
                                base: Base::Named {
                                    module: una_core::PRELUDE_MODULE.to_owned(),
                                    name: name.to_owned(),
                                    args: vec![element.clone()],
                                },
                                refinements: Vec::new(),
                                label: type_label(binder).clone(),
                            };
                            match op {
                                SeqOp::PeekFront | SeqOp::PeekBack => wrapped("Option"),
                                SeqOp::Items => wrapped("List"),
                                SeqOp::PushFront
                                | SeqOp::PushBack
                                | SeqOp::PopFront
                                | SeqOp::PopBack => sequence,
                            }
                        }
                        _ => unknown(),
                    }
                }
                // ADR 0119 V6: an item a set gives back carries its position's refinements,
                // as an element of a list does (ADR 0103 rule 6).
                Prim::Set(op) => {
                    let receiver = args.first().map(|set| self.atom(set, ctx).ty);
                    match (op, receiver) {
                        (
                            SetOp::First | SetOp::Last | SetOp::Between | SetOp::Items,
                            Some(Type::Base {
                                base: Base::Named { module, name, args },
                                ..
                            }),
                        ) if is_collection(&module, &name) && name == "Set" && args.len() == 1 => {
                            let wrapper = match op {
                                SetOp::First | SetOp::Last => "Option",
                                _ => "List",
                            };
                            Type::Base {
                                base: Base::Named {
                                    module: una_core::PRELUDE_MODULE.to_owned(),
                                    name: wrapper.to_owned(),
                                    args,
                                },
                                refinements: Vec::new(),
                                label: type_label(binder).clone(),
                            }
                        }
                        // A set that an update or a set operation gives holds the receiver's
                        // items, and the items `add`, `with` and `union` store, each a flow
                        // site of its own: its type is the receiver's, as a map's `with` is.
                        (
                            SetOp::With
                            | SetOp::Without
                            | SetOp::Union
                            | SetOp::Intersection
                            | SetOp::Difference,
                            Some(Type::Base {
                                base: base @ Base::Named { .. },
                                ..
                            }),
                        ) if matches!(
                            &base,
                            Base::Named { module, name, args }
                                if is_collection(module, name) && name == "Set" && args.len() == 1
                        ) =>
                        {
                            Type::Base {
                                base,
                                refinements: Vec::new(),
                                label: type_label(binder).clone(),
                            }
                        }
                        _ => unknown(),
                    }
                }
                // ADR 0119 V6: what a hash map or a hash set gives back carries its position's
                // refinements, as for a map (ADR 0103 rule 6), and an update or a set operation
                // gives a table of the receiver's type, each stored key, value and item a flow
                // site of its own.
                Prim::HashMap(op) => {
                    let receiver = args.first().map(|map| self.atom(map, ctx).ty);
                    match receiver {
                        Some(Type::Base {
                            base: base @ Base::Named { .. },
                            ..
                        }) if matches!(
                            &base,
                            Base::Named { module, name, args }
                                if is_collection(module, name) && name == "HashMap" && args.len() == 2
                        ) =>
                        {
                            let Base::Named { args: kv, .. } = &base else {
                                return Some(unknown());
                            };
                            let Ok([key, value]) = <[Type; 2]>::try_from(kv.clone()) else {
                                return Some(unknown());
                            };
                            let named = |name: &str, args: Vec<Type>| Type::Base {
                                base: Base::Named {
                                    module: una_core::PRELUDE_MODULE.to_owned(),
                                    name: name.to_owned(),
                                    args,
                                },
                                refinements: Vec::new(),
                                label: type_label(binder).clone(),
                            };
                            match op {
                                HashMapOp::Has | HashMapOp::SameEntries => unknown(),
                                HashMapOp::Get => named("Option", vec![value]),
                                HashMapOp::With | HashMapOp::Without => Type::Base {
                                    base,
                                    refinements: Vec::new(),
                                    label: type_label(binder).clone(),
                                },
                                HashMapOp::Keys => named("List", vec![key]),
                                HashMapOp::Values => named("List", vec![value]),
                                HashMapOp::Entries => {
                                    named("List", vec![named("Entry", vec![key, value])])
                                }
                            }
                        }
                        _ => unknown(),
                    }
                }
                Prim::HashSet(op) => {
                    let receiver = args.first().map(|set| self.atom(set, ctx).ty);
                    match (op, receiver) {
                        (
                            HashSetOp::Items,
                            Some(Type::Base {
                                base: Base::Named { module, name, args },
                                ..
                            }),
                        ) if is_collection(&module, &name)
                            && name == "HashSet"
                            && args.len() == 1 =>
                        {
                            Type::Base {
                                base: Base::Named {
                                    module: una_core::PRELUDE_MODULE.to_owned(),
                                    name: "List".to_owned(),
                                    args,
                                },
                                refinements: Vec::new(),
                                label: type_label(binder).clone(),
                            }
                        }
                        (
                            HashSetOp::With
                            | HashSetOp::Without
                            | HashSetOp::Union
                            | HashSetOp::Intersection
                            | HashSetOp::Difference,
                            Some(Type::Base {
                                base: base @ Base::Named { .. },
                                ..
                            }),
                        ) if matches!(
                            &base,
                            Base::Named { module, name, args }
                                if is_collection(module, name) && name == "HashSet" && args.len() == 1
                        ) =>
                        {
                            Type::Base {
                                base,
                                refinements: Vec::new(),
                                label: type_label(binder).clone(),
                            }
                        }
                        _ => unknown(),
                    }
                }
            }),
            Rhs::Call {
                function,
                type_args,
                ..
            } => {
                let key = (function.module.as_str(), function.name.as_str());
                Some(match self.index.contracts.get(&key) {
                    Some(contract) => substitute(contract.result, contract.type_params, type_args),
                    // A predicate: `Bool`.
                    None => unknown(),
                })
            }
            Rhs::Apply { callee, .. } => Some(match self.atom(callee, ctx).ty {
                Type::Fn { result, .. } => (*result).clone(),
                Type::Base { .. } => unknown(),
            }),
            Rhs::Op {
                capability_path,
                op,
                ..
            } => Some(
                self.index
                    .operation(capability_path, op)
                    .map_or_else(unknown, |signature| signature.result.clone()),
            ),
            Rhs::Field { value, index, .. } => {
                let record = self.atom(value, ctx);
                Some(
                    self.index
                        .field_type(&record.ty, 0, *index)
                        .unwrap_or_else(unknown),
                )
            }
            // `checked(a, φ, …)` is `a`: the predicate says nothing of its nested positions.
            Rhs::Checked { value, .. } => Some(self.atom(value, ctx).ty),
            // A policy's result has its `to` type.
            Rhs::Declassify { policy, .. } => {
                let key = (policy.module.as_str(), policy.name.as_str());
                Some(
                    self.index
                        .policies
                        .get(&key)
                        .map_or_else(unknown, |(_, to)| (*to).clone()),
                )
            }
            // Own flows: each field into the instantiated field type (`fields`), each element
            // into the element type (rule 3), and a lambda's own type against the binder's
            // (`fn_flow`, ADR 0037).
            Rhs::Construct { .. } | Rhs::List(_) | Rhs::Lambda { .. } => None,
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
        self.refined_flow(target, value, span, ctx);
    }

    /// [`Walker::refinement_sites`] without the flows of function types: one obligation per
    /// refinement of `target`, and those of its nested positions (ADR 0103 rule 4).
    fn refined_flow(&mut self, target: &Type, value: &Tv, span: Span, ctx: &Ctx) {
        let value_text = self.text(span);
        // ADR 0103 rule 4: the nested positions of the target, by variance.
        self.nested_flows(&value.ty, target, span, &value_text);
        for refinement in refinements(target) {
            let env = Env {
                vars: &|_| None,
                it: Some(value.clone()),
                result: None,
            };
            let goal = self.term(&refinement.clause.term, &env);
            let text = instantiate(&refinement.clause, &value_text);
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
    /// or a binder: their nested positions (ADR 0103 rule 4), and when `target` holds a
    /// function type, its flows (ADR 0037 rule 2), at each returned expression, or at `at`,
    /// the expression a binder is bound to. The top refinements of `target` are the caller's.
    fn returned_functions(
        &mut self,
        entry: &Ctx,
        leaves: &[Leaf],
        target: &Type,
        at: Option<Span>,
    ) {
        // Each path at its own value, as a function's returns are, a bound expression's too.
        for leaf in leaves {
            let Some(value) = &leaf.value else {
                continue;
            };
            let what = self.text(leaf.span);
            self.nested_flows(&value.ty, target, leaf.span, &what);
        }
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

    /// A value of type `source` flowing into `target` at its nested positions (ADR 0103 rule
    /// 4): at each refined position of `target` that `source` reaches, one `refinement`
    /// obligation per refinement that the same position of `source` does not carry, over a
    /// fresh binder that holds the refinements of `source` there. Covariant positions prove
    /// the target's refinements, contravariant ones the source's, invariant ones both. The
    /// obligations are closed: a kind's refinement speaks only of `it`, so no hypothesis of the
    /// path context is needed, and the same pair of refinements gives the same query wherever
    /// it is compared. The top position is [`Walker::refinement_sites`]'s, and the parameters
    /// and result of a function type are [`Walker::fn_flow`]'s. Linear in the size of the
    /// types.
    fn nested_flows(&mut self, source: &Type, target: &Type, span: Span, what: &str) {
        let mut found = Vec::new();
        self.index
            .flow_kinds
            .nested_positions(source, target, Variance::Covariant, "", &mut found);
        for position in found {
            if position.variance != Variance::Contravariant {
                self.nested_obligations(&position.from, &position.to, &position.path, span, what);
            }
            if position.variance != Variance::Covariant {
                self.nested_obligations(&position.to, &position.from, &position.path, span, what);
            }
        }
    }

    /// The obligations of one nested position: each refinement of `to` that `from` does not
    /// carry, as the same formula, holds of a fresh binder that holds those of `from`.
    fn nested_obligations(&mut self, from: &Type, to: &Type, path: &str, span: Span, what: &str) {
        let missing: Vec<Refinement> = una_core::flow::missing(from, to)
            .into_iter()
            .cloned()
            .collect();
        if missing.is_empty() {
            return;
        }
        let shown_binder: String = path
            .chars()
            .map(|c| if c == ' ' || c == '.' { '_' } else { c })
            .collect();
        let mut closed = Ctx::default();
        let binder = self.fresh_binder(&shown_binder);
        let value = self.flow_binder(&mut closed, &binder, from);
        for goal in &missing {
            let shown = instantiate(&goal.clause, &shown_binder);
            // The repair does not quote the value: its text may name an escape, as
            // `checked(…)` does, which an unmarked repair must never propose.
            let repair = format!(
                "build this value at the refined type, or give its {path} the refinement `{}`",
                goal.name
            );
            self.flow_obligation(
                &closed,
                goal,
                &value,
                format!("{path} of `{what}`: {shown}"),
                span,
                repair,
            );
        }
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
                        let shown = instantiate(&goal.clause, name);
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
                    let shown = instantiate(&goal.clause, "result");
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
            let shown = instantiate(&refinement.clause, "result");
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
            // A decimal literal is its coefficient (ADR 0120 V2).
            Atom::Decimal { coefficient, scale } => Tv {
                f: F::Int(coefficient.clone()),
                ty: base_type(Base::Decimal(*scale)),
            },
            // A float literal is its exact bits (ADR 0114 rule 8.1).
            Atom::F64(bits) => Tv {
                f: F::Float(*bits),
                ty: base_type(Base::F64),
            },
            Atom::F32(bits) => Tv {
                f: F::Float32(*bits),
                ty: base_type(Base::F32),
            },
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
            if una_core::complex::is_kind(module, name) {
                // `i!f!re!` and `i!f!im!` into the float sort (ADR 0120 V3).
                format!("i!f!{field}!{sort}")
            } else if kind.record {
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

    /// `find` and `contains_text` share an uninterpreted `i!contains!Text`: one is `Some`
    /// exactly when the other holds (`spec/22 §2` rule 2). Both engines run them over one
    /// search, so the symbol stands for the same relation. A position `find` returns lies in
    /// the text with its match after it.
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
                // The position of a match lies in the text, with the match after it.
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
        // Every collection of ADR 0119 has a length too (V1), with its ground fact `len >= 0`,
        // and a bit set's `len <= size` besides (V2).
        let has_len = sort == TEXT
            || sort.starts_with(LIST)
            || sort.starts_with(MAP)
            || sort.starts_with(ARRAY)
            || sort.starts_with(STACK)
            || sort.starts_with(QUEUE)
            || sort.starts_with(PRIORITY_QUEUE)
            || sort.starts_with(SET)
            || sort.starts_with(HASH_SET)
            || sort.starts_with(HASH_MAP)
            || sort == BITSET;
        has_len.then(|| len_of(&value.f))
    }

    /// `m.rows` or `m.columns` of a matrix, `t.rank` or the element count of a tensor, and
    /// `f.size` of an `Lu[T]`: `i!<fun>!<Sort>` (ADR 0120 V4).
    fn shape(&self, value: &Tv, fun: ShapeFun) -> Option<F> {
        let sort = value.f.sort();
        let has = match fun {
            ShapeFun::Rows | ShapeFun::Columns => sort.starts_with(MATRIX),
            ShapeFun::Rank | ShapeFun::Count => sort.starts_with(TENSOR),
            ShapeFun::Size => sort.starts_with(LU),
        };
        has.then(|| shape_of(&value.f, fun))
    }

    /// The shape of a value as the terms that the frame of a loop keeps (ADR 0071, ADR 0120
    /// V4): its length; or its rows and its columns; or a tensor's rank, its element count
    /// and its length along each of the 8 axes a rank allows.
    fn shape_terms(&self, value: &Tv) -> Vec<F> {
        let sort = value.f.sort();
        if sort.starts_with(MATRIX) {
            vec![
                shape_of(&value.f, ShapeFun::Rows),
                shape_of(&value.f, ShapeFun::Columns),
            ]
        } else if sort.starts_with(TENSOR) {
            let mut terms = vec![
                shape_of(&value.f, ShapeFun::Rank),
                shape_of(&value.f, ShapeFun::Count),
            ];
            terms.extend((0..MAX_RANK).map(|axis| dim_of(&value.f, &F::Int(axis.to_string()))));
            terms
        } else {
            self.len(value).into_iter().collect()
        }
    }

    /// The facts of the prelude's dense methods whose result is an `Option` (ADR 0120 rule
    /// 38), which a clause cannot state: on `Some`, `lu` holds an `Lu[T]` of the matrix's size,
    /// `solve` a matrix of the right-hand side's shape, `solve_array` an array of its length,
    /// and `cholesky` a matrix of the receiver's shape. Each kernel checks the shape it gives
    /// before it returns.
    fn dense_facts(&mut self, name: &str, values: &[Tv], x: &Tv, ctx: &mut Ctx) {
        if !matches!(
            name,
            "dense_matrix_lu"
                | "dense_matrix_solve"
                | "dense_matrix_solve_array"
                | "dense_matrix_cholesky"
        ) {
            return;
        }
        let (Some(tag), Some(payload), Some(m)) =
            (self.tag(x), self.field(x, 0, 0), values.first())
        else {
            return;
        };
        let some = F::eq(tag, F::Int("0".to_owned()));
        let (Some(rows), Some(columns)) = (
            self.shape(m, ShapeFun::Rows),
            self.shape(m, ShapeFun::Columns),
        ) else {
            return;
        };
        let facts = match name {
            "dense_matrix_lu" => self
                .shape(&payload, ShapeFun::Size)
                .map(|size| vec![F::eq(size, rows)]),
            "dense_matrix_solve" => {
                let rhs = values.get(1);
                match (
                    self.shape(&payload, ShapeFun::Rows),
                    self.shape(&payload, ShapeFun::Columns),
                    rhs.and_then(|rhs| self.shape(rhs, ShapeFun::Columns)),
                ) {
                    (Some(r), Some(c), Some(rhs_columns)) => {
                        Some(vec![F::eq(r, rows), F::eq(c, rhs_columns)])
                    }
                    _ => None,
                }
            }
            "dense_matrix_solve_array" => self.len(&payload).map(|len| vec![F::eq(len, rows)]),
            _ => match (
                self.shape(&payload, ShapeFun::Rows),
                self.shape(&payload, ShapeFun::Columns),
            ) {
                (Some(r), Some(c)) => Some(vec![F::eq(r, rows), F::eq(c, columns)]),
                _ => None,
            },
        };
        if let Some(facts) = facts {
            ctx.assume(F::op("=>", vec![some, F::and(facts)]));
        }
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
                // ADR 0069, amendment of 2026-10-19: in edition `0.5` (the root's), the update keeps `has`
                // and `get` at every other key (ADR 0119 V3); editions `0.2` and `0.3` keep M3
                // and M8 as written.
                if self.index.map_frames
                    && x.f.sort() == map.f.sort()
                    && let Some(option) = option_of(&map.ty)
                {
                    ctx.define(frame_of(&map.f, &x.f, &key.f, Some(self::sort(&option))));
                }
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

    /// Facts of a `Stack`'s or a `Queue`'s operation on its result `x` (ADR 0119 rules 5 and 6,
    /// V2), each true of every sequence: a push adds one to the length, a removal takes one off
    /// a nonempty sequence and keeps an empty one, `items()` has the sequence's length, and a
    /// read is `None` exactly when the sequence is empty. Which item a read gives is the
    /// uninterpreted `peek` or `peekback` of V1: a push puts its item at its end, and keeps the
    /// other end of a nonempty sequence; a stack's push has the stack before as its `below`,
    /// and a stack's pop of a nonempty stack gives that `below`. O(1) facts per call.
    fn seq_facts(&mut self, op: SeqOp, values: &[Tv], x: &Tv, ctx: &mut Ctx) {
        let Some(seq) = values.first() else {
            return;
        };
        let Some(old_len) = self.len(seq) else {
            return;
        };
        let zero = || F::Int("0".to_owned());
        let one = || F::Int("1".to_owned());
        let stack = seq.f.sort().starts_with(STACK);
        match op {
            SeqOp::PushFront | SeqOp::PushBack => {
                if let Some(new_len) = self.len(x) {
                    ctx.assume(F::eq(new_len, F::op("+", vec![old_len.clone(), one()])));
                }
                let Some(item) = values.get(1) else {
                    return;
                };
                if x.f.sort() != seq.f.sort() {
                    return;
                }
                // The end the item goes to, and the other end.
                let (near, far) = if op == SeqOp::PushFront {
                    ("peek", "peekback")
                } else {
                    ("peekback", "peek")
                };
                let Some(pushed) = self.seq_read(x, near) else {
                    return;
                };
                if let Some(fact) = self.some_is(&pushed, &item.f) {
                    ctx.assume(fact);
                }
                if stack {
                    // A stack has no back: its push is at the front, with the stack before
                    // under it.
                    ctx.assume(F::eq(below_of(&x.f), seq.f.clone()));
                    return;
                }
                let (Some(other), Some(before)) = (self.seq_read(x, far), self.seq_read(seq, far))
                else {
                    return;
                };
                if let Some(fact) = self.some_is(&other, &item.f) {
                    ctx.assume(F::op("=>", vec![F::eq(old_len.clone(), zero()), fact]));
                }
                ctx.assume(F::op(
                    "=>",
                    vec![F::op(">", vec![old_len, zero()]), F::eq(other.f, before.f)],
                ));
            }
            SeqOp::PopFront | SeqOp::PopBack => {
                if let Some(new_len) = self.len(x) {
                    let nonempty = F::op(">", vec![old_len.clone(), zero()]);
                    let shorter = F::op("-", vec![old_len.clone(), one()]);
                    ctx.assume(F::eq(
                        new_len,
                        F::op("ite", vec![nonempty, shorter, zero()]),
                    ));
                }
                // A stack's pop gives the stack under its top, and keeps an empty stack.
                if stack && op == SeqOp::PopFront && x.f.sort() == seq.f.sort() {
                    ctx.assume(F::op(
                        "=>",
                        vec![
                            F::op(">", vec![old_len.clone(), zero()]),
                            F::eq(x.f.clone(), below_of(&seq.f)),
                        ],
                    ));
                    ctx.assume(F::op(
                        "=>",
                        vec![F::eq(old_len, zero()), F::eq(x.f.clone(), seq.f.clone())],
                    ));
                }
            }
            SeqOp::Items => {
                if let Some(new_len) = self.len(x) {
                    ctx.assume(F::eq(new_len, old_len));
                }
            }
            // `None` is variant 1 of `Option`.
            SeqOp::PeekFront | SeqOp::PeekBack => {
                if let Some(tag) = self.tag(x) {
                    ctx.assume(F::eq(F::eq(tag, one()), F::eq(old_len, zero())));
                }
                let read = if op == SeqOp::PeekFront {
                    "peek"
                } else {
                    "peekback"
                };
                if let Some(read) = self.seq_read(seq, read)
                    && read.f.sort() == x.f.sort()
                {
                    ctx.define(F::eq(x.f.clone(), read.f));
                }
            }
        }
    }

    /// The read `read` (`peek` or `peekback`) of a `Stack`, a `Queue` or a `PriorityQueue`, of
    /// the type `Option` of its item (ADR 0119 V1); `None` for any other value.
    fn seq_read(&self, seq: &Tv, read: &str) -> Option<Tv> {
        let item = seq_element(&seq.ty)
            .or_else(|| priority_positions(&seq.ty).map(|(item, _)| item))?
            .clone();
        let ty = option_type(item);
        Some(Tv {
            f: read_of(&seq.f, read, sort(&ty)),
            ty,
        })
    }

    /// `option == Some(item)`, as its tag and its payload; `None` when the payload's sort is
    /// not the item's.
    fn some_is(&self, option: &Tv, item: &F) -> Option<F> {
        let tag = self.tag(option)?;
        let payload = self.field(option, 0, 0)?;
        if payload.f.sort() != item.sort() {
            return None;
        }
        Some(F::and(vec![
            F::eq(tag, F::Int("0".to_owned())),
            F::eq(payload.f, item.clone()),
        ]))
    }

    /// Facts of a `PriorityQueue`'s operation on its result `x` (ADR 0119 rule 13, V2): the
    /// length facts of a sequence's, since a push adds one item, a pop removes one from a
    /// nonempty queue and keeps an empty one, `items()` has the queue's length, and a read is
    /// `None` exactly when the queue is empty. `peek` is the uninterpreted read of V1: a push
    /// onto an empty queue puts its item first, and any push either puts its item first or
    /// keeps the first item, since the heap is stable. Which item a pop leaves first is not
    /// known. O(1) facts per call.
    fn priority_facts(&mut self, op: PriorityOp, values: &[Tv], x: &Tv, ctx: &mut Ctx) {
        let same = match op {
            PriorityOp::Push => SeqOp::PushBack,
            PriorityOp::Pop => SeqOp::PopFront,
            PriorityOp::Items => SeqOp::Items,
            PriorityOp::Peek | PriorityOp::PeekPriority | PriorityOp::PeekRanked => {
                SeqOp::PeekFront
            }
        };
        let Some(queue) = values.first() else {
            return;
        };
        match op {
            // The length facts only: a priority queue has no `peekback`, and its `peek` is
            // set below.
            PriorityOp::Push => {
                if let (Some(old_len), Some(new_len)) = (self.len(queue), self.len(x)) {
                    ctx.assume(F::eq(
                        new_len,
                        F::op("+", vec![old_len.clone(), F::Int("1".to_owned())]),
                    ));
                    let (Some(item), Some(after), Some(before)) = (
                        values.get(1),
                        self.seq_read(x, "peek"),
                        self.seq_read(queue, "peek"),
                    ) else {
                        return;
                    };
                    if x.f.sort() != queue.f.sort() {
                        return;
                    }
                    if let Some(first) = self.some_is(&after, &item.f) {
                        ctx.assume(F::op(
                            "=>",
                            vec![F::eq(old_len, F::Int("0".to_owned())), first.clone()],
                        ));
                        ctx.assume(F::or(vec![first, F::eq(after.f, before.f)]));
                    }
                }
            }
            // The item read is the queue's `peek`; its priority and its ranked form are known
            // only by their `None`.
            PriorityOp::Peek => self.seq_facts(same, values, x, ctx),
            PriorityOp::PeekPriority | PriorityOp::PeekRanked => {
                if let (Some(old_len), Some(tag)) = (self.len(queue), self.tag(x)) {
                    ctx.assume(F::eq(
                        F::eq(tag, F::Int("1".to_owned())),
                        F::eq(old_len, F::Int("0".to_owned())),
                    ));
                }
            }
            PriorityOp::Pop | PriorityOp::Items => self.seq_facts(same, values, x, ctx),
        }
    }

    /// Facts of an operation of a `Set`, a `HashSet` or a `HashMap` on its result `x` (ADR 0119
    /// V2 to V4). An update at a key makes it a member or not one, changes the length by the
    /// `ite` of its membership before, and records its frame (V3); a map's `set` gives `get`
    /// at the key. `get` is the uninterpreted `get` of V1, `Some` exactly when the key is a
    /// member; `first` and `last` are reads that are `None` exactly when the set is empty and
    /// otherwise members; listings have the length; the set operations give their length
    /// facts and record their relation (V4); `same_items` and `same_entries` are the
    /// uninterpreted `same`, which implies equal lengths and nothing about order. Membership is
    /// a term (`Walker::prim`). O(1) facts per call.
    fn table_facts(&mut self, prim: Prim, values: &[Tv], x: &Tv, ctx: &mut Ctx) {
        let Some(table) = values.first() else {
            return;
        };
        let sort = table.f.sort();
        if !(sort.starts_with(SET) || sort.starts_with(HASH_SET) || sort.starts_with(HASH_MAP)) {
            return;
        }
        // The key or item argument, when its sort is the members' sort: `has` and `get` are
        // declared once per collection sort.
        let member_sort = member_type(&table.ty).map(|ty| self::sort(&ty));
        let key = values
            .get(1)
            .filter(|key| Some(key.f.sort()) == member_sort);
        let get = option_of(&table.ty)
            .filter(|_| sort.starts_with(HASH_MAP))
            .map(|option| self::sort(&option));
        match prim {
            Prim::Set(SetOp::With) | Prim::HashSet(HashSetOp::With) => {
                if let Some(key) = key {
                    self.member_update(table, key, x, true, None, ctx);
                }
            }
            Prim::Set(SetOp::Without) | Prim::HashSet(HashSetOp::Without) => {
                if let Some(key) = key {
                    self.member_update(table, key, x, false, None, ctx);
                }
            }
            Prim::HashMap(HashMapOp::Without) => {
                if let Some(key) = key {
                    self.member_update(table, key, x, false, get, ctx);
                }
            }
            Prim::HashMap(HashMapOp::With) => {
                let Some(key) = key else {
                    return;
                };
                self.member_update(table, key, x, true, get, ctx);
                // `m.get(key: k) == Some(v)` after `set`, `with` and `update`.
                if let (Some(value), Some(option)) = (values.get(2), option_of(&table.ty)) {
                    let got = Tv {
                        f: get_of(&x.f, &key.f, &self::sort(&option)),
                        ty: option,
                    };
                    if let Some(fact) = self.some_is(&got, &value.f) {
                        ctx.assume(fact);
                    }
                }
            }
            Prim::HashMap(HashMapOp::Get) => {
                let Some(key) = key else {
                    return;
                };
                if get.as_ref() != Some(&x.f.sort()) {
                    return;
                }
                ctx.define(F::eq(x.f.clone(), get_of(&table.f, &key.f, &x.f.sort())));
                // `result` is `Some` exactly when `m.has(key: k)`; `Some` is variant 0.
                if let Some(tag) = self.tag(x) {
                    ctx.define(F::eq(
                        F::eq(tag, F::Int("0".to_owned())),
                        has_of(&table.f, &key.f),
                    ));
                }
            }
            Prim::Set(SetOp::First) => self.first_or_last(table, "first", x, ctx),
            Prim::Set(SetOp::Last) => self.first_or_last(table, "last", x, ctx),
            Prim::Set(SetOp::Between) => {
                if let Some(len) = self.len(x) {
                    ctx.assume(F::op("<=", vec![len, len_of(&table.f)]));
                }
            }
            Prim::Set(SetOp::Items)
            | Prim::HashSet(HashSetOp::Items)
            | Prim::HashMap(HashMapOp::Keys | HashMapOp::Values | HashMapOp::Entries) => {
                if let Some(len) = self.len(x) {
                    ctx.assume(F::eq(len, len_of(&table.f)));
                }
            }
            Prim::Set(SetOp::Union) | Prim::HashSet(HashSetOp::Union) => {
                self.set_relation(Combine::Union, values, x, ctx);
            }
            Prim::Set(SetOp::Intersection) | Prim::HashSet(HashSetOp::Intersection) => {
                self.set_relation(Combine::Intersection, values, x, ctx);
            }
            Prim::Set(SetOp::Difference) | Prim::HashSet(HashSetOp::Difference) => {
                self.set_relation(Combine::Difference, values, x, ctx);
            }
            Prim::Set(SetOp::IsSubset) | Prim::HashSet(HashSetOp::IsSubset) => {
                self.set_relation(Combine::Subset, values, x, ctx);
            }
            Prim::HashSet(HashSetOp::SameItems) | Prim::HashMap(HashMapOp::SameEntries) => {
                let Some(other) = values.get(1) else {
                    return;
                };
                if other.f.sort() != sort || x.f.sort() != "Bool" {
                    return;
                }
                ctx.define(F::eq(x.f.clone(), same_of(&table.f, &other.f)));
                ctx.assume(F::op(
                    "=>",
                    vec![x.f.clone(), F::eq(len_of(&table.f), len_of(&other.f))],
                ));
            }
            _ => {}
        }
    }

    /// The facts of an update of a set, a hash set, a hash map or a bit set at `key`, giving
    /// `x` (ADR 0119 V2, V3): `key` is a member of `x` after `add`, `with`, `set` and `update`
    /// and not one after `remove` and `without`; the length changes by one exactly when the
    /// membership changes; and `x` is `table` updated at `key` only, with `get` the option
    /// sort of a map's `get`.
    fn member_update(
        &mut self,
        table: &Tv,
        key: &Tv,
        x: &Tv,
        add: bool,
        get: Option<Sort>,
        ctx: &mut Ctx,
    ) {
        if x.f.sort() != table.f.sort() {
            return;
        }
        let had = has_of(&table.f, &key.f);
        let old = len_of(&table.f);
        let one = || F::Int("1".to_owned());
        if add {
            ctx.assume(has_of(&x.f, &key.f));
            ctx.assume(F::eq(
                len_of(&x.f),
                F::op("ite", vec![had, old.clone(), F::op("+", vec![old, one()])]),
            ));
        } else {
            ctx.assume(F::not(has_of(&x.f, &key.f)));
            ctx.assume(F::eq(
                len_of(&x.f),
                F::op("ite", vec![had, F::op("-", vec![old.clone(), one()]), old]),
            ));
        }
        ctx.define(frame_of(&table.f, &x.f, &key.f, get));
    }

    /// `x = s.first()` or `s.last()` of a `Set` or a `BitSet` (ADR 0119 V2): the read `read`
    /// of V1, `None` exactly when the set is empty, and a member otherwise.
    fn first_or_last(&mut self, set: &Tv, read: &str, x: &Tv, ctx: &mut Ctx) {
        ctx.define(F::eq(x.f.clone(), read_of(&set.f, read, x.f.sort())));
        let member_sort = member_type(&set.ty).map(|ty| self::sort(&ty));
        if let (Some(tag), Some(value)) = (self.tag(x), self.field(x, 0, 0))
            && Some(value.f.sort()) == member_sort
        {
            ctx.assume(F::op(
                "=>",
                vec![F::eq(tag, F::Int("0".to_owned())), has_of(&set.f, &value.f)],
            ));
        }
    }

    /// The facts of `s.union(other: t)` and the other set operations of a `Set`, a `HashSet`
    /// or a `BitSet`, on its result `x` (ADR 0119 rules 7, 12 and 14, V4): the length facts of
    /// their contracts, and the record of their relation, which every item term of a query
    /// instantiates (`instances`).
    fn set_relation(&mut self, op: Combine, values: &[Tv], x: &Tv, ctx: &mut Ctx) {
        let (Some(s), Some(t)) = (values.first(), values.get(1)) else {
            return;
        };
        let sort = s.f.sort();
        let Some(item) = member_type(&s.ty).map(|ty| self::sort(&ty)) else {
            return;
        };
        if t.f.sort() != sort {
            return;
        }
        let (ls, lt) = (len_of(&s.f), len_of(&t.f));
        if op == Combine::Subset {
            if x.f.sort() != "Bool" {
                return;
            }
            ctx.define(F::eq(
                x.f.clone(),
                combine_of(op, &[s.f.clone(), t.f.clone()], item),
            ));
            ctx.assume(F::op("=>", vec![x.f.clone(), F::op("<=", vec![ls, lt])]));
            return;
        }
        if x.f.sort() != sort {
            return;
        }
        let lx = len_of(&x.f);
        match op {
            Combine::Union => {
                ctx.assume(F::op(">=", vec![lx.clone(), ls]));
                ctx.assume(F::op(">=", vec![lx, lt]));
            }
            Combine::Intersection => {
                ctx.assume(F::op("<=", vec![lx.clone(), ls]));
                ctx.assume(F::op("<=", vec![lx, lt]));
            }
            Combine::Difference | Combine::Subset => {
                ctx.assume(F::op("<=", vec![lx, ls]));
            }
        }
        ctx.define(combine_of(
            op,
            &[s.f.clone(), t.f.clone(), x.f.clone()],
            item,
        ));
    }

    /// The preconditions of a bit set's operation (ADR 0119 rule 14), each a `precondition`
    /// obligation at its argument: `0 <= i and i < b.size` for `add` and `with`, and
    /// `c.size == b.size` for a set operation. O(1) per call.
    fn bit_set_sites(
        &mut self,
        op: BitSetOp,
        values: &[Tv],
        arg_spans: &[Span],
        span: Span,
        ctx: &Ctx,
    ) {
        let (Some(bits), Some(argument)) = (values.first(), values.get(1)) else {
            return;
        };
        if bits.f.sort() != BITSET {
            return;
        }
        let at = arg_spans.get(1).copied().unwrap_or(span);
        let receiver = arg_spans
            .first()
            .map_or_else(String::new, |receiver| self.text(*receiver));
        let shown = self.text(at);
        let (goal, text) = if op == BitSetOp::With {
            (
                F::and(vec![
                    F::op("<=", vec![F::Int("0".to_owned()), argument.f.clone()]),
                    F::op("<", vec![argument.f.clone(), size_of(&bits.f)]),
                ]),
                format!("0 <= {shown} and {shown} < {receiver}.size"),
            )
        } else if op.pairs() && argument.f.sort() == BITSET {
            (
                F::eq(size_of(&argument.f), size_of(&bits.f)),
                format!("{shown}.size == {receiver}.size"),
            )
        } else {
            return;
        };
        self.site(
            ObligationKind::Precondition,
            ctx,
            Some(goal),
            text,
            at,
            None,
        );
    }

    /// The facts of a bit set's operation as facts of its result `x` (ADR 0119 V2 to V4): an
    /// update keeps the size, makes its item a member or not one, changes the length by the
    /// `ite` of its membership before, and records its frame (V3); a set operation keeps the
    /// size, gives its length facts and records its relation (V4); `first` and `last` are
    /// reads, `None` exactly when the bit set is empty and members otherwise; `items` has its
    /// length. `size` and `has` are terms (`Walker::prim`). O(1) per call.
    fn bit_set_facts(&mut self, op: BitSetOp, values: &[Tv], x: &Tv, ctx: &mut Ctx) {
        let Some(bits) = values.first() else {
            return;
        };
        if bits.f.sort() != BITSET {
            return;
        }
        let b = &bits.f;
        let zero = || F::Int("0".to_owned());
        let one = || F::Int("1".to_owned());
        match op {
            BitSetOp::With | BitSetOp::Without => {
                let Some(item) = values.get(1) else {
                    return;
                };
                if x.f.sort() != BITSET || item.f.sort() != "Int" {
                    return;
                }
                ctx.assume(F::eq(size_of(&x.f), size_of(b)));
                self.member_update(bits, item, x, op == BitSetOp::With, None, ctx);
            }
            BitSetOp::Union | BitSetOp::Intersection | BitSetOp::Difference => {
                if x.f.sort() == BITSET {
                    ctx.assume(F::eq(size_of(&x.f), size_of(b)));
                }
                let relation = match op {
                    BitSetOp::Union => Combine::Union,
                    BitSetOp::Intersection => Combine::Intersection,
                    _ => Combine::Difference,
                };
                self.set_relation(relation, values, x, ctx);
            }
            BitSetOp::IsSubset => self.set_relation(Combine::Subset, values, x, ctx),
            // `None` is variant 1 of `Option`.
            BitSetOp::First | BitSetOp::Last => {
                if let Some(tag) = self.tag(x) {
                    ctx.assume(F::eq(F::eq(tag, one()), F::eq(len_of(b), zero())));
                }
                let read = if op == BitSetOp::First {
                    "first"
                } else {
                    "last"
                };
                self.first_or_last(bits, read, x, ctx);
            }
            BitSetOp::Items => {
                if let Some(len) = self.len(x) {
                    ctx.assume(F::eq(len, len_of(b)));
                }
            }
            BitSetOp::Size | BitSetOp::Has => {}
        }
    }

    /// How many tasks a `together for` over `source` runs: the length of its list, or the
    /// number of integers of its range, zero when it is empty.
    fn task_count(&mut self, source: &LoopSource, ctx: &Ctx) -> Option<F> {
        match source {
            LoopSource::Until { .. } | LoopSource::Ever { .. } => None,
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

    /// The obligations of an operation of `Decimal` (ADR 0120 rule 15, V2): `division` for
    /// `divide` and `remainder` by a divisor that is not a nonzero literal, and the
    /// precondition of `rescale_exact` that no nonzero digit is dropped,
    /// `(= (mod c 10^k) 0)`, linear.
    fn decimal_sites(
        &mut self,
        prim: Prim,
        args: &[Atom],
        values: &[Tv],
        arg_spans: &[Span],
        span: Span,
        ctx: &mut Ctx,
    ) {
        let Prim::Decimal(op) = prim else {
            return;
        };
        match op {
            DecimalOp::Divide(..) | DecimalOp::Remainder(..) => {
                let (Some(divisor), Some(divisor_atom)) = (values.get(1), args.get(1)) else {
                    return;
                };
                let literal = match divisor_atom {
                    Atom::Decimal { coefficient, .. } => coefficient.trim_start_matches('-') != "0",
                    Atom::Var(name) => self.defs.get(name).is_some_and(nonzero_literal),
                    _ => false,
                };
                if literal || nonzero_literal(&divisor.f) {
                    return;
                }
                let text = format!(
                    "{} != 0",
                    arg_spans.get(1).map_or_else(String::new, |s| self.text(*s))
                );
                self.site(
                    ObligationKind::Division,
                    ctx,
                    Some(F::not(F::eq(divisor.f.clone(), F::Int("0".to_owned())))),
                    text,
                    span,
                    None,
                );
                std::rc::Rc::make_mut(&mut ctx.divisions).push((divisor.f.clone(), span));
            }
            DecimalOp::RescaleExact(a, s) => {
                let Some(value) = values.first() else {
                    return;
                };
                let k = a.saturating_sub(s);
                let goal = F::eq(
                    F::op("mod", vec![value.f.clone(), F::Int(pow10(k))]),
                    F::Int("0".to_owned()),
                );
                let text = format!(
                    "{} drops no nonzero digit at scale {s}",
                    arg_spans
                        .first()
                        .map_or_else(String::new, |at| self.text(*at))
                );
                self.site(
                    ObligationKind::Precondition,
                    ctx,
                    Some(goal),
                    text,
                    span,
                    None,
                );
            }
            _ => {}
        }
    }

    /// The obligations of a primitive of fixed-width integers (`spec/18 §3`, `§5`): an
    /// `overflow` obligation for arithmetic whose result has a fixed-width type and for a
    /// narrowing conversion, and the precondition `0 <= n and n < width` of a shift. A negated
    /// literal is a literal, which the checker bounds (U2022). In a `systems` package, `Int`
    /// arithmetic has its `overflow` obligations too ([`Walker::int_sites`], ADR 0109 rule 3).
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
        let int = match ty {
            Type::Base {
                base: Base::Fixed(int),
                ..
            } => *int,
            Type::Base {
                base: Base::Int, ..
            } if self.index.profile == Profile::Systems => {
                self.int_sites(prim, args, values, arg_spans, span, ctx);
                return;
            }
            _ => return,
        };
        let exact = match prim {
            Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Mod => self.prim(prim, values),
            Prim::Neg if !matches!(args.first(), Some(Atom::Int(_))) => self.prim(prim, values),
            Prim::Convert(Some(_)) => {
                let source = values.first().map(|v| &v.ty);
                let widening = matches!(
                    source,
                    Some(Type::Base { base: Base::Fixed(from), .. }) if from.widens_to(int)
                );
                if widening { None } else { arg(0) }
            }
            _ => None,
        };
        let Some(exact) = exact else {
            return;
        };
        let range = format!("{}..{}", int.least(), int.greatest());
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
        // An element-wise operator of dense values (ADR 0120 rule 34) applies the operation to
        // the elements its lambda receives, whose operands carry the operator's own span: no
        // guard written on the operands' text would test them.
        let element_wise = arg_spans.contains(&span);
        let guard = match prim {
            _ if element_wise => None,
            Prim::Convert(_) => {
                let written = arg_spans
                    .first()
                    .map_or_else(String::new, |s| self.text(*s));
                Some(format!("{written} in {range}"))
            }
            _ => operation_guard(prim, &operand, &range),
        };
        self.overflow_site(
            exact,
            (int.least(), int.greatest()),
            guard,
            OverflowOf::Fixed,
            span,
            ctx,
        );
    }

    /// The `overflow` obligations of a primitive whose result is an `Int` in a `systems`
    /// package (ADR 0109 rule 3, `spec/18 §3`): its exact result lies in the tagged range,
    /// [`INT_LEAST`] to [`INT_GREATEST`]. They are those of `+`, `-`, `*`, `/`, `mod`, of
    /// unary `-` on a value that is not a literal, of the product by its ratio that
    /// `convert()` elaborates to, and of `to_int()` on `I64` and `U64`. A negated literal is
    /// one literal: outside the range, its ground goal is refuted at once. An element-wise
    /// operator of dense values carries none on `Int` elements, as the checker records none
    /// (ADR 0120 rule 34 states the element obligation for fixed-width elements).
    fn int_sites(
        &mut self,
        prim: Prim,
        args: &[Atom],
        values: &[Tv],
        arg_spans: &[Span],
        span: Span,
        ctx: &Ctx,
    ) {
        // The operands of an element-wise operator carry the operator's span; the ratio of
        // `convert()` does too, as a literal.
        let element_wise = args
            .iter()
            .zip(arg_spans)
            .any(|(atom, at)| *at == span && !matches!(atom, Atom::Int(_)));
        if element_wise {
            return;
        }
        let range = format!("{INT_LEAST}..{INT_GREATEST}");
        // The operands as they are written: `Int` needs no conversion, and the ratio of
        // `convert()` is its digits.
        let operand = |i: usize| -> String {
            match args.get(i) {
                Some(Atom::Int(digits)) => digits.clone(),
                _ => arg_spans.get(i).map_or_else(String::new, |s| self.text(*s)),
            }
        };
        // The literal operand of a product, as the ratio of `convert()` is.
        let literal = |i: usize| match args.get(i) {
            Some(Atom::Int(digits)) => digits.parse::<i128>().ok(),
            _ => None,
        };
        // `convert()` multiplies its receiver by the ratio, at the method's own span.
        let ratio = (prim == Prim::Mul && arg_spans.get(1) == Some(&span))
            .then(|| literal(1))
            .flatten()
            .filter(|k| *k > 0);
        let (exact, guard) = match prim {
            Prim::Mul if ratio.is_some() => {
                let k = ratio.unwrap_or(1);
                (
                    self.prim(prim, values),
                    Some(format!(
                        "{} in -{}..{}",
                        operand(0),
                        -INT_LEAST / k,
                        INT_GREATEST / k
                    )),
                )
            }
            Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Mod => {
                (self.prim(prim, values), int_guard(prim, &operand, &literal))
            }
            Prim::Neg => match args.first() {
                // A negated literal is one literal: a site only outside the range.
                Some(Atom::Int(digits)) => {
                    let value = format!("-{digits}");
                    if fits_word(&value) {
                        return;
                    }
                    (Some(F::Int(value)), None)
                }
                _ => (self.prim(prim, values), int_guard(prim, &operand, &literal)),
            },
            // `to_int()` on a 64-bit value, which can lie outside the tagged range.
            Prim::Convert(None)
                if matches!(
                    values.first().map(|v| &v.ty),
                    Some(Type::Base {
                        base: Base::Fixed(IntType::I64 | IntType::U64),
                        ..
                    })
                ) =>
            {
                (
                    values.first().map(|v| v.f.clone()),
                    Some(format!("{} in {range}", operand(0))),
                )
            }
            _ => return,
        };
        let Some(exact) = exact else {
            return;
        };
        self.overflow_site(
            exact,
            (INT_LEAST, INT_GREATEST),
            guard,
            OverflowOf::Int,
            span,
            ctx,
        );
    }

    /// The `overflow` obligation of an `Int` literal outside the tagged range, bound to its
    /// own temporary in a `systems` package (ADR 0109 rule 3): its goal is ground, and
    /// refuted at once.
    fn literal_site(&mut self, ty: &Type, atom: &Atom, span: Span, ctx: &Ctx) {
        if self.index.profile != Profile::Systems {
            return;
        }
        let (
            Type::Base {
                base: Base::Int, ..
            },
            Atom::Int(digits),
        ) = (ty, atom)
        else {
            return;
        };
        if fits_word(digits) {
            return;
        }
        self.overflow_site(
            F::Int(digits.clone()),
            (INT_LEAST, INT_GREATEST),
            None,
            OverflowOf::Int,
            span,
            ctx,
        );
    }

    /// The `overflow` obligation of a call of the prelude's `abs` or `sum` in a `systems`
    /// package, over the call's result `x` (ADR 0109 rule 3): `abs`'s `ensures`, assumed
    /// already, proves it when the operand is bounded; `sum` states no bound on its result, so
    /// a refutation sets it freely and the obligation is unknown.
    fn prelude_site(&mut self, name: &str, x: &Tv, arg_spans: &[Span], span: Span, ctx: &Ctx) {
        if self.index.profile != Profile::Systems || self.eta.is_some() {
            return;
        }
        // The guard bounds the operand, never the result: in the range and off the least
        // value, `abs` stays in it. `sum` has none, as no test on its operands bounds its result.
        let (of, guard) = match name {
            "abs" => (
                OverflowOf::Int,
                arg_spans.first().map(|at| off_least(&self.text(*at))),
            ),
            "sum" => (OverflowOf::Unbounded(name), None),
            _ => return,
        };
        self.overflow_site(x.f.clone(), (INT_LEAST, INT_GREATEST), guard, of, span, ctx);
    }

    /// The `overflow` obligation of the prelude's `abs` or `sum` used as a function value at
    /// `span`, as in `xs.map(abs)`, in a `systems` package (ADR 0109 rule 3): unknown by
    /// construction, since the operands have no bounds at that use, so no query is sent. Its
    /// one repair is the call inside a lambda, where the operand's bounds apply.
    fn prelude_value_site(&mut self, path: &Path, span: Span, ctx: &Ctx) {
        if self.index.profile != Profile::Systems || !is_prelude(&path.module) {
            return;
        }
        let name = path.name.as_str();
        let operand = match name {
            "abs" => "x",
            "sum" => "xs",
            _ => return,
        };
        let text = format!("{} in {INT_LEAST}..{INT_GREATEST}", self.text(span));
        let mut place = self.place(ObligationKind::Overflow, None);
        place.int = true;
        place.unknown = Some(format!(
            "the operands of `{name}` have no bounds at this use"
        ));
        place.flow = Some(format!(
            "write the call inside a lambda, `fn({operand}) -> {name}({operand})`, so its operand's bounds apply"
        ));
        self.push(
            ObligationKind::Overflow,
            text,
            span,
            None,
            Vec::new(),
            Vec::new(),
            vec![F::Bool(false)],
            ctx,
            place,
            vec![ctx.divisions.to_vec()],
        );
    }

    /// An `overflow` obligation at `span`: `exact` lies in `least..greatest` (`spec/18 §3`).
    /// Its goal text is the operation's source text in that range, and `guard`, when given, is
    /// the repair's guard.
    fn overflow_site(
        &mut self,
        exact: F,
        (least, greatest): (i128, i128),
        guard: Option<String>,
        of: OverflowOf<'_>,
        span: Span,
        ctx: &Ctx,
    ) {
        let goal = within(exact, least, greatest);
        let text = format!("{} in {least}..{greatest}", self.text(span));
        let mut assertions: Vec<F> = ctx.hyps.iter().map(|h| h.f.clone()).collect();
        let mut place = self.place(ObligationKind::Overflow, Some(&goal));
        place.guard = guard;
        place.int = !matches!(of, OverflowOf::Fixed);
        if let OverflowOf::Unbounded(callee) = of {
            place.unbounded = Some(callee.to_owned());
        }
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
            // `m.rows` and `m.columns` (ADR 0120 V4).
            Prim::Shape(fun) => self.shape(&arg(0)?, fun)?,
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
            // `n.to_f64()` of an integer, by the integer's type (ADR 0114 rule 8.1).
            Prim::Float(FloatOp::FromInt) => {
                let value = arg(0)?;
                to_float(Fmt::F64, value.f, integer_type(&value.ty), false)?
            }
            Prim::Float32(FloatOp::FromInt) => {
                let value = arg(0)?;
                to_float(Fmt::F32, value.f, integer_type(&value.ty), false)?
            }
            // The other operations of the refinement language; a code-only method's result is
            // a fresh binder (ADR 0114 rule 8.1).
            Prim::Float(op) => {
                let args: Vec<F> = values.iter().map(|v| v.f.clone()).collect();
                float_term(Fmt::F64, op, &args)?
            }
            Prim::Float32(op) => {
                let args: Vec<F> = values.iter().map(|v| v.f.clone()).collect();
                float_term(Fmt::F32, op, &args)?
            }
            // Membership of a set, a hash set and a hash map is a term, `i!has!<Sort>` (ADR
            // 0119 V1), as a bit set's is below.
            Prim::Set(SetOp::Has)
            | Prim::HashSet(HashSetOp::Has)
            | Prim::HashMap(HashMapOp::Has) => {
                let (table, key) = (arg(0)?, arg(1)?.f);
                if !has_membership(&table.f.sort())
                    || member_type(&table.ty).map(|ty| self::sort(&ty)) != Some(key.sort())
                {
                    return None;
                }
                has_of(&table.f, &key)
            }
            // A sequence's operations are known by their facts (`seq_facts`), not as terms, and
            // so are the other methods of a set, a hash map and a hash set (`table_facts`,
            // ADR 0119 V2 to V4).
            Prim::Concat
            | Prim::ToText
            | Prim::Fixed(..)
            | Prim::Map(_)
            | Prim::Seq(_)
            | Prim::Set(_)
            | Prim::HashMap(_)
            | Prim::HashSet(_)
            | Prim::Priority(_) => {
                return None;
            }
            // A bit set's size and its membership are terms, `i!size!i!BitSet` and
            // `i!has!i!BitSet` (ADR 0119 V1); its other operations are known by their facts
            // (`bit_set_facts`).
            Prim::BitSet(BitSetOp::Size) => {
                let bits = arg(0)?.f;
                if bits.sort() != BITSET {
                    return None;
                }
                size_of(&bits)
            }
            Prim::BitSet(BitSetOp::Has) => {
                let bits = arg(0)?.f;
                if bits.sort() != BITSET {
                    return None;
                }
                has_of(&bits, &arg(1)?.f)
            }
            Prim::BitSet(_) => return None,
            Prim::Decimal(op) => return self.decimal(op, values),
        })
    }

    /// The term of an operation of `Decimal` over its operands' coefficients (ADR 0120 V2):
    /// linear wherever a divisor or a factor is a literal. `None` for the operations known
    /// only by their contracts (text, JSON and floats), whose results are fresh binders.
    fn decimal(&self, op: DecimalOp, values: &[Tv]) -> Option<F> {
        let arg = |i: usize| values.get(i).map(|v| v.f.clone());
        let aligned = decimal_aligned;
        let pair = |a: u8, b: u8| -> Option<(F, F)> {
            let m = a.max(b);
            Some((aligned(arg(0)?, m - a), aligned(arg(1)?, m - b)))
        };
        Some(match op {
            DecimalOp::Add(a, b) => {
                let (x, y) = pair(a, b)?;
                F::op("+", vec![x, y])
            }
            DecimalOp::Sub(a, b) => {
                let (x, y) = pair(a, b)?;
                F::op("-", vec![x, y])
            }
            DecimalOp::Mul => arithmetic(BinOp::Mul, self.literal(arg(0)?), self.literal(arg(1)?)),
            DecimalOp::Neg => F::op("-", vec![arg(0)?]),
            DecimalOp::Eq(a, b) => {
                let (x, y) = pair(a, b)?;
                F::eq(x, y)
            }
            DecimalOp::Ne(a, b) => {
                let (x, y) = pair(a, b)?;
                F::not(F::eq(x, y))
            }
            DecimalOp::Lt(a, b)
            | DecimalOp::Le(a, b)
            | DecimalOp::Gt(a, b)
            | DecimalOp::Ge(a, b) => {
                let (x, y) = pair(a, b)?;
                let symbol = match op {
                    DecimalOp::Lt(..) => "<",
                    DecimalOp::Le(..) => "<=",
                    DecimalOp::Gt(..) => ">",
                    _ => ">=",
                };
                F::op(symbol, vec![x, y])
            }
            DecimalOp::Widen(a, s) => aligned(arg(0)?, s.saturating_sub(a)),
            DecimalOp::Rescale(a, s) => {
                let mode = self.rounding_tag(values.get(1)?)?;
                rounded_quotient(arg(0)?, F::Int(pow10(a.saturating_sub(s))), &mode)
            }
            DecimalOp::ToInt(a) => {
                let mode = self.rounding_tag(values.get(1)?)?;
                rounded_quotient(arg(0)?, F::Int(pow10(a)), &mode)
            }
            // The precondition makes the quotient exact, so Euclidean `div` by the literal is
            // the exact quotient for either sign.
            DecimalOp::RescaleExact(a, s) => {
                F::op("div", vec![arg(0)?, F::Int(pow10(a.saturating_sub(s)))])
            }
            DecimalOp::Divide(a, b, s) => {
                let mode = self.rounding_tag(values.get(2)?)?;
                let n = aligned(self.literal(arg(0)?), b.saturating_add(s));
                let d = aligned(literal_int(self.literal(arg(1)?)), a);
                rounded_quotient(n, d, &mode)
            }
            DecimalOp::Remainder(a, b, s) => {
                let toward_zero = F::Int(TOWARD_ZERO.to_string());
                let n = aligned(self.literal(arg(0)?), b.saturating_add(s));
                let d = aligned(literal_int(self.literal(arg(1)?)), a);
                let q = rounded_quotient(n, d, &toward_zero);
                let t = una_core::decimal::remainder_scale(a, b, s);
                let product = arithmetic(BinOp::Mul, arg(1)?, q);
                F::op(
                    "-",
                    vec![
                        aligned(arg(0)?, t.saturating_sub(a)),
                        aligned(product, t.saturating_sub(b.saturating_add(s))),
                    ],
                )
            }
            DecimalOp::Abs => {
                let x = arg(0)?;
                F::op(
                    "ite",
                    vec![
                        F::op(">=", vec![x.clone(), F::Int("0".to_owned())]),
                        x.clone(),
                        F::op("-", vec![x]),
                    ],
                )
            }
            DecimalOp::Coefficient | DecimalOp::FromInt => arg(0)?,
            DecimalOp::FromF64(_)
            | DecimalOp::ToF64(_)
            | DecimalOp::ToText(_)
            | DecimalOp::Parse(_)
            | DecimalOp::FromJson(_)
            | DecimalOp::ToJson(_) => return None,
        })
    }

    /// The tag of a `Rounding` operand: the position of its variant, in the order of
    /// `una_core::decimal::Rounding::ALL`.
    fn rounding_tag(&self, rounding: &Tv) -> Option<F> {
        self.tag(rounding)
    }

    /// Equality of two values: of tags for a sum without payloads, of the values otherwise.
    fn equal(&self, a: &Tv, b: &Tv) -> Option<F> {
        if a.f.sort() != b.f.sort() {
            return None;
        }
        // `Complex[F64]` and `Complex[F32]` are equal when both components are, by identity
        // (ADR 0120 rule 23, V3), whichever elements of the sort they are.
        if let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &a.ty
            && una_core::complex::is_kind(module, name)
        {
            let mut parts = Vec::new();
            for index in 0..2 {
                let (x, y) = (self.field(a, 0, index)?, self.field(b, 0, index)?);
                parts.push(F::eq(x.f, y.f));
            }
            return Some(F::and(parts));
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

    /// `left == right` where one side is `complex(re: a, im: b)` (ADR 0120 V3): each component
    /// of one side equals the same component of the other by identity, a component of a
    /// construction being its argument and any other a field of its value.
    fn complex_equal(&mut self, left: &Term, right: &Term, env: &Env<'_>) -> Option<F> {
        let ((Term::Complex { ctor, .. }, _) | (_, Term::Complex { ctor, .. })) = (left, right)
        else {
            return None;
        };
        let mut parts = Vec::new();
        for (index, field) in ["re", "im"].into_iter().enumerate() {
            let component = |side: &Term| Term::Field {
                value: Box::new(side.clone()),
                kind: ctor.kind.clone(),
                field: field.to_owned(),
                index,
            };
            let (l, r) = (
                self.term(&component(left), env)?,
                self.term(&component(right), env)?,
            );
            if l.f.sort() != r.f.sort() {
                return None;
            }
            parts.push(F::eq(l.f, r.f));
        }
        Some(F::and(parts))
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
        // `F64` by IEEE totalOrder, also for a type parameter instantiated with it (ADR 0114
        // rules 8.1 and 8.7).
        if sort == FLOAT64 || sort == FLOAT32 {
            let fmt = if sort == FLOAT32 { Fmt::F32 } else { Fmt::F64 };
            let float_op = match op {
                BinOp::Lt => FloatOp::Lt,
                BinOp::Le => FloatOp::Le,
                BinOp::Gt => FloatOp::Gt,
                _ => FloatOp::Ge,
            };
            return float_term(fmt, float_op, &[a.f.clone(), b.f.clone()]);
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
            Term::F64(bits) => Tv {
                f: F::Float(*bits),
                ty: base_type(Base::F64),
            },
            Term::F32(bits) => Tv {
                f: F::Float32(*bits),
                ty: base_type(Base::F32),
            },
            Term::Float { op, args } | Term::Float32 { op, args } => {
                let fmt = if matches!(term, Term::Float32 { .. }) {
                    Fmt::F32
                } else {
                    Fmt::F64
                };
                let values: Vec<F> = args
                    .iter()
                    .map(|a| self.term(a, env).map(|v| v.f))
                    .collect::<Option<_>>()?;
                let f = float_term(fmt, *op, &values)?;
                let sort = f.sort();
                let ty = if sort == FLOAT64 {
                    base_type(Base::F64)
                } else if sort == FLOAT32 {
                    base_type(Base::F32)
                } else {
                    boolean()
                };
                Tv { f, ty }
            }
            Term::ToF64 { value, from } => {
                let value = self.term(value, env)?;
                Tv {
                    f: to_float(Fmt::F64, value.f, *from, true)?,
                    ty: base_type(Base::F64),
                }
            }
            Term::ToF32 { value, from } => {
                let value = self.term(value, env)?;
                Tv {
                    f: to_float(Fmt::F32, value.f, *from, true)?,
                    ty: base_type(Base::F32),
                }
            }
            Term::Unit => return None,
            Term::Ctor(_) => return None,
            // A component of `complex(re: a, im: b)` is that component (ADR 0120 V3).
            Term::Field { value, index, .. } if matches!(value.as_ref(), Term::Complex { .. }) => {
                let Term::Complex { re, im, .. } = value.as_ref() else {
                    return None;
                };
                match *index {
                    0 => self.term(re, env)?,
                    1 => self.term(im, env)?,
                    _ => return None,
                }
            }
            Term::Field { value, index, .. } => {
                let value = self.term(value, env)?;
                self.field(&value, 0, *index)?
            }
            // A construction has no term of its own: `==` and `!=` and the fields read its
            // components (`binary`, above), and any other use is outside the encoding.
            Term::Complex { .. } => return None,
            Term::Len(value) => {
                let value = self.term(value, env)?;
                Tv {
                    f: self.len(&value)?,
                    ty: int(),
                }
            }
            Term::Shape { fun, value } => {
                let value = self.term(value, env)?;
                Tv {
                    f: self.shape(&value, *fun)?,
                    ty: int(),
                }
            }
            // `t.dim(axis: k)`, `i!dim!<Sort>` (ADR 0120 V4).
            Term::Dim { value, axis } => {
                let value = self.term(value, env)?;
                let axis = self.term(axis, env)?;
                if !value.f.sort().starts_with(TENSOR) || axis.f.sort() != "Int" {
                    return None;
                }
                Tv {
                    f: dim_of(&value.f, &axis.f),
                    ty: int(),
                }
            }
            // The element of a dense value, uninterpreted, of its element type's sort (ADR
            // 0120 V4): no fact relates it to other elements, which would need a quantifier. A
            // tensor has one function per rank, so each application has a fixed arity.
            Term::Element { value, indices, .. } => {
                let value = self.term(value, env)?;
                let sort = value.f.sort();
                if !(sort.starts_with(ARRAY)
                    || sort.starts_with(MATRIX)
                    || sort.starts_with(TENSOR))
                {
                    return None;
                }
                let element = match &value.ty {
                    Type::Base {
                        base: Base::Named { args, .. },
                        ..
                    } => args.first()?.clone(),
                    _ => return None,
                };
                let mut args = vec![value.f.clone()];
                for index in indices {
                    args.push(self.term(index, env)?.f);
                }
                Tv {
                    f: element_of(&args, &self::sort(&element)),
                    ty: element,
                }
            }
            // A key of a map or a hash map, an item of a set, a hash set or a bit set (ADR 0119
            // V5), each at the sort of its members, so that `has` is declared once per sort.
            Term::Has {
                map: set,
                key: item,
            }
            | Term::Member { set, item } => {
                let set = self.term(set, env)?;
                let item = self.term(item, env)?;
                let sort = set.f.sort();
                let members = if sort.starts_with(MAP) {
                    Some(item.f.sort())
                } else {
                    member_type(&set.ty).map(|ty| self::sort(&ty))
                };
                if !has_membership(&sort) || members != Some(item.f.sort()) {
                    return None;
                }
                Tv {
                    f: has_of(&set.f, &item.f),
                    ty: boolean(),
                }
            }
            // `b.size` of a bit set (ADR 0119 V5).
            Term::Size(value) => {
                let value = self.term(value, env)?;
                if value.f.sort() != BITSET {
                    return None;
                }
                Tv {
                    f: size_of(&value.f),
                    ty: int(),
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
        // `==` and `!=` with `complex(re: a, im: b)` compare the components by identity (ADR
        // 0120 V3): the construction is never a term of its own.
        if matches!(op, BinOp::Eq | BinOp::Ne)
            && (matches!(left, Term::Complex { .. }) || matches!(right, Term::Complex { .. }))
        {
            let eq = self.complex_equal(left, right, env)?;
            let f = if op == BinOp::Eq { eq } else { F::not(eq) };
            return Some(Tv { f, ty: boolean });
        }
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
    /// `decreases_step` at the end of a path of an iteration of a `for until` loop whose state
    /// is then `components` (ADR 0116 V1): the measure there is less than at the start.
    fn decreases_step(&mut self, measure: &Measure, ctx: &Ctx, components: &[Option<Tv>]) {
        let lookup = |name: &str| match measure.state.iter().position(|state| state == name) {
            Some(index) => components.get(index).cloned().flatten(),
            None => ctx.types.contains_key(name).then(|| own(ctx, name)),
        };
        let env = Env {
            vars: &lookup,
            it: None,
            result: None,
        };
        let goal = self
            .term(&measure.clause.term, &env)
            .map(|end| F::op("<", vec![end.f, measure.start.clone()]));
        self.site(
            ObligationKind::DecreasesStep,
            ctx,
            goal,
            measure.clause.text.clone(),
            measure.decreases,
            None,
        );
    }

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

    /// Puts the bindings of the declaration walked in scope, each holding itself, and records
    /// the conditions on their operations (ADR 0092).
    fn own_bindings(&mut self, ctx: &mut Ctx, uses: &'p [Use]) {
        self.conditions.clear();
        let caps = std::rc::Rc::make_mut(&mut ctx.caps);
        for usage in uses {
            caps.push((usage.cap.clone(), Some(usage.cap.clone())));
            for (op, clause) in &usage.conds {
                self.conditions
                    .insert((usage.cap.clone(), op.clone()), clause);
            }
        }
    }

    /// The `effect_condition` obligations of a site that may perform `ops` of the binding the
    /// reference `cap` holds here (ADR 0092 rule 3): for each operation the declaration
    /// walked conditions by `C` on that binding, the goal `C`, or `C′ ⇒ C` when `promised`
    /// gives the condition `C′` the site's callee or handler writes on it.
    fn effect_sites(
        &mut self,
        ctx: &Ctx,
        cap: &str,
        ops: &[String],
        promised: &dyn Fn(&mut Self, &str) -> Option<Option<F>>,
        span: Span,
        at: &EffectAt,
    ) {
        let Some(binding) = ctx.binding(cap) else {
            return;
        };
        for op in ops {
            let Some(clause) = self.conditions.get(&(binding.clone(), op.clone())).copied() else {
                continue;
            };
            let premise = promised(self, op);
            self.condition_site(ctx, &binding, op, clause, premise, span, at);
        }
    }

    /// The `effect_condition` obligation of `op` of the declaration's binding `binding`,
    /// conditioned by `clause`, at `span`: the goal `C`, or `C′ ⇒ C` with `premise`.
    #[allow(clippy::too_many_arguments)]
    fn condition_site(
        &mut self,
        ctx: &Ctx,
        binding: &str,
        op: &str,
        clause: &'p Clause,
        premise: Option<Option<F>>,
        span: Span,
        at: &EffectAt,
    ) {
        let condition = {
            let vars = |name: &str| Some(own(ctx, name));
            let env = Env {
                vars: &vars,
                it: None,
                result: None,
            };
            self.term(&clause.term, &env).map(|tv| tv.f)
        };
        // ADR 0092, amendment of 2026-10-14: the edit that the checker computed for this site
        // is kept under condition (f), when the caller's condition and the callee's, if it
        // writes one, are both within the encoding.
        let edit = match at {
            EffectAt::Call(callee) | EffectAt::With(callee)
                if condition.is_some() && !matches!(premise, Some(None)) =>
            {
                self.index
                    .condition_edits
                    .get(&(span, binding, op))
                    .filter(|edit| edit.callee == *callee)
                    .map(|edit| (edit.range, edit.replacement.clone()))
            }
            _ => None,
        };
        // A caller's condition outside the encoding is no goal that holds: the query keeps
        // `not false` and is refuted, never silently proven. A callee's outside it is
        // dropped, which only strengthens the goal.
        let goal = match premise {
            None | Some(None) => condition,
            Some(Some(premise)) => condition.map(|c| F::op("=>", vec![premise, c])),
        };
        let mut assertions: Vec<F> = ctx.hyps.iter().map(|h| h.f.clone()).collect();
        let mut place = self.place(ObligationKind::EffectCondition, goal.as_ref());
        place.effect = Some(Effect {
            operation: format!("@{binding}.{op}"),
            condition: clause.text.clone(),
            at: at.clone(),
            edit,
        });
        assertions.push(F::not(goal.unwrap_or(F::Bool(false))));
        let hypotheses = self.shown_hypotheses(&ctx.hyps);
        self.push(
            ObligationKind::EffectCondition,
            clause.text.clone(),
            span,
            clause.span.map(|span| (span, "condition")),
            Vec::new(),
            hypotheses,
            assertions,
            ctx,
            place,
            vec![ctx.divisions.to_vec()],
        );
    }

    /// The `contract_lock` obligation of `lock` in the function `name`: `C_src ⇒ C_lock`, or
    /// `C_lock` when the header writes no condition (ADR 0092 rule 8, as amended).
    fn contract_lock(&mut self, ctx: &Ctx, name: &str, lock: &'p una_core::Lock) {
        let (locked, header) = {
            let vars = |name: &str| Some(own(ctx, name));
            let env = Env {
                vars: &vars,
                it: None,
                result: None,
            };
            let locked = self.term(&lock.lock.term, &env).map(|tv| tv.f);
            let header = lock
                .header
                .as_ref()
                .and_then(|clause| self.term(&clause.term, &env))
                .map(|tv| tv.f);
            (locked, header)
        };
        // A lock condition outside the encoding is no goal that holds: the query keeps `not
        // false` and is refuted, never silently proven. A header condition outside it is
        // dropped, which only strengthens the goal.
        let unencodable = locked.is_none();
        // ADR 0092, amendment of 2026-10-15, condition (c): the edit that restores the lock is
        // kept when the lock's condition, and the header's if it writes one, are both within
        // the encoding; a header condition outside it is dropped from the premise, and a
        // conjunction with it would be dropped too.
        let edit = lock
            .edit
            .clone()
            .filter(|_| restorable(!unencodable, lock.header.as_ref().map(|_| header.is_some())));
        let goal = match header {
            Some(premise) => locked.map(|c| F::op("=>", vec![premise, c])),
            None => locked,
        };
        let mut assertions: Vec<F> = ctx.hyps.iter().map(|h| h.f.clone()).collect();
        let mut place = self.place(ObligationKind::ContractLock, goal.as_ref());
        place.effect = Some(Effect {
            operation: format!("@{}.{}", lock.cap, lock.op),
            condition: lock.lock.text.clone(),
            at: EffectAt::Lock(name.to_owned()),
            edit,
        });
        assertions.push(F::not(goal.unwrap_or(F::Bool(false))));
        let hypotheses = self.shown_hypotheses(&ctx.hyps);
        self.push(
            ObligationKind::ContractLock,
            lock.lock.text.clone(),
            lock.at,
            Some((lock.entry, "lock")),
            Vec::new(),
            hypotheses,
            assertions,
            ctx,
            place,
            vec![ctx.divisions.to_vec()],
        );
        if unencodable && let Some(obligation) = self.obligations.last_mut() {
            obligation.unencodable = Some(lock.lock.text.clone());
        }
    }

    /// Every condition of the declaration walked, charged at `span` with no premise: for a
    /// site whose effects cannot be bounded to some bindings, such as a call of a `uses of`
    /// parameter, whose argument may run any of them (ADR 0092 rule 3, as amended).
    fn every_condition(&mut self, ctx: &Ctx, span: Span, at: &EffectAt) {
        let all: Vec<(Name, String, &'p Clause)> = self
            .conditions
            .iter()
            .map(|((binding, op), clause)| (binding.clone(), op.clone(), *clause))
            .collect();
        for (binding, op, clause) in all {
            self.condition_site(ctx, &binding, &op, clause, None, span, at);
        }
    }

    /// A function value passed to a parameter whose effects run through the caller's
    /// bindings, a `uses of` parameter or one of a standard function: it is charged at the
    /// call like a call of it, unless it is a lambda, charged where it is defined (ADR 0092
    /// rule 3, as amended).
    fn passed_function(&mut self, ctx: &Ctx, atom: &Atom, value: &Tv, span: Span) {
        if let Atom::Var(name) = atom {
            if self.lambda_binders.contains(name) {
                return;
            }
            if self.uses_of.contains(name) {
                self.every_condition(ctx, span, &EffectAt::Value);
                return;
            }
        }
        if let Type::Fn { uses, .. } = &value.ty {
            for usage in uses {
                self.effect_sites(
                    ctx,
                    &usage.cap,
                    &usage.ops,
                    &|_, _| None,
                    span,
                    &EffectAt::Value,
                );
            }
        }
    }

    /// The `effect_condition` obligations of a call of the function or handler `key` with
    /// `values` for its parameters `params`: each of its bindings receives the caller's of the
    /// same name, and promises the condition it writes on each operation, `true` when none.
    fn effect_call(
        &mut self,
        ctx: &Ctx,
        key: (&'p str, &'p str),
        params: &'p [(Name, Type)],
        values: &[Tv],
        span: Span,
        at: &EffectAt,
    ) {
        let Some(uses) = self.index.uses.get(&key).copied() else {
            // A standard function has no `uses` of its own. Any other callee missing from the
            // index is charged every condition: never a silent skip.
            if !self.index.library.contains(&key) {
                self.every_condition(ctx, span, at);
            }
            return;
        };
        let by_name: BTreeMap<&str, Tv> = params
            .iter()
            .zip(values)
            .map(|((name, _), value)| (name.as_str(), value.clone()))
            .collect();
        for usage in uses {
            let promised = |walker: &mut Self, op: &str| -> Option<Option<F>> {
                let (_, clause) = usage.conds.iter().find(|(o, _)| o == op)?;
                let vars = |name: &str| by_name.get(name).cloned();
                let env = Env {
                    vars: &vars,
                    it: None,
                    result: None,
                };
                Some(walker.term(&clause.term, &env).map(|tv| tv.f))
            };
            self.effect_sites(ctx, &usage.cap, &usage.ops, &promised, span, at);
        }
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
        let mut paths_f = Vec::new();
        for (number, leaf) in leaves.iter().enumerate() {
            let selector = path_selector(number + 1);
            let path = self.path(leaf, base, result);
            assertions.push(F::eq(selector.clone(), path.clone()));
            selectors.push(selector);
            paths_f.push(path);
        }
        assertions.push(F::or(selectors));
        // A single path with an `F64` term is also asserted on its own: its selector is true,
        // so this adds nothing a model does not already satisfy, and the solver bit-blasts
        // the float facts of the path once instead of under the selector's equivalence,
        // which the pinned Z3 does not simplify first (a product's NaN fact costs 2.3M steps
        // so, and 67M under the equivalence). Queries without floats are unchanged.
        if let [path] = paths_f.as_slice()
            && path.mentions_float()
        {
            assertions.push(path.clone());
        }
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
        // ADR 0126 rule 8: a reference to a constant elaborates its value as the constant's
        // check does, with the same types, so an obligation at a position of the value is the
        // check's, which has no hypothesis; generating it again at each reference would only
        // repeat it. O(log c) for c constants.
        if !self.in_constant && inside_constant_value(&self.constant_values, source) {
            return;
        }
        // ADR 0114 rule 8.3: an obligation's query keeps the `F64` hypotheses of its cone; the
        // full query decides a refutation that is not final.
        let (assertions, sliced) = slice_floats(assertions, &self.let_defs);
        let mut full = match sliced {
            Some((full, refutation_final)) => {
                let mut full = Query {
                    assertions: full,
                    definitions: BTreeMap::new(),
                };
                let (facts, _) = self.ground_facts(&full);
                let (cone_facts, _) = self.ground_facts(&Query {
                    assertions: assertions.clone(),
                    definitions: BTreeMap::new(),
                });
                // A ground fact of a dropped term counts as a dropped constraint (rule 8.5): the
                // refutation is final only when every fact of the full query is one of the
                // cone query's.
                let cone_facts: BTreeSet<&F> = cone_facts.iter().collect();
                let refutation_final =
                    refutation_final && facts.iter().all(|fact| cone_facts.contains(fact));
                let mut all = facts;
                all.append(&mut full.assertions);
                full.assertions = all;
                self.add_definitions(&mut full);
                (!refutation_final).then_some(full)
            }
            None => None,
        };
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
        // Edition 0.4: a hypothesis shows its source text without the layout of a wrapped line,
        // as the goal does (ADR 0125, amendment of 2026-10-25). Earlier editions show it as
        // written, as before.
        let hypotheses = match self.layout {
            SourceLayout::Joined => shown_facts,
            SourceLayout::Wrapped => shown_facts.iter().map(|text| unwrap_layout(text)).collect(),
        };
        self.add_definitions(&mut query);
        let shown = self.shown(&query, ctx);
        // A shown value of a sum is read through its tag. When no assertion uses that tag, the
        // query would not declare it, and the model could not be read (#902): it is declared
        // here with its range, a fact every model of the query already satisfies.
        let used = query.applications();
        let used_full = full.as_ref().map(Query::applications);
        for item in &shown {
            if let Show::Variant { tag, .. } = &item.show
                && let F::App {
                    fun:
                        SmtFun {
                            role: FunRole::Tag { variants, .. },
                            ..
                        },
                    ..
                } = tag
                // A sum has at least one variant (`tag` gives no tag otherwise), so the range
                // is never empty and the fact never makes a satisfiable query unsatisfiable.
                && *variants >= 1
            {
                let range = F::and(vec![
                    F::op("<=", vec![F::Int("0".to_owned()), tag.clone()]),
                    F::op("<", vec![tag.clone(), F::Int(variants.to_string())]),
                ]);
                // The full query of `spec/29 §8` reads the same values, so it declares the
                // tag too.
                if let (Some(full), Some(used_full)) = (full.as_mut(), used_full.as_ref())
                    && !used_full.contains(tag)
                {
                    full.assertions.push(range.clone());
                }
                if !used.contains(tag) {
                    query.assertions.push(range);
                }
            }
        }
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
        let normalized = normalized_goal(&goal, self.layout);
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
            unencodable: None,
            full,
        });
    }

    /// Starts a body: the loops of another one made nothing fresh here (ADR 0071 D3).
    fn forget_loops(&mut self) {
        self.loop_fresh.clear();
        self.loop_collections.clear();
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
    fn forgotten<'t>(
        &self,
        terms: impl IntoIterator<Item = &'t F>,
    ) -> (Vec<(String, Collection)>, bool) {
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
                    if !names.iter().any(|(known, _)| known == source) {
                        let collection = self
                            .loop_collections
                            .get(source)
                            .copied()
                            .unwrap_or(Collection::Other);
                        names.push((source.clone(), collection));
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
                int: false,
                unbounded: None,
                unknown: None,
                effect: None,
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
            effect: None,
            guard: None,
            int: false,
            unbounded: None,
            unknown: None,
        }
    }

    /// The facts about the terms of a query that no quantifier states (`spec/12 §2, §4`): lengths are at least 0, tags are in range, and fields satisfy their
    /// declared refinements.
    fn ground_facts(&mut self, query: &Query) -> (Vec<F>, Vec<F>) {
        let mut facts: Vec<F> = Vec::new();
        let mut listed: Vec<F> = Vec::new();
        let mut seen: BTreeSet<F> = BTreeSet::new();
        let mut pending: Vec<F> = query.applications().into_iter().collect();
        // The frames and set relations of ADR 0119 V3 and V4, at the key terms of the query:
        // their `has` and `get` terms get their own ground facts below.
        for fact in instances(query) {
            let probe = Query {
                assertions: vec![fact.clone()],
                definitions: BTreeMap::new(),
            };
            pending.extend(probe.applications());
            facts.push(fact);
        }
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
                        // A rank is from 1 to 8, and a tensor's element count is the product
                        // of its lengths, by its rank: products of terms, which
                        // the model check of `spec/12 §6` and ADR 0054 handle as any other.
                        FunRole::Shape if fun.name.starts_with("i!rank!") => {
                            new.push(F::and(vec![
                                F::op("<=", vec![F::Int("1".to_owned()), term.clone()]),
                                F::op("<=", vec![term.clone(), F::Int(MAX_RANK.to_string())]),
                            ]));
                        }
                        FunRole::Shape if fun.name.starts_with("i!count!") => {
                            new.push(F::op(">=", vec![term.clone(), F::Int("0".to_owned())]));
                            if let Some(tensor) = args.first() {
                                new.extend(count_facts(&term, tensor));
                            }
                        }
                        FunRole::Len | FunRole::Shape => {
                            new.push(F::op(">=", vec![term.clone(), F::Int("0".to_owned())]));
                            // A bit set has at most `size` members (ADR 0119 V2).
                            if fun.role == FunRole::Len
                                && let Some(bits) = args.first()
                                && bits.sort() == BITSET
                            {
                                new.push(F::op("<=", vec![term.clone(), size_of(bits)]));
                            }
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
                            // A member of a bit set is in its range (ADR 0119 V2).
                            if let (Some(bits), Some(item)) = (args.first(), args.get(1))
                                && bits.sort() == BITSET
                            {
                                new.push(F::op(
                                    "=>",
                                    vec![
                                        term.clone(),
                                        F::and(vec![
                                            F::op("<=", vec![F::Int("0".to_owned()), item.clone()]),
                                            F::op("<", vec![item.clone(), size_of(bits)]),
                                        ]),
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
                        // A read of a collection is `None` exactly when the collection is
                        // empty (ADR 0119 V2); `None` is variant 1 of `Option`.
                        FunRole::Peek => {
                            if let Some(collection) = args.first() {
                                let tag = tag_of(&term, una_core::PRELUDE_MODULE, "Option", 2);
                                new.push(F::eq(
                                    F::eq(tag, F::Int("1".to_owned())),
                                    F::eq(len_of(collection), F::Int("0".to_owned())),
                                ));
                            }
                        }
                        FunRole::Predicate
                        | FunRole::Get
                        | FunRole::Element
                        | FunRole::Collection
                        | FunRole::Frame { .. }
                        | FunRole::Combine { .. }
                        | FunRole::Other => {}
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
                            label: ty.deep_label(),
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
                    let label = self.term_label(&term, ctx);
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
                            label,
                        },
                    );
                }
                _ => {}
            }
        }
        out.into_values().collect()
    }

    /// The label of a shown term: its binder's declared type, deeply, and for a field or a
    /// length, its owner's label joined with the field's declared type, deeply.
    fn term_label(&self, f: &F, ctx: &Ctx) -> una_core::Label {
        match f {
            F::Const(Const { name, .. }) => ctx
                .types
                .get(name)
                .map_or_else(una_core::Label::public, Type::deep_label),
            F::App { fun, args } => {
                let owner = args
                    .first()
                    .map_or_else(una_core::Label::public, |a| self.term_label(a, ctx));
                match &fun.role {
                    FunRole::Field {
                        module,
                        kind,
                        variant,
                        index,
                        ..
                    } => self
                        .index
                        .kinds
                        .get(&(module.as_str(), kind.as_str()))
                        .and_then(|k| k.variants.get(*variant))
                        .and_then(|v| v.fields.get(*index))
                        .map_or(owner.clone(), |(_, t)| owner.join(&t.deep_label())),
                    _ => owner,
                }
            }
            _ => una_core::Label::public(),
        }
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
        if let Type::Base {
            base: Base::Decimal(scale),
            ..
        } = &value.ty
        {
            return Show::Decimal(*scale);
        }
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
            // A float literal shows its rendering (`spec/29 §7`).
            F::Float(bits) => {
                let text = una_core::float::render(f64::from_bits(*bits));
                let level = if text.starts_with('-') { 2 } else { 1 };
                (text, level)
            }
            F::Float32(bits) => {
                let text = una_core::float32::render(f32::from_bits(*bits));
                let level = if text.starts_with('-') { 2 } else { 1 };
                (text, level)
            }
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
                    FunRole::Shape if args.len() == 2 => {
                        let axis = args.get(1).map_or_else(String::new, |a| operand(a, 10));
                        (format!("{}.{base}(axis: {axis})", first()), 1)
                    }
                    FunRole::Shape => (format!("{}.{base}", first()), 1),
                    FunRole::Element
                        if args
                            .first()
                            .is_some_and(|value| value.sort().starts_with(TENSOR)) =>
                    {
                        let shown: Vec<String> =
                            args.iter().skip(1).map(|a| operand(a, 10)).collect();
                        (format!("{}.get(at: [{}])", first(), shown.join(", ")), 1)
                    }
                    FunRole::Element => {
                        let labels: &[&str] = if args.len() == 2 {
                            &["index"]
                        } else {
                            &["row", "column"]
                        };
                        let shown: Vec<String> = args
                            .iter()
                            .skip(1)
                            .zip(labels)
                            .map(|(a, label)| format!("{label}: {}", operand(a, 10)))
                            .collect();
                        (format!("{}.get({})", first(), shown.join(", ")), 1)
                    }
                    FunRole::Tag { .. } => (format!("tag({})", first()), 1),
                    FunRole::Has | FunRole::Get => {
                        let key = args.get(1).map_or_else(String::new, |a| operand(a, 10));
                        // A set's argument is an item (`spec/28 §6, §7, §9`), a map's a key.
                        let label = match args.first() {
                            Some(set) if has_items(&set.sort()) => "item",
                            _ => "key",
                        };
                        (format!("{}.{base}({label}: {key})", first()), 1)
                    }
                    // `q.peek()`, `q.peek_back()`, `s.first()` and `s.last()` (ADR 0119 V1).
                    FunRole::Peek => {
                        let read = if base == "peekback" {
                            "peek_back"
                        } else {
                            base
                        };
                        (format!("{}.{read}()", first()), 1)
                    }
                    // `below(s)` and `same(a, b)`, and the records of V3 and V4, as calls.
                    FunRole::Collection | FunRole::Frame { .. } | FunRole::Combine { .. } => {
                        let shown: Vec<String> = args.iter().map(|a| operand(a, 10)).collect();
                        (format!("{base}({})", shown.join(", ")), 1)
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

/// `b.size` of a bit set, `i!size!i!BitSet`, uninterpreted, at least 0 (ADR 0119 V1).
fn size_of(bits: &F) -> F {
    let sort = bits.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!size!{sort}"),
            args: vec![sort],
            result: "Int".to_owned(),
            role: FunRole::Shape,
        },
        args: vec![bits.clone()],
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

/// `m.rows` or `m.columns`, `i!rows!<Sort>` or `i!columns!<Sort>` (ADR 0120 V4).
fn shape_of(value: &F, fun: ShapeFun) -> F {
    let sort = value.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!{}!{sort}", fun.as_str()),
            args: vec![sort],
            result: "Int".to_owned(),
            role: FunRole::Shape,
        },
        args: vec![value.clone()],
    }
}

/// The element count of `tensor` by its rank (ADR 0120 V4): for each r from 1 to 8, `rank = r`
/// implies `count = dim 0 × ... × dim (r - 1)`. The products of terms stay uninterpreted
/// (`spec/12 §2`), so a refutation that rests on them is model-checked as any other (ADR 0053,
/// ADR 0054). O(1) per tensor: 8 facts.
fn count_facts(count: &F, tensor: &F) -> Vec<F> {
    let rank = shape_of(tensor, ShapeFun::Rank);
    let mut product = dim_of(tensor, &F::Int("0".to_owned()));
    let mut facts = Vec::new();
    for r in 1..=MAX_RANK {
        if r > 1 {
            product = arithmetic(
                BinOp::Mul,
                product,
                dim_of(tensor, &F::Int((r - 1).to_string())),
            );
        }
        facts.push(F::op(
            "=>",
            vec![
                F::eq(rank.clone(), F::Int(r.to_string())),
                F::eq(count.clone(), product.clone()),
            ],
        ));
    }
    facts
}

/// `t.dim(axis: k)`, `i!dim!<Sort>` from the tensor and the axis (ADR 0120 V4).
fn dim_of(value: &F, axis: &F) -> F {
    let sort = value.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!dim!{sort}"),
            args: vec![sort, "Int".to_owned()],
            result: "Int".to_owned(),
            role: FunRole::Shape,
        },
        args: vec![value.clone(), axis.clone()],
    }
}

/// The most axes a tensor has (ADR 0120 rule 27).
const MAX_RANK: usize = 8;

/// The element of a dense value at its indices, `i!get!<Sort>` with the value first, then one
/// `Int` per axis, of the element's sort (ADR 0120 V4). A tensor's is `i!get!<Sort>!<r>` for r
/// indices, one function per rank.
fn element_of(args: &[F], element: &Sort) -> F {
    let sort = args.first().map_or_else(String::new, F::sort);
    let name = if sort.starts_with(TENSOR) {
        format!("i!get!{sort}!{}", args.len().saturating_sub(1))
    } else {
        format!("i!get!{sort}")
    };
    F::App {
        fun: SmtFun {
            name,
            args: args.iter().map(F::sort).collect(),
            result: element.clone(),
            role: FunRole::Element,
        },
        args: args.to_vec(),
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

/// A read of a collection that gives an `Option` of an item, `i!<read>!<Sort>`, of the option
/// sort `option` (ADR 0119 V1): `peek`, `peekback`, `first` or `last`.
fn read_of(value: &F, read: &str, option: Sort) -> F {
    let sort = value.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!{read}!{sort}"),
            args: vec![sort],
            result: option,
            role: FunRole::Peek,
        },
        args: vec![value.clone()],
    }
}

/// `below(s)`, the stack under the top of the stack `s`, `i!below!<Sort>` (ADR 0119 V1).
fn below_of(stack: &F) -> F {
    let sort = stack.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!below!{sort}"),
            args: vec![sort.clone()],
            result: sort,
            role: FunRole::Collection,
        },
        args: vec![stack.clone()],
    }
}

/// `same(a, b)`, the result of `a.same_entries(as: b)` or `a.same_items(as: b)`,
/// `i!same!<Sort>`, uninterpreted (ADR 0119 rule 11, V1): never the solver's equality.
fn same_of(a: &F, b: &F) -> F {
    let sort = a.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!same!{sort}"),
            args: vec![sort.clone(), b.sort()],
            result: "Bool".to_owned(),
            role: FunRole::Collection,
        },
        args: vec![a.clone(), b.clone()],
    }
}

/// `frame(s, s', x)`: `s'` is `s` updated at the key `x` only (ADR 0119 V3), with `get` the
/// option sort of a map's `get`.
fn frame_of(old: &F, new: &F, key: &F, get: Option<Sort>) -> F {
    let sort = old.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!frame!{sort}"),
            args: vec![sort.clone(), new.sort(), key.sort()],
            result: "Bool".to_owned(),
            role: FunRole::Frame { get },
        },
        args: vec![old.clone(), new.clone(), key.clone()],
    }
}

/// The relation `op` between the sets `args`, over items of the sort `item` (ADR 0119 V4):
/// `union(s, t, r)` and the others, `subset(s, t)`.
fn combine_of(op: Combine, args: &[F], item: Sort) -> F {
    let sort = args.first().map_or_else(String::new, F::sort);
    F::App {
        fun: SmtFun {
            name: format!("i!{}!{sort}", op.as_str()),
            args: args.iter().map(F::sort).collect(),
            result: "Bool".to_owned(),
            role: FunRole::Combine { op, item },
        },
        args: args.to_vec(),
    }
}

/// The facts of the records of ADR 0119 V3 and V4 in `query`, each instantiated at every key
/// term of its key or item sort, other than the updated key itself: for a frame `frame(s, s',
/// x)` and a key term `y`, `frame(s, s', x) and y != x => has(s', y) = has(s, y)`, and the
/// same of `get` for a map; for a set relation, its membership law at `y`. Each fact holds of
/// every run, since a run makes its record true and the law holds of the operation; `y` and
/// `x` may denote the same key, so a frame says nothing at `y` unless the query proves them
/// distinct (V3). No quantifier enters the query.
///
/// The key terms are the keys and items that the query looks up, the second argument of a
/// `has` or a `get`, and the keys of the frames. A term looked up nowhere gains nothing from an
/// instance: every `has` and `get` at it would occur only in the instances, which a model
/// satisfies by making each `has` false and each `get` equal along the frames, so leaving it
/// out loses no proof. They are read from the query before these facts, so a query with k
/// records and t key terms gains O(k · t) facts (V8).
fn instances(query: &Query) -> Vec<F> {
    let applications = query.applications();
    let mut records: Vec<&F> = Vec::new();
    let mut keys: BTreeMap<Sort, BTreeSet<&F>> = BTreeMap::new();
    for term in &applications {
        let F::App { fun, args } = term else {
            continue;
        };
        let key = match fun.role {
            FunRole::Has | FunRole::Get => args.get(1),
            FunRole::Frame { .. } => {
                records.push(term);
                args.get(2)
            }
            FunRole::Combine { .. } => {
                records.push(term);
                None
            }
            _ => None,
        };
        if let Some(key) = key {
            keys.entry(key.sort()).or_default().insert(key);
        }
    }
    let none = BTreeSet::new();
    let mut facts = Vec::new();
    for record in records {
        let F::App { fun, args } = record else {
            continue;
        };
        match (&fun.role, args.as_slice()) {
            (FunRole::Frame { get }, [old, new, key]) => {
                for &y in keys.get(&key.sort()).unwrap_or(&none) {
                    if y == key {
                        continue;
                    }
                    let guard = F::and(vec![record.clone(), F::not(F::eq(y.clone(), key.clone()))]);
                    facts.push(F::op(
                        "=>",
                        vec![guard.clone(), F::eq(has_of(new, y), has_of(old, y))],
                    ));
                    if let Some(option) = get {
                        facts.push(F::op(
                            "=>",
                            vec![guard, F::eq(get_of(new, y, option), get_of(old, y, option))],
                        ));
                    }
                }
            }
            (FunRole::Combine { op, item }, [s, t, rest @ ..]) => {
                for &y in keys.get(item).unwrap_or(&none) {
                    let (in_s, in_t) = (has_of(s, y), has_of(t, y));
                    let law = match (op, rest) {
                        (Combine::Union, [r]) => F::eq(has_of(r, y), F::or(vec![in_s, in_t])),
                        (Combine::Intersection, [r]) => {
                            F::eq(has_of(r, y), F::and(vec![in_s, in_t]))
                        }
                        (Combine::Difference, [r]) => {
                            F::eq(has_of(r, y), F::and(vec![in_s, F::not(in_t)]))
                        }
                        (Combine::Subset, []) => F::op("=>", vec![in_s, in_t]),
                        _ => continue,
                    };
                    facts.push(F::op("=>", vec![record.clone(), law]));
                }
            }
            _ => {}
        }
    }
    facts
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
    within(value, int.least(), int.greatest())
}

/// `least <= value <= greatest`.
fn within(value: F, least: i128, greatest: i128) -> F {
    F::and(vec![
        F::op("<=", vec![F::Int(least.to_string()), value.clone()]),
        F::op("<=", vec![value, F::Int(greatest.to_string())]),
    ])
}

/// The least value of `Int` that stays in a machine word, `-2^62`: in a `systems` package, no
/// result of `Int` arithmetic leaves the tagged range (ADR 0109 rule 3, `spec/18 §3`).
const INT_LEAST: i128 = -(1 << 62);

/// The greatest value of `Int` that stays in a machine word, `2^62 - 1` (ADR 0109 rule 3).
const INT_GREATEST: i128 = (1 << 62) - 1;

/// Whether the integer literal `digits`, in decimal with an optional `-`, lies in the tagged
/// range of `Int`. A literal too long for 128 bits lies outside it.
fn fits_word(digits: &str) -> bool {
    digits
        .parse::<i128>()
        .is_ok_and(|value| (INT_LEAST..=INT_GREATEST).contains(&value))
}

/// What an `overflow` obligation is on, which decides its repairs (`spec/18 §3`).
#[derive(Clone, Copy, Debug)]
enum OverflowOf<'a> {
    /// A fixed-width type.
    Fixed,
    /// `Int` in a `systems` package (ADR 0109 rule 3).
    Int,
    /// The result of the named prelude function in a `systems` package, which states no bound
    /// on it.
    Unbounded(&'a str),
}

/// The guard that states the `overflow` goal of the arithmetic primitive `prim` with its
/// operands as `operand` writes them, as in `a + b in -128..127` (`spec/18 §3`).
fn operation_guard(prim: Prim, operand: &dyn Fn(usize) -> String, range: &str) -> Option<String> {
    let symbol = match prim {
        Prim::Neg => return Some(format!("-{} in {range}", operand(0))),
        Prim::Add => "+",
        Prim::Sub => "-",
        Prim::Mul => "*",
        Prim::Div => "/",
        Prim::Mod => "mod",
        _ => return None,
    };
    Some(format!("{} {symbol} {} in {range}", operand(0), operand(1)))
}

/// The guard of an `Int` operation of a `systems` package, with its operands as `operand`
/// writes them (ADR 0109 rule 3): it bounds the operands, never the result, since a test on
/// the exact result is itself an `Int` operation whose obligation could not be proven. For `+`
/// and `-`, each operand in -2^61..2^61 - 1; for `*`, each in -(2^31 - 1)..2^31 - 1; for `/`,
/// the dividend in -(2^62 - 1)..2^62 - 1, the divisor unbounded; for `mod`, both operands in
/// that range, since the result is bounded by the divisor; for unary `-`, the operand in
/// -(2^62 - 1)..2^62 - 1. An operand that is a
/// literal within its bound needs no test; `None` when no operand does.
fn int_guard(
    prim: Prim,
    operand: &dyn Fn(usize) -> String,
    literal: &dyn Fn(usize) -> Option<i128>,
) -> Option<String> {
    let bounded = |bound: i128| -> Option<String> {
        let tests: Vec<String> = (0..2)
            .filter(|i| literal(*i).is_none_or(|value| value.abs() > bound))
            .map(|i| format!("{} in {}..{}", operand(i), -bound - 1, bound))
            .collect();
        (!tests.is_empty()).then(|| tests.join(" and "))
    };
    match prim {
        Prim::Add | Prim::Sub => {
            // -2^61..2^61 - 1.
            let half = (1i128 << 61) - 1;
            bounded(half)
        }
        Prim::Mul => {
            let bound = (1i128 << 31) - 1;
            let tests: Vec<String> = (0..2)
                .filter(|i| literal(*i).is_none_or(|value| value.abs() > bound))
                .map(|i| format!("{} in -{bound}..{bound}", operand(i)))
                .collect();
            (!tests.is_empty()).then(|| tests.join(" and "))
        }
        // The dividend, or the operand of `-`, off the least value; the divisor of `/` needs
        // no bound, since the quotient's magnitude never exceeds the dividend's.
        Prim::Div | Prim::Neg => Some(off_least(&operand(0))),
        // `a mod b` is bounded by `|b|`, not by `a` (`-1 mod (2^62 + 5)` is `2^62 + 4`), so both
        // operands are bounded.
        Prim::Mod => Some(format!(
            "{} and {}",
            off_least(&operand(0)),
            off_least(&operand(1))
        )),
        _ => None,
    }
}

/// The guard that keeps `operand` in the tagged range but off its least value, -2^62, where
/// `-`, `abs` and a division by -1 leave it (ADR 0109 rule 3).
fn off_least(operand: &str) -> String {
    format!("{operand} in {}..{INT_GREATEST}", INT_LEAST + 1)
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

/// The cone of `spec/29 §8` (ADR 0114 rule 8.3) over `assertions`, `Π` and the negated goal
/// last: every hypothesis is split into its conjuncts; an `F64` conjunct is kept when it is
/// connected to the goal's `F64` terms through shared `F64` binders, transitively; a dropped
/// constraint is kept with each float atom replaced by a fresh `Bool`, and a dropped definition
/// (a selfified `let` of `let_defs`) is left out. `None` when nothing is dropped, so the query
/// is unchanged; otherwise the full assertions, and whether a refutation of the cone query is
/// final: no constraint was abstracted, and no dropped definition's binder occurs in a kept
/// hypothesis or the goal. The union-find makes it O(|Π| α(|Π|)) in the terms of `Π`.
fn slice_floats(assertions: Vec<F>, let_defs: &BTreeSet<F>) -> (Vec<F>, Option<(Vec<F>, bool)>) {
    if !assertions.iter().any(F::mentions_float) {
        return (assertions, None);
    }
    let Some((goal, hyps)) = assertions.split_last() else {
        return (assertions, None);
    };
    let parts: Vec<&F> = hyps.iter().flat_map(conjuncts).collect();
    // The `F64`-sorted binders of a term: constants and applications, by their text.
    fn float_binders(f: &F, out: &mut Vec<String>) {
        let mut terms = Vec::new();
        f.visit(&mut terms);
        for term in terms {
            if matches!(term, F::Const(_) | F::App { .. })
                && crate::smt::is_float_sort(&term.sort())
            {
                out.push(term.text());
            }
        }
    }
    let mut ids: BTreeMap<String, usize> = BTreeMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while let Some(&p) = parent.get(x) {
            if p == x {
                break;
            }
            let grand = parent.get(p).copied().unwrap_or(p);
            if let Some(slot) = parent.get_mut(x) {
                *slot = grand;
            }
            x = p;
        }
        x
    }
    let id = |name: String, ids: &mut BTreeMap<String, usize>, parent: &mut Vec<usize>| {
        *ids.entry(name).or_insert_with(|| {
            parent.push(parent.len());
            parent.len() - 1
        })
    };
    let mut part_ids: Vec<Vec<usize>> = Vec::with_capacity(parts.len());
    for part in &parts {
        let mut names = Vec::new();
        if part.mentions_float() {
            float_binders(part, &mut names);
        }
        let mine: Vec<usize> = names
            .into_iter()
            .map(|name| id(name, &mut ids, &mut parent))
            .collect();
        if let Some((&first, rest)) = mine.split_first() {
            for &other in rest {
                let (a, b) = (find(&mut parent, first), find(&mut parent, other));
                if let Some(slot) = parent.get_mut(a) {
                    *slot = b;
                }
            }
        }
        part_ids.push(mine);
    }
    let mut goal_names = Vec::new();
    float_binders(goal, &mut goal_names);
    let roots: BTreeSet<usize> = goal_names
        .into_iter()
        .filter_map(|name| ids.get(&name).copied())
        .map(|i| find(&mut parent, i))
        .collect();
    let mut kept: Vec<F> = Vec::new();
    let mut dropped_definitions: Vec<&F> = Vec::new();
    let mut abstracted = false;
    let mut dropped = false;
    let mut atoms: BTreeMap<F, F> = BTreeMap::new();
    for (part, mine) in parts.iter().zip(&part_ids) {
        if !part.mentions_float() {
            kept.push((*part).clone());
            continue;
        }
        let in_cone = mine.iter().any(|&i| roots.contains(&find(&mut parent, i)));
        if in_cone {
            kept.push((*part).clone());
            continue;
        }
        dropped = true;
        if let_defs.contains(*part) {
            dropped_definitions.push(*part);
        } else {
            abstracted = true;
            kept.push(abstract_atoms(part, &mut atoms));
        }
    }
    if !dropped {
        return (assertions, None);
    }
    let mut used: Vec<&F> = Vec::new();
    for f in kept.iter().chain(std::iter::once(goal)) {
        f.visit(&mut used);
    }
    let used: BTreeSet<String> = used
        .into_iter()
        .filter(|t| matches!(t, F::Const(_)))
        .map(F::text)
        .collect();
    let definitions_unused = dropped_definitions
        .iter()
        .all(|definition| match definition {
            F::Op { op: "=", args } => args
                .first()
                .is_some_and(|binder| !used.contains(&binder.text())),
            _ => false,
        });
    let refutation_final = !abstracted && definitions_unused;
    kept.push(goal.clone());
    (kept, Some((assertions, refutation_final)))
}

/// `f` with each float atom, a maximal `Bool` subterm with an `F64` term, replaced by a fresh
/// `Bool` binder `i!v!fa!<n>`, one per distinct atom, numbered in order of first occurrence
/// across a query (ADR 0114 rule 8.3). The Boolean structure stays: `and`, `or`, `not`,
/// `=>`, and `=` and `ite` over `Bool`.
fn abstract_atoms(f: &F, atoms: &mut BTreeMap<F, F>) -> F {
    if !f.mentions_float() {
        return f.clone();
    }
    if let F::Op { op, args } = f
        && matches!(*op, "and" | "or" | "not" | "=>" | "=" | "ite")
        && args.iter().all(|a| a.sort() == "Bool")
    {
        return F::Op {
            op,
            args: args.iter().map(|a| abstract_atoms(a, atoms)).collect(),
        };
    }
    let next = atoms.len();
    atoms
        .entry(f.clone())
        .or_insert_with(|| {
            F::Const(Const {
                name: format!("fa!{next}"),
                sort: "Bool".to_owned(),
                role: ConstRole::Internal,
            })
        })
        .clone()
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
fn instantiate(clause: &Clause, value: &str) -> String {
    written_text(clause, &|t| matches!(t, Term::It).then(|| value.to_owned()))
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

/// The normalized goal of `spec/12 §8`, from which an obligation's id is computed. Editions 0.1
/// to 0.3 ([`SourceLayout::Joined`]) collapse its white space, exactly as before soft wrapping,
/// so their ids are unchanged by construction; edition 0.4 also erases the layout of a wrapped
/// line ([`unwrap_layout`]).
fn normalized_goal(goal: &str, layout: SourceLayout) -> String {
    match layout {
        SourceLayout::Joined => goal.split_whitespace().collect::<Vec<_>>().join(" "),
        SourceLayout::Wrapped => unwrap_layout(goal),
    }
}

/// Edition 0.4's normalization of source text in an obligation (`spec/12 §8`, ADR 0125,
/// amendment of 2026-10-25): white space that holds a line break right before a closing `)` or
/// `]` token is erased, which is where the canonical form puts the closer of a wrapped group,
/// and then every run of white space collapses to one space. Text literals hold no line break,
/// and a closer inside one is no token, so their text is kept but for the collapse, which also joins runs of spaces inside a text literal
/// shown in a hypothesis or contract: the text is for display and comparison, never evaluated
/// (#1175, kept as it is). One lexing
/// pass and one pass over the closers: O(n) time and space.
pub fn unwrap_layout(text: &str) -> String {
    let mut sources = SourceDb::new();
    let file = sources.add("<obligation-text>", text.as_bytes());
    let source = sources.file(file);
    let mut interner = Interner::new();
    let lexed = lex_in(source, file, &mut interner, KeywordSet::V0_5);
    let closers = lexed
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::RParen | TokenKind::RBracket))
        .map(|token| token.span.start);
    let text = source.text();
    let mut unwrapped = String::with_capacity(text.len());
    let mut previous = 0;
    for closer in closers {
        // INVARIANT: token starts ascend and fall on character boundaries of `text`, so `get`
        // never refuses; were it to, `previous` stays and the text is copied with a later range.
        let Some(prefix) = text.get(previous..closer) else {
            continue;
        };
        let kept = prefix.trim_end_matches(char::is_whitespace);
        let trailing = prefix.get(kept.len()..).unwrap_or_default();
        unwrapped.push_str(if trailing.contains('\n') {
            kept
        } else {
            prefix
        });
        previous = closer;
    }
    unwrapped.push_str(text.get(previous..).unwrap_or_default());
    unwrapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `ob-` and the first 12 hex digits of SHA-256 over the module path, the declaration, the
/// kind and the normalized goal (`spec/12 §8`).
fn obligation_id(module: &str, declaration: &str, kind: ObligationKind, goal: &str) -> String {
    let digest =
        Sha256::digest(format!("{module}\n{declaration}\n{}\n{goal}", kind.as_str()).as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("ob-{}", hex.get(..12).unwrap_or(&hex))
}

/// ADR 0092, amendment of 2026-10-15, condition (c): whether the edit that restores a lock is
/// kept, given whether the lock's condition is within the encoding and, when the header
/// writes a condition, whether that one is. A header condition outside it is dropped from the
/// premise, so a conjunction with it would be dropped too, and the obligation stay refuted.
fn restorable(locked: bool, header: Option<bool>) -> bool {
    locked && header.unwrap_or(true)
}

/// A negated integer literal, `(- 2)`, as the literal `-2`, so that alignment and division by
/// it stay literal; any other term as it is.
fn literal_int(f: F) -> F {
    if let F::Op { op: "-", args } = &f
        && let [F::Int(digits)] = args.as_slice()
    {
        return match digits.strip_prefix('-') {
            Some(positive) => F::Int(positive.to_owned()),
            None if digits == "0" => F::Int("0".to_owned()),
            None => F::Int(format!("-{digits}")),
        };
    }
    f
}

/// The position of `TowardZero` among the variants of `Rounding`.
const TOWARD_ZERO: usize = 3;

/// `f` times the literal 10^k, a coefficient aligned to a scale k larger (ADR 0120 V2); a
/// literal coefficient stays a literal.
fn decimal_aligned(f: F, k: u8) -> F {
    if k == 0 {
        return f;
    }
    if let F::Int(text) = &f {
        // Ten to the k appends k zeros to the digits of a nonzero literal.
        if text
            .trim_start_matches('-')
            .trim_start_matches('0')
            .is_empty()
        {
            return f;
        }
        return F::Int(format!("{text}{}", "0".repeat(usize::from(k))));
    }
    F::op("*", vec![F::Int(pow10(k)), f])
}

/// 10^k in decimal digits, a literal of a decimal alignment (ADR 0120 V2).
fn pow10(k: u8) -> String {
    una_core::decimal::pow10(u32::from(k)).to_string()
}

/// The magnitude form of ADR 0120 V2: the exact quotient `n / d` rounded to an integer by the
/// `Rounding` whose tag is `mode`, for `d` not zero. The magnitudes are divided, which floor,
/// truncating and Euclidean division agree on, so SMT-LIB's `div` and `mod` (or `i!div!Int`
/// and `i!mod!Int` for a divisor that is no literal) give the quotient q and the remainder r;
/// the rounded magnitude is q or q + 1 by the mode's case on r, written linearly, and the sign
/// is negative exactly when one of `n` and `d` is negative and `n` is not zero. The run time
/// of `una_core::decimal::round_div` computes the same value.
fn rounded_quotient(n: F, d: F, mode: &F) -> F {
    let zero = || F::Int("0".to_owned());
    let magnitude = |x: F| {
        F::op(
            "ite",
            vec![
                F::op(">=", vec![x.clone(), zero()]),
                x.clone(),
                F::op("-", vec![x]),
            ],
        )
    };
    let an = magnitude(n.clone());
    // A literal divisor's magnitude is a literal, so its quotient stays linear.
    let ad = match &d {
        F::Int(text) => F::Int(text.trim_start_matches('-').to_owned()),
        _ => magnitude(d.clone()),
    };
    let negative = F::and(vec![
        F::not(F::eq(n.clone(), zero())),
        F::not(F::eq(
            F::op("<", vec![n, zero()]),
            F::op("<", vec![d, zero()]),
        )),
    ]);
    let q = arithmetic(BinOp::Div, an.clone(), ad.clone());
    let r = arithmetic(BinOp::Mod, an, ad.clone());
    let nonzero = F::op(">", vec![r.clone(), zero()]);
    let twice = F::op("*", vec![F::Int("2".to_owned()), r]);
    let above = F::op(">", vec![twice.clone(), ad.clone()]);
    let tie = F::eq(twice, ad);
    let odd = F::eq(
        F::op("mod", vec![q.clone(), F::Int("2".to_owned())]),
        F::Int("1".to_owned()),
    );
    // In the order of `Rounding::ALL`.
    let ups = [
        F::or(vec![above.clone(), F::and(vec![tie.clone(), odd])]),
        F::or(vec![above.clone(), tie]),
        above,
        F::Bool(false),
        nonzero.clone(),
        F::and(vec![nonzero.clone(), F::not(negative.clone())]),
        F::and(vec![nonzero, negative.clone()]),
    ];
    let mut up = F::Bool(false);
    for (index, case) in ups.into_iter().enumerate().rev() {
        up = F::op(
            "ite",
            vec![F::eq(mode.clone(), F::Int(index.to_string())), case, up],
        );
    }
    let rounded = F::op(
        "ite",
        vec![up, F::op("+", vec![q.clone(), F::Int("1".to_owned())]), q],
    );
    F::op(
        "ite",
        vec![negative, F::op("-", vec![rounded.clone()]), rounded],
    )
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
            uses_of: Vec::new(),
            locks: Vec::new(),
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
                constants: Vec::new(),
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
        let obligations = super::obligations(&first, &sources, SourceLayout::Joined).unwrap();
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
        let obligations = super::obligations(&second, &sources, SourceLayout::Joined).unwrap();
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

    /// `f(a: Int, b: Int) -> Int` whose body computes `a + b`, binds the literal
    /// `4611686018427387904`, negates `a` where the source writes `-x` and the literal `5`
    /// where it writes `-5`, and returns `a + b`, in a package of `profile`.
    fn int_program(text: &str, file: una_syntax::FileId, profile: Profile) -> Program {
        let at = |part: &str| {
            let start = text.find(part).unwrap();
            Span::new(file, start, start + part.len())
        };
        let int = || base(Base::Int);
        let sum = at("a + b");
        let (a, b) = (
            Span::new(file, sum.start, sum.start + 1),
            Span::new(file, sum.end - 1, sum.end),
        );
        let neg = at("-x");
        let x = Span::new(file, neg.start + 1, neg.end);
        let small = at("-5");
        let five = Span::new(file, small.start + 1, small.end);
        let var = |name: &str| Atom::Var(name.to_owned());
        let bind = |name: &str, value: Rhs, span: Span, body: Expr| Expr::Let {
            name: name.to_owned(),
            ty: int(),
            value: Box::new(Bound::Rhs(value)),
            body: Box::new(body),
            span,
            written: false,
        };
        let body = bind(
            "%1",
            Rhs::Prim {
                prim: Prim::Add,
                args: vec![var("a"), var("b")],
                arg_spans: vec![a, b],
                span: sum,
            },
            sum,
            bind(
                "%2",
                Rhs::Atom(Atom::Int("4611686018427387904".to_owned())),
                at("4611686018427387904"),
                bind(
                    "%3",
                    Rhs::Prim {
                        prim: Prim::Neg,
                        args: vec![var("a")],
                        arg_spans: vec![x],
                        span: neg,
                    },
                    neg,
                    bind(
                        "%4",
                        Rhs::Prim {
                            prim: Prim::Neg,
                            args: vec![Atom::Int("5".to_owned())],
                            arg_spans: vec![five],
                            span: small,
                        },
                        small,
                        Expr::Atom {
                            atom: var("%1"),
                            span: sum,
                        },
                    ),
                ),
            ),
        );
        Program {
            profile,
            modules: vec![una_core::Module {
                path: "m".to_owned(),
                decls: vec![Decl::Fun(Box::new(Fun {
                    name: "f".to_owned(),
                    type_params: Vec::new(),
                    constraints: Vec::new(),
                    params: vec![("a".to_owned(), int()), ("b".to_owned(), int())],
                    result: int(),
                    uses: Vec::new(),
                    uses_of: Vec::new(),
                    locks: Vec::new(),
                    fails: None,
                    error: None,
                    requires: Vec::new(),
                    ensures: Vec::new(),
                    body,
                    span: Span::new(file, 0, 1),
                    public: false,
                }))],
                examples: Vec::new(),
                constants: Vec::new(),
            }],
            ..Program::default()
        }
    }

    #[test]
    fn int_arithmetic_of_a_systems_package_has_overflow_obligations() {
        // ADR 0109 rule 3, `spec/18 §3`: in `systems`, `a + b`, `-x` and a literal outside the
        // tagged range each carry an `overflow` obligation, with the goal `<exact> in
        // -4611686018427387904..4611686018427387903` and a guard that bounds the operands;
        // the literal's goal is ground, and `-5` is a literal in range, with none. In `app`,
        // none of them carries one (rule 2.1).
        let text = "a + b\n4611686018427387904\n-x\n-5\n";
        let mut sources = SourceDb::new();
        let file = sources.add("m.una", text.as_bytes());
        let range = "-4611686018427387904..4611686018427387903";
        let systems = int_program(text, file, Profile::Systems);
        let obligations = super::obligations(&systems, &sources, SourceLayout::Joined).unwrap();
        let overflow: Vec<(&str, Option<&str>, bool, bool)> = obligations
            .iter()
            .filter(|o| o.kind == ObligationKind::Overflow)
            .map(|o| {
                (
                    o.goal.as_str(),
                    o.site.guard.as_deref(),
                    o.site.int,
                    o.site.ground,
                )
            })
            .collect();
        let sum_goal = format!("a + b in {range}");
        let literal_goal = format!("4611686018427387904 in {range}");
        let neg_goal = format!("-x in {range}");
        // The guards bound the operands, never the result (ADR 0109 rule 3).
        let half = "-2305843009213693952..2305843009213693951";
        let sum_guard = format!("a in {half} and b in {half}");
        let neg_guard = "x in -4611686018427387903..4611686018427387903";
        assert_eq!(
            overflow,
            [
                (sum_goal.as_str(), Some(sum_guard.as_str()), true, false),
                (literal_goal.as_str(), None, true, true),
                (neg_goal.as_str(), Some(neg_guard), true, false),
            ]
        );
        // The literal's query is `not (least <= 2^62 <= greatest)`, refuted by any model.
        let literal = obligations.iter().find(|o| o.goal == literal_goal).unwrap();
        assert!(
            literal.query.text().contains("4611686018427387904"),
            "{}",
            literal.query.text()
        );
        // The repairs of an `Int` site are rule 3's, the guard first.
        let refuted = crate::run::Outcome {
            status: una_diag::ObligationStatus::Refuted,
            reason: None,
            cached: false,
            path: None,
            bindings: Vec::new(),
            division: None,
            values_error: None,
        };
        let sum = obligations.iter().find(|o| o.goal == sum_goal).unwrap();
        let diagnostics = crate::report::diagnostics(
            std::slice::from_ref(sum),
            std::slice::from_ref(&refuted),
            &sources,
        );
        let repairs: Vec<&str> = diagnostics[0]
            .repairs
            .iter()
            .map(|r| r.description.as_str())
            .collect();
        let guard = format!("guard the operation with `if {sum_guard}`");
        assert_eq!(
            repairs,
            [
                guard.as_str(),
                "bound the operands with a refined kind, as `kind Small = Int where it >= 0 and it <= 1000` for a parameter or a field, so that the exact result stays in the range",
                "give the values a fixed-width type, as `I64`, converting them explicitly",
                "if wrapping or saturating is meant, give the values a fixed-width type and use its `wrapping_add`, `saturating_add` or their siblings",
            ]
        );
        // A ground goal has no repair (R-021) and no counterexample.
        let diagnostics = crate::report::diagnostics(
            std::slice::from_ref(literal),
            std::slice::from_ref(&refuted),
            &sources,
        );
        assert_eq!(diagnostics[0].code, una_diag::Code::U5001);
        assert!(diagnostics[0].repairs.is_empty());
        assert!(diagnostics[0].counterexample.is_none());
        let app = int_program(text, file, Profile::App);
        let obligations = super::obligations(&app, &sources, SourceLayout::Joined).unwrap();
        assert!(
            obligations
                .iter()
                .all(|o| o.kind != ObligationKind::Overflow),
            "{:?}",
            obligations.iter().map(|o| &o.goal).collect::<Vec<_>>()
        );
    }

    #[test]
    fn abs_as_a_function_value_is_unknown_with_no_query() {
        // ADR 0109 rule 3: `abs` used as a value in a `systems` package, the lambda that stands
        // for it at the reference, has an `overflow` obligation unknown by construction: its
        // reason and its one repair are fixed, and `verify` sends no query, so a solver that
        // does not exist is never started. In `app` there is none.
        let text = "xs.map(each: abs)\n";
        let mut sources = SourceDb::new();
        let file = sources.add("m.una", text.as_bytes());
        let start = text.find("abs").unwrap();
        let at = Span::new(file, start, start + 3);
        let int = || base(Base::Int);
        let program = |profile: Profile| Program {
            profile,
            modules: vec![una_core::Module {
                path: "m".to_owned(),
                decls: vec![Decl::Fun(Box::new(Fun {
                    name: "f".to_owned(),
                    type_params: Vec::new(),
                    constraints: Vec::new(),
                    params: Vec::new(),
                    result: base(Base::Unit),
                    uses: Vec::new(),
                    uses_of: Vec::new(),
                    locks: Vec::new(),
                    fails: None,
                    error: None,
                    requires: Vec::new(),
                    ensures: Vec::new(),
                    body: Expr::Let {
                        name: "%1".to_owned(),
                        ty: base(Base::Unit),
                        value: Box::new(Bound::Rhs(Rhs::Lambda {
                            params: vec![("x".to_owned(), int())],
                            body: Box::new(Expr::Atom {
                                atom: Atom::Var("x".to_owned()),
                                span: at,
                            }),
                            function: Some(una_core::Path {
                                module: una_core::PRELUDE_MODULE.to_owned(),
                                name: "abs".to_owned(),
                            }),
                        })),
                        body: Box::new(Expr::Atom {
                            atom: Atom::Unit,
                            span: at,
                        }),
                        span: at,
                        written: false,
                    },
                    span: Span::new(file, 0, 1),
                    public: false,
                }))],
                examples: Vec::new(),
                constants: Vec::new(),
            }],
            ..Program::default()
        };
        let systems = program(Profile::Systems);
        let obligations = super::obligations(&systems, &sources, SourceLayout::Joined).unwrap();
        let reason = "the operands of `abs` have no bounds at this use";
        let repair =
            "write the call inside a lambda, `fn(x) -> abs(x)`, so its operand's bounds apply";
        let overflow: Vec<&Obligation> = obligations
            .iter()
            .filter(|o| o.kind == ObligationKind::Overflow)
            .collect();
        assert_eq!(overflow.len(), 1, "{obligations:?}");
        assert_eq!(overflow[0].source, at);
        assert_eq!(overflow[0].site.unknown.as_deref(), Some(reason));
        assert_eq!(overflow[0].site.flow.as_deref(), Some(repair));
        // No solver can run: the obligation unknown by construction must send no query.
        #[derive(Debug)]
        struct Unavailable;
        impl crate::solver::Solver for Unavailable {
            fn version(&self) -> Result<String, String> {
                Err("no solver in this test".to_owned())
            }
            fn describe(&self) -> String {
                "no solver".to_owned()
            }
            fn library_digest(&self) -> String {
                String::new()
            }
            fn run(
                &self,
                _: &str,
                _: std::time::Duration,
                _: Option<&std::sync::atomic::AtomicBool>,
            ) -> Result<crate::solver::Raw, String> {
                Err("no solver in this test".to_owned())
            }
        }
        let settings = crate::run::Settings {
            solver: std::sync::Arc::new(Unavailable),
            lock: crate::run::Lock {
                text: String::new(),
                solver_version: "0.0.0".to_owned(),
                step_budget: 1,
                random_seed: 0,
            },
            wallclock: std::time::Duration::from_secs(1),
            cache: None,
            cancel: None,
        };
        let outcomes = crate::run::verify(&obligations, &settings).unwrap();
        assert_eq!(outcomes.len(), obligations.len());
        assert!(outcomes.iter().all(|o| !o.cached));
        let index = obligations
            .iter()
            .position(|o| o.kind == ObligationKind::Overflow)
            .unwrap();
        assert_eq!(outcomes[index].status, una_diag::ObligationStatus::Unknown);
        assert_eq!(outcomes[index].reason.as_deref(), Some(reason));
        let diagnostics = crate::report::diagnostics(
            std::slice::from_ref(overflow[0]),
            std::slice::from_ref(&outcomes[index]),
            &sources,
        );
        assert_eq!(diagnostics[0].code, una_diag::Code::U5002);
        assert_eq!(
            diagnostics[0].message,
            format!(
                "overflow obligation `abs in -4611686018427387904..4611686018427387903` could not be proven ({reason})"
            )
        );
        let repairs: Vec<&str> = diagnostics[0]
            .repairs
            .iter()
            .map(|r| r.description.as_str())
            .collect();
        assert_eq!(repairs, [repair]);
        let app = program(Profile::App);
        let obligations = super::obligations(&app, &sources, SourceLayout::Joined).unwrap();
        assert!(
            obligations
                .iter()
                .all(|o| o.kind != ObligationKind::Overflow)
        );
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
                constants: Vec::new(),
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
    fn a_kind_core_does_not_declare_is_recorded_at_every_lookup() {
        // #407: a `Named` that resolves to no declaration, no table of the prelude and no
        // opaque kind of the prelude is recorded at each lookup, never skipped silently.
        let program = Program {
            modules: vec![una_core::Module {
                path: "m".to_owned(),
                decls: vec![Decl::Data {
                    name: "Here".to_owned(),
                    params: Vec::new(),
                    variants: vec![DataVariant {
                        name: "Only".to_owned(),
                        fields: Vec::new(),
                    }],
                    record: false,
                }],
                examples: Vec::new(),
                constants: Vec::new(),
            }],
            ..Program::default()
        };
        let named = |module: &str, name: &str| Type::Base {
            base: Base::Named {
                module: module.to_owned(),
                name: name.to_owned(),
                args: Vec::new(),
            },
            refinements: Vec::new(),
            label: una_core::Label::public(),
        };
        type Lookup<'l> = &'l dyn Fn(&Index<'_>, &str, &str);
        let lookups: [Lookup<'_>; 4] = [
            &|index, module, name| {
                index.prelude_kind(module, name);
            },
            &|index, module, name| {
                index.variant_names(module, name);
            },
            &|index, module, name| {
                index.payload_free(module, name);
            },
            &|index, module, name| {
                index.field_type(&named(module, name), 0, 0);
            },
        ];
        for lookup in lookups {
            let index = Index::new(&program);
            lookup(&index, "m", "Gone");
            lookup(&index, "prelude", "Gone");
            lookup(&index, "", "Here");
            assert_eq!(
                index.unresolved(),
                vec![
                    ("".to_owned(), "Here".to_owned()),
                    ("m".to_owned(), "Gone".to_owned()),
                    ("prelude".to_owned(), "Gone".to_owned()),
                ]
            );
            let index = Index::new(&program);
            lookup(&index, "m", "Here");
            for prelude in [
                "Option",
                "Result",
                "FileError",
                "KeyError",
                "List",
                "Map",
                "Entry",
                "ParseError",
            ] {
                lookup(&index, "prelude", prelude);
            }
            assert_eq!(index.unresolved(), Vec::<(String, String)>::new());
        }
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
                    constants: Vec::new(),
                },
                una_core::Module {
                    path: "bbb".to_owned(),
                    decls: vec![files(Vec::new())],
                    examples: Vec::new(),
                    constants: Vec::new(),
                },
            ],
            library: Vec::new(),
            host: vec![files(Vec::new())],
            condition_edits: Vec::new(),
            ..Program::default()
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
        let mut walker = Walker::new(&index, &sources, "m", SourceLayout::Joined);
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

    /// The goal of the toolshelf example's obligation on one line, and the same goal as the
    /// canonical form of edition 0.4 wraps it.
    const INLINE_GOAL: &str = "Checkout(tool: Tool(name: \"saw\", deposit: 15), member: Member(name: \"Bo\", holding: 3), due: 10).member.holding > 0";
    const WRAPPED_GOAL: &str = "Checkout(tool: Tool(name: \"saw\", deposit: 15), member: Member(name: \"Bo\", holding: 3\n  ), due: 10).member.holding > 0";

    fn toolshelf_id(goal: &str) -> String {
        obligation_id(
            "toolshelf.tools",
            "toolshelf.tools.example \"bringing a tool back frees a place\"",
            ObligationKind::Precondition,
            goal,
        )
    }

    #[test]
    fn editions_0_1_to_0_3_keep_their_goal_normalization_and_ids() {
        // `spec/12 §8`, ADR 0125 amendment of 2026-10-25: the goal of editions 0.1 to 0.3
        // collapses white space, exactly as before soft wrapping, so the ids of the same
        // sources are the ids they had: these are the ids of the rule before it.
        for goal in [
            INLINE_GOAL,
            WRAPPED_GOAL,
            "f(\"a  b\")",
            "f(value )",
            "f(value,\n  1\n)",
            "f(value) -- a  comment",
        ] {
            assert_eq!(
                normalized_goal(goal, SourceLayout::Joined),
                goal.split_whitespace().collect::<Vec<_>>().join(" "),
                "{goal}"
            );
        }
        assert_eq!(
            toolshelf_id(&normalized_goal(INLINE_GOAL, SourceLayout::Joined)),
            "ob-436d76f3dbf7"
        );
        // A line break before a closer, which a comment inside a line may keep in these
        // editions, still counts as a space there, as it always did.
        assert_eq!(
            toolshelf_id(&normalized_goal(WRAPPED_GOAL, SourceLayout::Joined)),
            "ob-4b9498c40ba5"
        );
    }

    #[test]
    fn edition_0_4_erases_the_layout_of_a_wrapped_line() {
        // The closer of a wrapped group changes neither the goal nor the id in edition 0.4.
        assert_eq!(
            normalized_goal(WRAPPED_GOAL, SourceLayout::Wrapped),
            normalized_goal(INLINE_GOAL, SourceLayout::Joined)
        );
        assert_eq!(
            toolshelf_id(&normalized_goal(WRAPPED_GOAL, SourceLayout::Wrapped)),
            "ob-436d76f3dbf7"
        );
        // A closer inside a text literal is no token: its space stays.
        assert_ne!(unwrap_layout("f(\"x )\")"), unwrap_layout("f(\"x)\")"));
        assert_ne!(unwrap_layout("f(\"x ]\")"), unwrap_layout("f(\"x]\")"));
        for unchanged in [
            "f(\"a  b\")",
            "f(\"a \\\"b\")",
            "f(\"{value}  )\")",
            r#"f("outer {g("inner ) ]")}  )")"#,
            "f(value )",
            "f(value) -- a  comment",
        ] {
            assert_eq!(
                unwrap_layout(unchanged),
                unchanged.split_whitespace().collect::<Vec<_>>().join(" "),
                "{unchanged}"
            );
        }
        for size in [16, 256, 4096] {
            let wrapped = (0..size)
                .map(|index| format!("f(value,\n  {index}\n)"))
                .collect::<Vec<_>>()
                .join(" + ");
            let inline = (0..size)
                .map(|index| format!("f(value, {index})"))
                .collect::<Vec<_>>()
                .join(" + ");
            assert_eq!(unwrap_layout(&wrapped), inline);
        }
    }

    #[test]
    fn goal_normalization_scales_linearly_with_wrapped_delimiters() {
        use std::time::Instant;

        let per_byte = |size: usize| {
            let unit = "f(value,\n  1\n) + ";
            let goal = unit.repeat(size / unit.len());
            let started = Instant::now();
            let normalized = unwrap_layout(std::hint::black_box(&goal));
            std::hint::black_box(normalized);
            started.elapsed().as_secs_f64() / goal.len() as f64
        };
        per_byte(1 << 15);
        let small = (0..3)
            .map(|_| per_byte(1 << 15))
            .fold(f64::INFINITY, f64::min);
        let large = (0..3)
            .map(|_| per_byte(1 << 19))
            .fold(f64::INFINITY, f64::min);
        assert!(
            large <= small * 4.0,
            "per-byte time grew from {small:e} to {large:e}"
        );
    }

    #[test]
    fn c_a_restoration_needs_both_conditions_within_the_encoding() {
        // ADR 0092, amendment of 2026-10-15, condition (c). The checker rejects a condition
        // outside the encoding first (U2014 in a header, U7002 in a lock), so this is pinned
        // here rather than by a program.
        assert!(restorable(true, None));
        assert!(restorable(true, Some(true)));
        assert!(!restorable(true, Some(false)));
        assert!(!restorable(false, None));
        assert!(!restorable(false, Some(true)));
    }
}
