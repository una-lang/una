//! The environment of the type checker: every kind, alias, function, capability and handler of
//! the package, the prelude and `std.testing`, the scopes of each module (ADR 0023), and the
//! lowering of written types (U2002, U2003).

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use una_core::IntType;
use una_core::bigint::BigInt;
use una_diag::Code;
use una_project::{Edition, Profile};
use una_syntax::ast::{
    CapabilityDecl, ClauseKind, Convention, DeclKind, Expr, Field, File, FnTypeClause,
    FunctionDecl, HandlerDecl, Ident, KindBody, KindDecl, Label as LabelAst, OpClauseKind, Param,
    PolicyDecl, PredicateDecl, Type, TypeCore, UsesClause,
};
use una_syntax::{Interner, Span, Symbol};

use super::Report;
use super::closest;
use super::dense::{dense_float, dense_numeric};
use super::label::{Accepts, Label};
use super::ty::{
    AliasId, CapId, Declared, Dense, FnRefinements, FnTy, FnUse, KindId, KindLabels, Names,
    Refinement, Shared, Show, Ty,
};
use crate::names::prelude;

/// A module the checker knows: a package module, the prelude or `std.testing`.
pub(crate) struct Module<'a> {
    pub(crate) tree: &'a File,
    pub(crate) interner: &'a Interner,
    /// Whether problems in it are reported: a package module without syntax errors (ADR 0023
    /// rule 7). The standard modules are never reported.
    pub(crate) reports: bool,
    /// Its path, from its header; a dependency's module `m` is `<local>.m` (ADR 0122 rule 47).
    pub(crate) path: String,
    /// The local name of the dependency it is a module of, if any.
    pub(crate) mount: Option<&'a str>,
    pub(crate) upper: BTreeMap<&'a str, Entity>,
    pub(crate) lower: BTreeMap<&'a str, Entity>,
    /// Names imported explicitly.
    pub(crate) imports: BTreeMap<&'a str, Entity>,
    /// Variants of the sum kinds it imports (ADR 0023 rule 2).
    pub(crate) imported_variants: BTreeMap<&'a str, Vec<Entity>>,
    /// What each of `tree.decls` defines, repeats included; `None` for requirements and
    /// examples.
    pub(crate) decls: Vec<Option<Entity>>,
}

/// What a name refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entity {
    Kind(KindId),
    Variant(KindId, usize),
    Alias(AliasId),
    Capability(CapId),
    Function(usize),
    Handler(usize),
    Builtin(Builtin),
    /// A unit of measure, edition 0.3 (ADR 0104 rule 1).
    Unit(usize),
    /// A built-in constant of `std.time`, edition 0.4: its value in the unit `unit`, an index
    /// of `units` (ADR 0116 rule 0, amended 2026-10-19).
    Constant {
        value: i64,
        unit: usize,
    },
    /// A module constant, edition 0.5 (ADR 0126): an index of `constants`.
    Const(usize),
}

/// The built-in types (`spec/03 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Builtin {
    Int,
    /// A fixed-width integer type, built in from edition 0.2 (`spec/18 §1`).
    Fixed(IntType),
    Bool,
    Text,
    Unit,
    Hashed,
    /// `ReplyRight` of `std.host`, edition 0.4 (ADR 0123 rule 1): exported by `std.host` and
    /// in no scope without an import.
    ReplyRight,
    List,
    /// `Map[K, V]`, built in from edition 0.2 (`spec/21 §1`).
    Map,
    /// `F64`, built in from edition 0.3 (`spec/29`, ADR 0114 rule 1).
    F64,
    /// `F32`, built in from edition 0.5 (`spec/29`, ADR 0120 rule 5).
    F32,
    /// A collection type that `std.collections` exports, edition 0.5 (ADR 0119 rule 0).
    Coll(una_core::Coll),
    /// `Complex[F64]` and `Complex[F32]`, built in from edition 0.5 (`spec/29 §19`, ADR 0120
    /// rule 22): two types, each a record kind of the prelude, written with their component
    /// type in brackets.
    Complex,
}

const BUILTINS: [(&str, Builtin); 6] = [
    ("Int", Builtin::Int),
    ("Bool", Builtin::Bool),
    ("Text", Builtin::Text),
    ("Unit", Builtin::Unit),
    ("Hashed", Builtin::Hashed),
    ("List", Builtin::List),
];

/// A field of a record or of a variant.
#[derive(Clone, Debug)]
pub(crate) struct FieldDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) ty: Declared,
    /// Where its type is written.
    pub(crate) span: Span,
}

/// A variant of a sum kind.
#[derive(Clone, Debug)]
pub(crate) struct VariantDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) fields: Vec<FieldDef<'a>>,
}

/// A record or sum kind.
#[derive(Clone, Debug)]
pub(crate) struct KindDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) module: usize,
    pub(crate) params: Vec<&'a str>,
    /// `Some` for a record, with its fields.
    pub(crate) record: Option<Vec<FieldDef<'a>>>,
    /// The variants of a sum.
    pub(crate) variants: Vec<VariantDef<'a>>,
    pub(crate) ast: &'a KindDecl,
}

impl KindDef<'_> {
    pub(crate) fn is_sum(&self) -> bool {
        self.record.is_none()
    }
}

/// A unit of measure a module declares (ADR 0104 rule 1), with its normal form once
/// [`Env::lower_units`] has computed it: `None` until then, and for a unit whose declaration
/// is refused (U2029).
pub(crate) struct UnitDef<'a> {
    pub(crate) module: usize,
    pub(crate) ast: &'a una_syntax::ast::MeasureDecl,
    pub(crate) norm: Option<Rc<super::units::Unit>>,
}

/// An alias or refinement kind, structural over its base (`spec/03 §2` rule 2).
#[derive(Clone, Debug)]
pub(crate) struct AliasDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) module: usize,
    pub(crate) params: Vec<&'a str>,
    pub(crate) base: Declared,
    /// The `where` predicate, over `it` of the base type.
    pub(crate) refinement: Option<&'a Expr>,
    /// The base type as written.
    pub(crate) ty: &'a Type,
}

/// What kind of callable a function definition is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FnKind {
    Function,
    Predicate {
        reflect: bool,
    },
    /// A policy, called as `p(v)` (`spec/09 §3`).
    Policy,
}

/// The declaration of a function definition.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FnAst<'a> {
    Function(&'a FunctionDecl),
    Predicate(&'a PredicateDecl),
    Policy(&'a PolicyDecl),
}

/// A parameter of a function, operation or handler.
#[derive(Clone, Debug)]
pub(crate) struct ParamDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) ty: Declared,
    /// `edit` or `take`, from edition 0.2 (`spec/17 §3`).
    pub(crate) convention: Option<Convention>,
    /// Where its type is written.
    pub(crate) span: Span,
}

/// A capability that a header or a handler requires, `uses @name: C[ops]`, declared or
/// inferred (`spec/05 §3`).
#[derive(Clone, Debug)]
pub(crate) struct Need {
    pub(crate) usage: FnUse,
    /// Where the clause is written, or where an inferred requirement first arises.
    pub(crate) span: Span,
}

/// The code a write or a call is in, for the highest program counter it can be entered under
/// (S-14, ADR 0085).
#[derive(Clone, Debug)]
pub(crate) enum Entry {
    /// The body of a declared function, predicate or policy.
    Function(usize),
    /// A handler operation's body, which its callers may enter with data up to this floor.
    Floor(Label),
    /// A lambda's body, which a caller may run under the cap.
    Lambda,
    /// An example, run at the public program counter.
    Root,
}

/// A function, predicate or policy, with its signature.
#[derive(Clone, Debug)]
pub(crate) struct FnDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) module: usize,
    pub(crate) kind: FnKind,
    pub(crate) type_params: Vec<&'a str>,
    /// The constraint of each type parameter: written, or inferred for a private function
    /// whose parameter has none written (`spec/20 §3` rule 4).
    pub(crate) constraints: Vec<Constraint>,
    /// Whether the constraint of each type parameter is written.
    pub(crate) constraints_written: Vec<bool>,
    pub(crate) params: Vec<ParamDef<'a>>,
    pub(crate) result: Declared,
    /// Where the result type is written.
    pub(crate) result_span: Span,
    /// The declared error kind (`fails E`); for a policy, that of its `via` function.
    pub(crate) fails: Option<Ty>,
    /// Whether it is `pub`.
    pub(crate) public: bool,
    /// Its `uses`, declared or inferred; for a policy, those of its `via` function.
    pub(crate) uses: Vec<Need>,
    /// Whether its `uses` are inferred: a private function without `uses` (ADR 0007).
    pub(crate) infer_uses: bool,
    /// Whether its `fails` is inferred: a private function without `fails` (ADR 0007).
    pub(crate) infer_fails: bool,
    /// Whether the header says `diverges`, edition 0.4 (`spec/26 §5`): never inferred.
    pub(crate) diverges: bool,
    /// The parameters of function type whose effects it performs, by `uses of p`, written or
    /// inferred (ADR 0062).
    pub(crate) uses_of: Vec<usize>,
    /// The parameter of function type whose failures it passes, by `fails of p`.
    pub(crate) fails_of: Option<usize>,
    pub(crate) requires: bool,
    pub(crate) ensures: bool,
    pub(crate) ast: FnAst<'a>,
}

/// A built-in constraint on a type parameter (`spec/20 §1`), in order: each includes the
/// ones before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Constraint {
    /// No constraint.
    #[default]
    None,
    /// `==` and `!=`.
    Eq,
    /// Also `<`, `<=`, `>` and `>=`.
    Ord,
    /// Also `x.to_int()`.
    Integer,
}

impl Constraint {
    /// The constraint named `name`, if any.
    pub(crate) fn from_name(name: &str) -> Option<Constraint> {
        match name {
            "Eq" => Some(Constraint::Eq),
            "Ord" => Some(Constraint::Ord),
            "Integer" => Some(Constraint::Integer),
            _ => None,
        }
    }

    /// Its name, as written.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Constraint::None => "none",
            Constraint::Eq => "Eq",
            Constraint::Ord => "Ord",
            Constraint::Integer => "Integer",
        }
    }
}

/// An operation of a capability.
#[derive(Clone, Debug)]
pub(crate) struct OpDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) params: Vec<ParamDef<'a>>,
    pub(crate) result: Declared,
    /// Where the result type is written.
    pub(crate) result_span: Span,
    pub(crate) fails: Option<Ty>,
    pub(crate) requires: bool,
    pub(crate) ensures: bool,
    /// The sink label of its arguments (`spec/05 §2`, `spec/07 §4`).
    pub(crate) accepts: Accepts,
    /// The positions of the parameters that accept every label and whose labels the result
    /// carries: `data` of `Keys.hmac` (ADR 0039).
    pub(crate) carried: Vec<usize>,
}

/// A capability.
#[derive(Clone, Debug)]
pub(crate) struct CapDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) module: usize,
    pub(crate) ops: Vec<OpDef<'a>>,
    pub(crate) ast: &'a CapabilityDecl,
}

/// A module constant, edition 0.5 (ADR 0126): its declaration, its declared type once the
/// signatures are lowered, and its value once [`super::constants`] has evaluated it.
#[derive(Clone, Debug)]
pub(crate) struct ConstDef<'a> {
    pub(crate) module: usize,
    pub(crate) name: &'a str,
    pub(crate) ast: &'a una_syntax::ast::ConstDecl,
    /// The declared type (ADR 0126 rule 6).
    pub(crate) declared: Declared,
}

/// A handler.
#[derive(Clone, Debug)]
pub(crate) struct HandlerDef<'a> {
    pub(crate) module: usize,
    /// Its own `uses`, which the context installing it holds (`spec/05 §4` rule 2).
    pub(crate) uses: Vec<Need>,
    pub(crate) params: Vec<ParamDef<'a>>,
    /// The capability it implements, when its name resolves to one.
    pub(crate) capability: Option<CapId>,
    pub(crate) ast: &'a HandlerDecl,
}

/// A module for [`Env::build`].
pub(crate) struct Source<'a> {
    pub(crate) tree: &'a File,
    pub(crate) interner: &'a Interner,
    pub(crate) reports: bool,
    /// The local name of the dependency it is a module of, if any (ADR 0122 rule 47).
    pub(crate) mount: Option<&'a str>,
}

/// How far the base of an alias has been lowered, to find aliases defined through themselves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AliasState {
    Pending,
    Lowering,
    Done,
}

/// For each `(name, module)` of `decls`, whether a declaration of another module has the same
/// name: O(n log n), by a sort of the names.
fn shared<'a>(decls: impl Iterator<Item = (&'a str, usize)>) -> Vec<bool> {
    let decls: Vec<(&str, usize)> = decls.collect();
    let mut modules: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for (name, module) in &decls {
        modules.entry(name).or_default().insert(*module);
    }
    decls
        .iter()
        .map(|(name, _)| modules.get(name).is_some_and(|m| m.len() > 1))
        .collect()
}

/// The environment.
pub(crate) struct Env<'a> {
    pub(crate) modules: Vec<Module<'a>>,
    pub(crate) kinds: Vec<KindDef<'a>>,
    pub(crate) aliases: Vec<AliasDef<'a>>,
    /// The units of measure of every module (ADR 0104).
    pub(crate) units: Vec<UnitDef<'a>>,
    /// The module constants of every module, edition 0.5 (ADR 0126).
    pub(crate) constants: Vec<ConstDef<'a>>,
    /// The integer value of each constant that has one, for range bounds (ADR 0126 rule 9).
    pub(crate) bound_values: BTreeMap<usize, Option<BigInt>>,
    pub(crate) functions: Vec<FnDef<'a>>,
    pub(crate) capabilities: Vec<CapDef<'a>>,
    pub(crate) handlers: Vec<HandlerDef<'a>>,
    /// For each kind, and each capability, whether another one of another module shares its
    /// bare name, the standard library's included: messages then name it with its module's
    /// path (#13, #17).
    pub(crate) shared_kind_names: Vec<bool>,
    pub(crate) shared_cap_names: Vec<bool>,
    /// The index of the prelude in `modules`; `std.testing` follows it.
    pub(crate) prelude: usize,
    /// The label categories that `una.toml` declares (`spec/07 §2`).
    pub(crate) categories: Vec<String>,
    /// The `via` function of each policy, and the policy (`spec/07 §5`).
    pub(crate) vias: BTreeMap<usize, usize>,
    /// The well-formed locked clauses of each locked function, by function (ADR 0092 rule 8,
    /// as amended).
    pub(crate) locks: BTreeMap<usize, Vec<super::lock::Entry<'a>>>,
    /// The labels that each kind's fields declare and the variance of its type parameters
    /// (ADR 0046), computed once the signatures are lowered.
    pub(crate) kind_labels: Rc<KindLabels>,
    /// The solved label variables of editable bindings, by symbol (ADR 0075).
    pub(crate) edit_labels: BTreeMap<String, Label>,
    /// Whether any editable binding has label variables, so pass 1 runs once more after the
    /// summaries reach their fixpoint (ADR 0075 rule 5).
    pub(crate) edit_labels_seen: bool,
    /// Whether a check of pass 1 changed a solution of `edit_labels`, and one that changed.
    pub(crate) edit_labels_changed: bool,
    pub(crate) edit_labels_changed_name: Option<String>,
    /// The binding of each label variable, for a diagnostic that names it.
    pub(crate) edit_label_spans: BTreeMap<String, Span>,
    /// Whether pass 1 met a call of the host operation `Files.write`, so that pass 2 is
    /// probed first for the file-state label (S-14, ADR 0085).
    pub(crate) files_written: bool,
    /// The file-state label: the join of the labels that decide where every `Files.write` of
    /// the package writes, and whether it runs, whatever run it happens in, which labels the failure of every
    /// `Files` operation (S-14, ADR 0085). Public until the probe of pass 2 sets it.
    pub(crate) file_state: Label,
    /// The timing label `τ(e)` of each `e within d`, by the span of `d` (ADR 0117 IF2): what the
    /// probe of pass 2 found, read by the checks of pass 2; a site absent here is `public` (the
    /// first probe).
    pub(crate) within_timing: BTreeMap<Span, Label>,
    /// `L_clk` of each reading of the host's `Clock`, by its span (ADR 0116 amendment of
    /// 2026-11-09, #898, C1): what the probe of pass 2 found, read by the checks of pass 2; a
    /// site absent here is `public` (the first probe).
    pub(crate) clock_timing: BTreeMap<Span, Label>,
    /// The residue label `L_res` (ADR 0116 P9) that the probe of pass 2 found, already joined
    /// into [`Self::within_timing`], with its sources, which the messages of pass 2 name.
    pub(crate) residue: super::timing::Residue,
    /// What a check of pass 2 records for the file-state label: each `Files.write`, with the
    /// labels of its path and the program counter, and the code it is in; each direct call
    /// of a function, with the program counter; the functions used as values, which a caller
    /// may run under their effect bound; and the cap, what `Files.write` accepts.
    pub(crate) file_writes: Vec<(Entry, Label, Span)>,
    pub(crate) file_calls: Vec<(Entry, usize, Label)>,
    pub(crate) file_escaped: BTreeSet<usize>,
    pub(crate) file_cap: Label,
    /// The type an open position of an editable binding resolved to, without labels, by the
    /// name of its variable: later checks give each label position inside it a variable.
    pub(crate) edit_shapes: BTreeMap<String, super::ty::Ty>,
    alias_state: Vec<AliasState>,
    /// The package's edition (`spec/13 §7`).
    pub(crate) edition: Edition,
    /// The package's profile (ADR 0109 rule 1): `app` for a single file and before edition
    /// 0.3. In `systems`, each `Int` operation outside clauses records an `overflow` site
    /// (rule 3), and elaboration records it in the program (rule 6).
    pub(crate) profile: Profile,
    /// The result summary of each generic function declared outside the prelude, by function
    /// (ADR 0067, `spec/07 §7` rule 8); edition 0.2 only.
    pub(crate) summaries: BTreeMap<usize, Summary>,
    /// Whether a summary grew in the current round of their least fixpoint.
    pub(crate) summaries_grew: bool,
    /// For a parameter that an `of` clause names, by function and parameter index: the type
    /// parameters whose decisions reach a call of it in the body, which its callers' arguments
    /// run under (ADR 0067 amendment, #234). Grown with the summaries.
    pub(crate) of_raises: BTreeMap<(usize, usize), std::collections::BTreeSet<usize>>,
    /// The function whose signature or body is lowering types, for the key types of maps.
    pub(crate) owner: Option<usize>,
    /// The key type of each map type written in the package, which must satisfy `Ord`
    /// (`spec/21 §1` rule 2, U2025).
    pub(crate) map_keys: Vec<MapKey>,
    /// The opaque predicates of the checked modules whose bodies call no other opaque
    /// predicate, set after pass 1: a function with such a body is not U2016 again, so U2016
    /// offers it as a repair (ADR 0095).
    pub(crate) opaque_plain: BTreeSet<usize>,
    /// The module paths that `una.toml` lists under `[trusted] modules` (`spec/07 §6`): a
    /// module is trusted only when its header says so and it is listed here. Empty for a file
    /// checked on its own.
    pub(crate) trusted_listed: BTreeSet<String>,
    /// The modules of dependencies the root trusts by a trust line (ADR 0122 rule 53.1), by
    /// index: a release whose policy one declares is not a release into a boundary.
    pub(crate) trusted_dependencies: BTreeSet<usize>,
    /// The code of an invalid project setting: U7002 in `una.toml`, U7008 in `ita.una`
    /// (ADR 0122 rule 13).
    pub(crate) project_code: Code,
    /// The package's manifest, `una.toml` or `ita.una`, as messages name it (ADR 0122 rule 13).
    pub(crate) manifest_file: &'static str,
    /// The boundaries of `una.toml`, and the boundary whose extent holds each package module
    /// (ADR 0093 rules 1 and 9); empty before edition 0.3.
    pub(crate) boundaries: super::boundary::Boundaries,
    /// For each kind, whether a value of it may hold a function value, through a field of a
    /// function type or of a type that holds one, at any depth: computed for every package,
    /// with or without boundaries, for the timing label of a handler's parameters (ADR 0116
    /// IF5 item 3, issue #1436).
    pub(crate) holds_function_kinds: Vec<bool>,
    /// For each kind, whether a value of it may hold a `ReplyRight` of `std.host`, at any
    /// depth (ADR 0123 IF6): `Exchange`, and every kind with a field that holds one.
    pub(crate) holds_reply_kinds: Vec<bool>,
    /// The capabilities whose operations can run an operation of `HttpServer` through a handler
    /// (ADR 0123 rule 5, U3033).
    pub(crate) serving_capabilities: BTreeSet<super::ty::CapId>,
    /// The `serve` section of `ita.una`, at its header, when the package has one: the driver
    /// of ADR 0123 rule 16 is a hand-off site of `L_srv` (IF-S1).
    pub(crate) serve_driver: Option<Span>,
    /// The `drive` section of `ita.una`, at its header, when the package has one: the
    /// diagnostics of its module's shapes point at its `module` line (`spec/26 §2`).
    pub(crate) drive_driver: Option<Span>,
    /// Whether a module of the package imports a name of the server from `std.host` (ADR 0123
    /// rule 0), or one of `std.testing` that names the server's kinds (rule 21): only then does
    /// Core IR declare the server's kinds, and a value of the package can hold a reply right.
    pub(crate) server_imported: bool,
    /// Whether a module of the package imports `HttpServer`, `Exchange` or `ReplyRight`, or
    /// `test_exchange` of `std.testing`, the names through which a value of the package can
    /// hold a reply right (ADR 0123 IF6 and rule 21).
    pub(crate) reply_rights: bool,
    /// Whether a module of the package imports `test_request`, `test_exchange` or
    /// `TestRequestError` of `std.testing` (ADR 0123 rule 21): only then does Core IR declare
    /// them, so the Core IR of a package that does not, a server's included, does not change.
    pub(crate) testing_server: bool,
    /// The `via` functions whose failure reveals nothing, by function id (ADR 0098): a call of
    /// their policy fails at the program counter, not at `pc ⊔ ℓ(to)`.
    pub(crate) plain_vias: BTreeSet<usize>,
    /// The `via` functions of ADR 0098's shape that a `Keys` handler named outside its
    /// declaration and examples keeps from it, by function id: the handler's name and the first
    /// place it is named, which the U4002 at a call of their policy shows.
    pub(crate) blocked_vias: BTreeMap<usize, (String, Span)>,
    /// The module of the map's built-in method `update`, in edition 0.5 (ADR 0119 rule 18):
    /// in no scope, so only method syntax on a map reaches it.
    pub(crate) map_module: Option<usize>,
    /// The function `update` of that module.
    pub(crate) map_update: Option<usize>,
    /// The module `std.collections`, when a package of edition 0.5 imports it (ADR 0119
    /// rule 0).
    pub(crate) collections: Option<usize>,
    /// The hidden function `update` of that module, which only method syntax on a hash map
    /// reaches (ADR 0119 rule 12).
    pub(crate) hash_map_update: Option<usize>,
    /// The handler `limiter` of `std.limit`, when a package of edition 0.4 imports the module
    /// (ADR 0117 rule 9): the runtime provides it, and its installation is checked for IF5.
    pub(crate) limiter: Option<usize>,
}

/// The key type of a written `Map[K, V]`, or an argument of a written collection type that
/// must satisfy a constraint (ADR 0119 rules 7, 8 and 13), where it is written, and the
/// function whose type parameters it may name.
#[derive(Clone, Debug)]
pub(crate) struct MapKey {
    pub(crate) key: Ty,
    pub(crate) span: Span,
    pub(crate) owner: Option<usize>,
    /// The collection type and the index of the argument; `None` for a map's key.
    pub(crate) of: Option<(una_core::Coll, usize)>,
}

impl MapKey {
    /// The constraint the argument must satisfy: `Ord` for a map's key.
    pub(crate) fn need(&self) -> Constraint {
        match self.of {
            None => Constraint::Ord,
            Some((coll, index)) => match coll.needs(index) {
                una_core::Constraint::Eq => Constraint::Eq,
                una_core::Constraint::Ord => Constraint::Ord,
                una_core::Constraint::Integer => Constraint::Integer,
                una_core::Constraint::None => Constraint::None,
            },
        }
    }
}

/// A step from a position of a type to a position inside it (ADR 0067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    /// The element of a `List`.
    Element,
    /// The type argument of `Option`, `Result` or a kind, by index.
    Arg(usize),
    /// The result of a function type.
    Result,
}

/// The positions of a declared result type, as paths of steps from its top, and the type
/// parameters, by index, whose symbolic labels the function's body can put at each one
/// (ADR 0067 L1). A position missing from the map has an empty summary.
pub(crate) type Summary = BTreeMap<Vec<Step>, std::collections::BTreeSet<usize>>;

/// A position written as `spec/13 §5` writes it: `top`, or its steps joined by `.`.
pub(crate) fn position_text(path: &[Step]) -> String {
    if path.is_empty() {
        return "top".to_owned();
    }
    path.iter()
        .map(|step| match step {
            Step::Element => "element".to_owned(),
            Step::Arg(index) => format!("arg {}", index + 1),
            Step::Result => "result".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

impl<'a> Env<'a> {
    /// Builds the environment of the package modules, then the prelude, `std.testing`, in
    /// edition 0.5 the module of the map's `update` and, when the package imports it,
    /// `std.collections`, and lowers every signature.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(
        package: Vec<Source<'a>>,
        prelude: Source<'a>,
        after_prelude: Vec<Source<'a>>,
        map_0_4: Option<Source<'a>>,
        collections: Option<Source<'a>>,
        limit: Option<Source<'a>>,
        categories: Vec<String>,
        edition: Edition,
        profile: Profile,
        report: &mut Report<'_>,
    ) -> Env<'a> {
        let count = package.len();
        let mut env = Env {
            modules: Vec::new(),
            shared_kind_names: Vec::new(),
            shared_cap_names: Vec::new(),
            kinds: Vec::new(),
            aliases: Vec::new(),
            units: Vec::new(),
            constants: Vec::new(),
            bound_values: BTreeMap::new(),
            functions: Vec::new(),
            capabilities: Vec::new(),
            handlers: Vec::new(),
            prelude: count,
            categories,
            vias: BTreeMap::new(),
            locks: BTreeMap::new(),
            kind_labels: Rc::new(KindLabels::default()),
            edit_labels: BTreeMap::new(),
            edit_labels_seen: false,
            edit_labels_changed: false,
            edit_labels_changed_name: None,
            edit_label_spans: BTreeMap::new(),
            files_written: false,
            file_state: Label::public(),
            within_timing: BTreeMap::new(),
            clock_timing: BTreeMap::new(),
            residue: super::timing::Residue::none(),
            file_writes: Vec::new(),
            file_calls: Vec::new(),
            file_escaped: BTreeSet::new(),
            file_cap: Label::public(),
            edit_shapes: BTreeMap::new(),
            alias_state: Vec::new(),
            edition,
            profile,
            summaries: BTreeMap::new(),
            summaries_grew: false,
            of_raises: BTreeMap::new(),
            owner: None,
            map_keys: Vec::new(),
            opaque_plain: BTreeSet::new(),
            trusted_listed: BTreeSet::new(),
            trusted_dependencies: BTreeSet::new(),
            project_code: Code::U7002,
            manifest_file: "una.toml",
            boundaries: super::boundary::Boundaries::default(),
            holds_function_kinds: Vec::new(),
            holds_reply_kinds: Vec::new(),
            serving_capabilities: BTreeSet::new(),
            serve_driver: None,
            drive_driver: None,
            server_imported: false,
            reply_rights: false,
            testing_server: false,
            plain_vias: BTreeSet::new(),
            blocked_vias: BTreeMap::new(),
            map_module: None,
            map_update: None,
            collections: None,
            hash_map_update: None,
            limiter: None,
        };
        for source in package
            .into_iter()
            .chain(std::iter::once(prelude))
            .chain(after_prelude)
        {
            env.register(source);
        }
        if let Some(source) = map_0_4 {
            env.map_module = Some(env.modules.len());
            env.register(source);
        }
        if let Some(source) = collections {
            env.collections = Some(env.modules.len());
            env.register(source);
        }
        // ADR 0117 rule 9: `std.limit` is in scope only for a package of edition 0.4 that
        // imports it, so no other package's checks or Core IR change.
        if let Some(source) = limit {
            let index = env.modules.len();
            env.register(source);
            env.limiter =
                env.modules
                    .get(index)
                    .and_then(|module| match module.lower.get("limiter") {
                        Some(Entity::Handler(id)) => Some(*id),
                        _ => None,
                    });
        }
        env.map_update =
            env.map_module
                .and_then(|index| match env.modules.get(index)?.lower.get("update") {
                    Some(Entity::Function(id)) => Some(*id),
                    _ => None,
                });
        env.hash_map_update =
            env.collections
                .and_then(|index| match env.modules.get(index)?.lower.get("update") {
                    Some(Entity::Function(id)) => Some(*id),
                    _ => None,
                });
        // ADR 0119 rule 0: `std.collections` exports its built-in types beside its kinds.
        if let Some(module) = env.collections.and_then(|index| env.modules.get_mut(index)) {
            for coll in una_core::Coll::ALL {
                module
                    .upper
                    .entry(coll.as_str())
                    .or_insert(Entity::Builtin(Builtin::Coll(coll)));
            }
        }
        if let Some(module) = env.modules.get_mut(count) {
            for (name, builtin) in BUILTINS {
                module.upper.entry(name).or_insert(Entity::Builtin(builtin));
            }
            if edition >= Edition::V0_2 {
                for int in IntType::ALL {
                    module
                        .upper
                        .entry(int.as_str())
                        .or_insert(Entity::Builtin(Builtin::Fixed(int)));
                }
                module
                    .upper
                    .entry("Map")
                    .or_insert(Entity::Builtin(Builtin::Map));
            }
            if edition >= Edition::V0_3 {
                module
                    .upper
                    .entry("F64")
                    .or_insert(Entity::Builtin(Builtin::F64));
            }
            if edition >= Edition::V0_5 {
                module
                    .upper
                    .entry("F32")
                    .or_insert(Entity::Builtin(Builtin::F32));
                module
                    .upper
                    .entry("Complex")
                    .or_insert(Entity::Builtin(Builtin::Complex));
            }
            if edition >= Edition::V0_4 {
                // ADR 0123 rule 1: `std.host` exports `ReplyRight`, which only an import brings
                // into scope (`spec/10 §11` rule 10).
                module
                    .upper
                    .entry("ReplyRight")
                    .or_insert(Entity::Builtin(Builtin::ReplyRight));
            }
        }
        env.imports(count);
        env.alias_state = vec![AliasState::Pending; env.aliases.len()];
        env.shared_kind_names = shared(env.kinds.iter().map(|k| (k.name, k.module)));
        env.shared_cap_names = shared(env.capabilities.iter().map(|c| (c.name, c.module)));
        env.signatures(report);
        for id in 0..env.functions.len() {
            let Some((module, FnAst::Policy(policy))) =
                env.functions.get(id).map(|f| (f.module, f.ast))
            else {
                continue;
            };
            let via = env.text(module, policy.via.name);
            if let Some(Entity::Function(target)) = env.lower(module, via) {
                env.vias.entry(target).or_insert(id);
            }
        }
        let fields: Vec<Vec<&Ty>> = env
            .kinds
            .iter()
            .map(|kind| {
                kind.record
                    .iter()
                    .flatten()
                    .chain(kind.variants.iter().flat_map(|v| v.fields.iter()))
                    .map(|field| &field.ty.base)
                    .collect()
            })
            .collect();
        let params: Vec<usize> = env.kinds.iter().map(|kind| kind.params.len()).collect();
        let mut kind_labels = KindLabels::new(&fields, &params);
        // Both editions mark deep labels, and flows tell `ℓ*_T` from `ℓ_T` in both (#354):
        // reading `T*` as `T` let an annotation, a field or a callback's parameter drop a
        // decision. Edition 0.2 also has result summaries.
        kind_labels.deep_symbols = true;
        env.kind_labels = Rc::new(kind_labels);
        env
    }

    /// The text of `symbol`, interned by module `module`.
    /// The `via` function of the policy `policy`, which a call of the policy runs (`spec/07
    /// §5`); `None` for a function that is not a policy, or whose `via` does not resolve.
    pub(crate) fn via_of(&self, policy: usize) -> Option<usize> {
        let def = self.functions.get(policy)?;
        let FnAst::Policy(decl) = def.ast else {
            return None;
        };
        match self.lower(def.module, self.text(def.module, decl.via.name)) {
            Some(Entity::Function(via)) => Some(via),
            _ => None,
        }
    }

    pub(crate) fn text(&self, module: usize, symbol: Symbol) -> &'a str {
        match self.modules.get(module) {
            Some(module) => module.interner.resolve(symbol),
            None => "",
        }
    }

    /// Whether problems in module `module` are reported.
    pub(crate) fn reports(&self, module: usize) -> bool {
        self.modules.get(module).is_some_and(|m| m.reports)
    }

    /// Whether module `module` belongs to the package, so its spans can be reported.
    pub(crate) fn in_package(&self, module: usize) -> bool {
        module < self.prelude
    }

    /// Registers the declarations of a module. The first of two declarations with one name
    /// wins, as in ADR 0023 (the second is U1002).
    fn register(&mut self, source: Source<'a>) {
        let index = self.modules.len();
        let interner = source.interner;
        let name = |ident: &Ident| -> &'a str { interner.resolve(ident.name) };
        let path = source
            .tree
            .module
            .as_ref()
            .map_or_else(String::new, |header| {
                let segments: Vec<&str> = header.path.segments.iter().map(name).collect();
                match source.mount {
                    Some(local) => format!("{local}.{}", segments.join(".")),
                    None => segments.join("."),
                }
            });
        let mut module = Module {
            tree: source.tree,
            interner,
            reports: source.reports,
            path,
            mount: source.mount,
            upper: BTreeMap::new(),
            lower: BTreeMap::new(),
            imports: BTreeMap::new(),
            imported_variants: BTreeMap::new(),
            decls: Vec::new(),
        };
        for decl in &source.tree.decls {
            let entity = match &decl.kind {
                DeclKind::Kind(kind) => {
                    let params: Vec<&'a str> = kind.params.iter().map(name).collect();
                    match &kind.body {
                        KindBody::Alias { ty, refinement } => {
                            self.aliases.push(AliasDef {
                                name: name(&kind.name),
                                module: index,
                                params,
                                base: Declared::error(),
                                refinement: refinement.as_deref(),
                                ty,
                            });
                            Some((name(&kind.name), Entity::Alias(self.aliases.len() - 1)))
                        }
                        KindBody::Sum(variants) => {
                            let id = self.kinds.len();
                            self.kinds.push(KindDef {
                                name: name(&kind.name),
                                module: index,
                                params,
                                record: None,
                                variants: variants
                                    .iter()
                                    .map(|variant| VariantDef {
                                        name: name(&variant.name),
                                        fields: Vec::new(),
                                    })
                                    .collect(),
                                ast: kind,
                            });
                            for (position, variant) in variants.iter().enumerate() {
                                module
                                    .upper
                                    .entry(name(&variant.name))
                                    .or_insert(Entity::Variant(id, position));
                            }
                            Some((name(&kind.name), Entity::Kind(id)))
                        }
                        KindBody::Record(_) => {
                            self.kinds.push(KindDef {
                                name: name(&kind.name),
                                module: index,
                                params,
                                record: Some(Vec::new()),
                                variants: Vec::new(),
                                ast: kind,
                            });
                            Some((name(&kind.name), Entity::Kind(self.kinds.len() - 1)))
                        }
                    }
                }
                DeclKind::Capability(capability) => {
                    self.capabilities.push(CapDef {
                        name: name(&capability.name),
                        module: index,
                        ops: Vec::new(),
                        ast: capability,
                    });
                    let id = self.capabilities.len() - 1;
                    Some((name(&capability.name), Entity::Capability(id)))
                }
                DeclKind::Function(function) => {
                    let ast = FnAst::Function(function);
                    let id = self.function(index, name(&function.name), FnKind::Function, ast);
                    Some((name(&function.name), Entity::Function(id)))
                }
                DeclKind::Predicate(predicate) => {
                    let kind = FnKind::Predicate {
                        reflect: predicate.reflect,
                    };
                    let ast = FnAst::Predicate(predicate);
                    let id = self.function(index, name(&predicate.name), kind, ast);
                    Some((name(&predicate.name), Entity::Function(id)))
                }
                DeclKind::Policy(policy) => {
                    let ast = FnAst::Policy(policy);
                    let id = self.function(index, name(&policy.name), FnKind::Policy, ast);
                    Some((name(&policy.name), Entity::Function(id)))
                }
                DeclKind::Handler(handler) => {
                    self.handlers.push(HandlerDef {
                        module: index,
                        uses: Vec::new(),
                        params: Vec::new(),
                        capability: None,
                        ast: handler,
                    });
                    let id = self.handlers.len() - 1;
                    Some((name(&handler.name), Entity::Handler(id)))
                }
                DeclKind::Measure(measure) => {
                    self.units.push(UnitDef {
                        module: index,
                        ast: measure,
                        norm: None,
                    });
                    Some((name(&measure.name), Entity::Unit(self.units.len() - 1)))
                }
                DeclKind::Const(constant) => {
                    self.constants.push(ConstDef {
                        module: index,
                        name: name(&constant.name),
                        ast: constant,
                        declared: Declared::error(),
                    });
                    Some((
                        name(&constant.name),
                        Entity::Const(self.constants.len() - 1),
                    ))
                }
                DeclKind::Requirement(_) | DeclKind::Example(_) => None,
            };
            if let Some((text, entity)) = entity {
                let space = match entity {
                    Entity::Function(_) | Entity::Handler(_) | Entity::Const(_) => {
                        &mut module.lower
                    }
                    _ => &mut module.upper,
                };
                space.entry(text).or_insert(entity);
            }
            module.decls.push(entity.map(|(_, entity)| entity));
        }
        self.modules.push(module);
    }

    fn function(&mut self, module: usize, name: &'a str, kind: FnKind, ast: FnAst<'a>) -> usize {
        let result_span = match ast {
            FnAst::Function(function) => function.result.span,
            FnAst::Predicate(predicate) => predicate.result.span,
            FnAst::Policy(policy) => policy.to.span,
        };
        self.functions.push(FnDef {
            name,
            module,
            kind,
            type_params: Vec::new(),
            params: Vec::new(),
            result: Declared::error(),
            result_span,
            fails: None,
            public: match ast {
                FnAst::Function(function) => function.public,
                FnAst::Predicate(predicate) => predicate.public,
                FnAst::Policy(policy) => policy.public,
            },
            uses: Vec::new(),
            infer_uses: false,
            constraints: Vec::new(),
            constraints_written: Vec::new(),
            infer_fails: false,
            diverges: false,
            uses_of: Vec::new(),
            fails_of: None,
            requires: false,
            ensures: false,
            ast,
        });
        self.functions.len() - 1
    }

    /// Resolves the imports of the package modules, the first `count` modules, and of the
    /// standard modules from `std.testing` on, which may import one another (`std.drive` and,
    /// in edition 0.4, `std.testing` import `std.time`). Imports that resolve to nothing were reported by M3 and are skipped.
    fn imports(&mut self, count: usize) {
        let mut paths: BTreeMap<String, usize> = BTreeMap::new();
        for (index, module) in self.modules.iter().enumerate().take(count) {
            paths.entry(module.path.clone()).or_insert(index);
        }
        paths.insert("std.testing".to_owned(), count + 1);
        // The standard modules of edition 0.4, after `std.testing` (ADR 0116).
        for (index, module) in self.modules.iter().enumerate().skip(count + 2) {
            paths.insert(module.path.clone(), index);
        }
        // `std.host` is the prelude's host declarations (`spec/10 §11` rule 10).
        paths.insert(prelude::STD_HOST.to_owned(), count);
        // `std.testing` imports `Millisecond` from `std.time` in edition 0.4 (ADR 0116 rule 19).
        let later = (count + 2)..self.modules.len();
        for index in (0..count).chain(std::iter::once(count + 1)).chain(later) {
            let Some(module) = self.modules.get(index) else {
                continue;
            };
            let mut imports: BTreeMap<&'a str, Entity> = BTreeMap::new();
            let mut variants: BTreeMap<&'a str, Vec<Entity>> = BTreeMap::new();
            for import in &module.tree.imports {
                let path: Vec<&str> = import
                    .module
                    .segments
                    .iter()
                    .map(|segment| module.interner.resolve(segment.name))
                    .collect();
                let joined = path.join(".");
                let host = joined == prelude::STD_HOST;
                let time = joined == prelude::STD_TIME;
                let joined_testing = joined == prelude::STD_TESTING;
                let standard = host
                    || time
                    || joined_testing
                    || joined == prelude::STD_DRIVE
                    || joined == prelude::STD_COLLECTIONS
                    || joined == prelude::STD_LIMIT;
                // A dependency's module imports its own modules and the standard ones, never
                // the root's (ADR 0122 rule 47).
                let key = match module.mount {
                    Some(local) if !standard => format!("{local}.{joined}"),
                    _ => joined,
                };
                let Some(target) = paths.get(&key).and_then(|&t| self.modules.get(t)) else {
                    continue;
                };
                for ident in &import.names {
                    let name = module.interner.resolve(ident.name);
                    if host && !prelude::std_host_names(self.edition).contains(&name) {
                        continue;
                    }
                    // ADR 0123 rule 0: a package module that imports a name of the server.
                    if host && index < count && prelude::SERVER_NAMES.contains(&name) {
                        self.server_imported = true;
                        if matches!(name, "HttpServer" | "Exchange" | "ReplyRight") {
                            self.reply_rights = true;
                        }
                    }
                    // ADR 0123 rule 21: the functions of `std.testing` that make and return
                    // the server's kinds, `test_exchange` a reply right.
                    if joined_testing
                        && index < count
                        && prelude::STD_TESTING_SERVER_NAMES.contains(&name)
                    {
                        self.server_imported = true;
                        self.testing_server = true;
                        if name == "test_exchange" {
                            self.reply_rights = true;
                        }
                    }
                    // `millisecond`, `second` and `minute` are built-in names of `std.time`, of
                    // type `Int of Millisecond` (ADR 0116 rule 0, amended 2026-10-19).
                    if time
                        && self.edition < una_project::Edition::V0_5
                        && let Some(&(_, value)) =
                            prelude::STD_TIME_CONSTANTS.iter().find(|(c, _)| *c == name)
                        && let Some(&Entity::Unit(unit)) = target.upper.get("Millisecond")
                    {
                        if !(module.upper.contains_key(name) || module.lower.contains_key(name)) {
                            imports.insert(name, Entity::Constant { value, unit });
                        }
                        continue;
                    }
                    let Some(&entity) = target.upper.get(name).or_else(|| target.lower.get(name))
                    else {
                        continue;
                    };
                    let own = module.upper.contains_key(name) || module.lower.contains_key(name);
                    if own || imports.contains_key(name) {
                        continue;
                    }
                    imports.insert(name, entity);
                    if let Entity::Kind(kind) = entity
                        && let Some(def) = self.kinds.get(kind)
                    {
                        for (position, variant) in def.variants.iter().enumerate() {
                            variants
                                .entry(variant.name)
                                .or_default()
                                .push(Entity::Variant(kind, position));
                        }
                    }
                }
            }
            if let Some(module) = self.modules.get_mut(index) {
                module.imports = imports;
                module.imported_variants = variants;
            }
        }
    }

    /// The dense type that `name` names in a type, in edition 0.5 (ADR 0120 rule 3): `Array`,
    /// `Matrix`, `Tensor` and `Lu` are built in, ahead of any declaration and of the variant
    /// `Json.Array`.
    pub(crate) fn dense_name(&self, name: &str) -> Option<Dense> {
        if self.edition < Edition::V0_5 {
            return None;
        }
        match name {
            "Array" => Some(Dense::Array),
            "Matrix" => Some(Dense::Matrix),
            "Tensor" => Some(Dense::Tensor),
            "Lu" => Some(Dense::Lu),
            _ => None,
        }
    }

    /// The candidates for an upper-case name in `module`, from the innermost scope that holds
    /// any (ADR 0023 rule 2). Several candidates are variants of imported kinds.
    pub(crate) fn upper(&self, module: usize, name: &str) -> Vec<Entity> {
        let Some(scope) = self.modules.get(module) else {
            return Vec::new();
        };
        if let Some(&entity) = scope.upper.get(name).or_else(|| scope.imports.get(name)) {
            return vec![entity];
        }
        if let Some(variants) = scope.imported_variants.get(name) {
            return variants.clone();
        }
        // ADR 0123 rule 0: the names of the server are in no scope without an import.
        if module < self.prelude && prelude::SERVER_NAMES.contains(&name) {
            return Vec::new();
        }
        match self
            .modules
            .get(self.prelude)
            .and_then(|p| p.upper.get(name))
        {
            Some(&entity) => vec![entity],
            None => Vec::new(),
        }
    }

    /// The entity of a lower-case name in `module`, outside locals.
    pub(crate) fn lower(&self, module: usize, name: &str) -> Option<Entity> {
        let scope = self.modules.get(module)?;
        scope
            .lower
            .get(name)
            .or_else(|| scope.imports.get(name))
            .or_else(|| {
                // The prelude functions of dense methods are reached by their receiver only
                // (`spec/29 §17`): no name in a program finds them.
                if name.starts_with(prelude::DENSE_HIDDEN) {
                    return None;
                }
                self.modules
                    .get(self.prelude)
                    .and_then(|p| p.lower.get(name))
            })
            .copied()
    }

    /// The prelude function `name`, whatever scopes a program declares: the functions that the
    /// methods of dense values resolve to (`spec/29 §17`).
    pub(crate) fn prelude_function(&self, name: &str) -> Option<usize> {
        match self.modules.get(self.prelude)?.lower.get(name)? {
            Entity::Function(id) => Some(*id),
            _ => None,
        }
    }

    /// How a program writes the type of kind `id` when it is a complex kind of the prelude:
    /// `Complex[F64]` or `Complex[F32]` (ADR 0120 rule 22).
    pub(crate) fn complex_written(&self, id: KindId) -> Option<&'static str> {
        let kind = self.kinds.get(id)?;
        if kind.module != self.prelude {
            return None;
        }
        una_core::complex::written(kind.name)
    }

    /// `Complex[T]` with the type arguments `bases`: the prelude's record kind of `T`, `F64` or
    /// `F32` without a label; any other argument is U2002 (ADR 0120 rule 2). A label inside the
    /// brackets would hide the components' labels, so the type is labeled as a whole.
    fn complex_type(
        &self,
        bases: &[Ty],
        span: Span,
        reports: bool,
        report: &mut Report<'_>,
    ) -> Declared {
        let name = match bases.first() {
            Some(Ty::F64) => una_core::complex::COMPLEX_F64,
            Some(Ty::F32) => una_core::complex::COMPLEX_F32,
            // Reported where it was lowered.
            Some(Ty::Error(_)) => return Declared::error(),
            _ => {
                if reports {
                    report.error(
                        Code::U2002,
                        span,
                        "`Complex` takes `F64` or `F32`, unlabeled, as in `Complex[F64]` or `secret Complex[F32]`".to_owned(),
                    );
                }
                return Declared::error();
            }
        };
        match self.prelude_kind(name) {
            Some(id) => Declared::plain(Ty::Kind(id, Vec::new().into())),
            None => Declared::error(),
        }
    }

    /// The kind `name` of `std.time`, `Timed` or `Deadline` (ADR 0117 rule 5), in edition 0.4.
    pub(crate) fn std_time_kind(&self, name: &str) -> Option<KindId> {
        let module = self
            .modules
            .iter()
            .find(|module| module.path == crate::names::prelude::STD_TIME)?;
        match module.upper.get(name)? {
            Entity::Kind(id) => Some(*id),
            _ => None,
        }
    }

    /// The kind `name` of the prelude, such as `Option` or `Result`.
    pub(crate) fn prelude_kind(&self, name: &str) -> Option<KindId> {
        match self.modules.get(self.prelude)?.upper.get(name)? {
            Entity::Kind(id) => Some(*id),
            _ => None,
        }
    }

    /// The name of a kind as declared, for Core IR, reports, repairs and the index, which name
    /// it as source does.
    pub(crate) fn kind_name(&self, id: KindId) -> &'a str {
        self.kinds.get(id).map_or("?", |kind| kind.name)
    }

    /// The name of a kind, for messages: with its module's path when another module declares
    /// a kind of the same name (#17).
    /// How a refined nested position is written in messages (ADR 0103): the refined kind its
    /// last refinement comes from, or its base followed by its range.
    pub(crate) fn refined_display(&self, refinements: &[Refinement], base: &str) -> Option<String> {
        match refinements.last()? {
            Refinement::Range(range, _) => Some(format!("{base} {range}")),
            Refinement::Alias(id, _) => self.aliases.get(*id).map(|alias| alias.name.to_owned()),
        }
    }

    pub(crate) fn kind_display(&self, id: KindId) -> String {
        // A complex kind of the prelude is written as its type, `Complex[F64]` (ADR 0120
        // rule 22).
        if let Some(written) = self.complex_written(id) {
            return written.to_owned();
        }
        match self.kinds.get(id) {
            Some(kind) if self.shared_kind_names.get(id).copied().unwrap_or(false) => {
                self.qualified(kind.module, kind.name)
            }
            Some(kind) => kind.name.to_owned(),
            None => "?".to_owned(),
        }
    }

    /// The name of a capability, for messages: with its module's path when another module
    /// declares a capability of the same name, as a module's own `Console` and the host's do
    /// (#13, #17).
    pub(crate) fn cap_display(&self, id: CapId) -> String {
        match self.capabilities.get(id) {
            Some(cap) if self.shared_cap_names.get(id).copied().unwrap_or(false) => {
                self.qualified(cap.module, cap.name)
            }
            Some(cap) => cap.name.to_owned(),
            None => "?".to_owned(),
        }
    }

    /// [`Env::cap_display`] for a capability at hand: O(capabilities), for a message.
    pub(crate) fn cap_def_name(&self, cap: &CapDef<'_>) -> String {
        if self
            .capabilities
            .iter()
            .any(|other| other.name == cap.name && other.module != cap.module)
        {
            self.qualified(cap.module, cap.name)
        } else {
            cap.name.to_owned()
        }
    }

    /// `name` with the path of `module`. The host capabilities and their failure kinds are
    /// declared in the prelude and named by the module that exports them, `std.host`
    /// (`spec/14 §3`).
    fn qualified(&self, module: usize, name: &str) -> String {
        if module == self.prelude
            && crate::names::prelude::std_host_names(self.edition).contains(&name)
        {
            return format!("{}.{name}", crate::names::prelude::STD_HOST);
        }
        match self.modules.get(module) {
            Some(module) => format!("{}.{name}", module.path),
            None => name.to_owned(),
        }
    }

    /// Computes the normal form of every declared unit (ADR 0104 rule 1), in declaration
    /// order, following references depth first. A cycle, an unknown unit, a factor that is not
    /// a positive integer, a scale past `u128` and a derived unit with no dimension and scale 1
    /// are U2029 at the declaration.
    fn lower_units(&mut self, report: &mut Report<'_>) {
        let mut state = vec![0u8; self.units.len()];
        for id in 0..self.units.len() {
            self.lower_unit(id, &mut state, report);
        }
    }

    /// `state`: 0 not visited, 1 being computed (a cycle when met again), 2 done.
    fn lower_unit(&mut self, id: usize, state: &mut [u8], report: &mut Report<'_>) {
        match state.get(id) {
            Some(0) => {}
            Some(1) => {
                if let Some(def) = self.units.get(id)
                    && self.reports(def.module)
                {
                    let name = self.text(def.module, def.ast.name.name).to_owned();
                    report.error(
                        Code::U2029,
                        def.ast.name.span,
                        format!("unit `{name}` is defined in terms of itself"),
                    );
                }
                return;
            }
            _ => return,
        }
        if let Some(slot) = state.get_mut(id) {
            *slot = 1;
        }
        let Some((module, ast)) = self.units.get(id).map(|def| (def.module, def.ast)) else {
            return;
        };
        let name = self.text(module, ast.name.name).to_owned();
        let norm = match &ast.unit {
            None => Some(super::units::Unit::base(super::units::BaseUnit {
                module: self.modules.get(module).map(|m| m.path.clone()).unwrap_or_default(),
                name: name.clone(),
            })),
            Some(expr) => self.unit_expr(module, expr, Some(state), report).and_then(|unit| {
                if unit.is_one() {
                    if self.reports(module) {
                        report.error(
                            Code::U2029,
                            ast.name.span,
                            format!(
                                "unit `{name}` has no dimension and scale 1, so `Int of {name}` would be `Int`; declare a base unit with `measure {name}`"
                            ),
                        );
                    }
                    None
                } else {
                    Some(unit)
                }
            }),
        };
        if let Some(def) = self.units.get_mut(id) {
            def.norm = norm.map(Rc::new);
        }
        if let Some(slot) = state.get_mut(id) {
            *slot = 2;
        }
    }

    /// The normal form of the unit expression `expr` in `module` (ADR 0104 rule 2), or `None`
    /// after a U2029. `state` is given while declarations are being lowered, so that a
    /// reference is lowered first and a cycle is found.
    pub(crate) fn unit_expr(
        &mut self,
        module: usize,
        expr: &una_syntax::ast::UnitExpr,
        mut state: Option<&mut [u8]>,
        report: &mut Report<'_>,
    ) -> Option<super::units::Unit> {
        use una_syntax::ast::UnitExpr;
        let reports = self.reports(module);
        match expr {
            UnitExpr::Name(ident) => {
                let name = self.text(module, ident.name).to_owned();
                match self.upper(module, &name).first().copied() {
                    Some(Entity::Unit(id)) => {
                        if let Some(state) = state.as_deref_mut() {
                            self.lower_unit(id, state, report);
                        }
                        let norm = self.units.get(id).and_then(|def| def.norm.clone());
                        norm.map(|norm| (*norm).clone())
                    }
                    found => {
                        if reports {
                            let message = if found.is_some() {
                                format!("`{name}` is not a unit; a unit is declared with `measure`")
                            } else {
                                format!("unknown unit `{name}`")
                            };
                            report.error(Code::U2029, ident.span, message);
                        }
                        None
                    }
                }
            }
            UnitExpr::Factor { digits, span } => {
                let factor = digits
                    .parse::<u128>()
                    .ok()
                    .and_then(super::units::Unit::factor);
                if factor.is_none() && reports {
                    report.error(
                        Code::U2029,
                        *span,
                        format!(
                            "the factor `{digits}` is not a positive integer that a unit can take"
                        ),
                    );
                }
                factor
            }
            UnitExpr::Paren(inner, _) => self.unit_expr(module, inner, state, report),
            UnitExpr::Mul(a, b) | UnitExpr::Div(a, b) => {
                let left = self.unit_expr(module, a, state.as_deref_mut(), report);
                let right = self.unit_expr(module, b, state, report);
                let (left, right) = (left?, right?);
                let result = if matches!(expr, UnitExpr::Mul(..)) {
                    left.mul(&right)
                } else {
                    left.div(&right)
                };
                if result.is_none() && reports {
                    report.error(
                        Code::U2029,
                        expr.span(),
                        "the unit's scale is too large".to_owned(),
                    );
                }
                result
            }
        }
    }

    /// Lowers every signature: aliases, capabilities, kinds, functions and handlers. Capabilities
    /// come before kinds, whose fields may have function types with `uses`.
    fn signatures(&mut self, report: &mut Report<'_>) {
        self.lower_units(report);
        // ADR 0126 rule 9: the values of integer constants, for range bounds, before any type.
        self.compute_bound_values();
        for id in 0..self.aliases.len() {
            self.lower_alias(id, report);
        }
        for id in 0..self.capabilities.len() {
            self.capability_signature(id, report);
        }
        for id in 0..self.kinds.len() {
            let Some((module, params, ast)) = self
                .kinds
                .get(id)
                .map(|k| (k.module, k.params.clone(), k.ast))
            else {
                continue;
            };
            match &ast.body {
                KindBody::Sum(variants) => {
                    let lowered: Vec<Vec<FieldDef<'a>>> = variants
                        .iter()
                        .map(|variant| self.fields(module, &variant.fields, &params, report))
                        .collect();
                    if let Some(def) = self.kinds.get_mut(id) {
                        for (variant, fields) in def.variants.iter_mut().zip(lowered) {
                            variant.fields = fields;
                        }
                    }
                }
                KindBody::Record(fields) => {
                    let lowered = self.fields(module, fields, &params, report);
                    if let Some(def) = self.kinds.get_mut(id) {
                        def.record = Some(lowered);
                    }
                }
                KindBody::Alias { .. } => {}
            }
        }
        for id in 0..self.functions.len() {
            self.function_signature(id, report);
        }
        // ADR 0126 rule 6: the declared type of each module constant.
        for id in 0..self.constants.len() {
            let Some((module, ast)) = self.constants.get(id).map(|c| (c.module, c.ast)) else {
                continue;
            };
            // ADR 0126 rule 9: a constant may not be used in its own type.
            let own = self.text(module, ast.name.name);
            let bound = ast
                .ty
                .range
                .iter()
                .flat_map(|r| [&r.low, &r.high])
                .find_map(|b| {
                    b.constant
                        .as_ref()
                        .filter(|ident| self.text(module, ident.name) == own)
                });
            if let Some(ident) = bound {
                if self.reports(module) {
                    report.error(
                        Code::U1011,
                        ident.span,
                        format!("constant `{own}` is used in its own type"),
                    );
                }
                continue;
            }
            let declared = self.lower_type(module, &ast.ty, &[], report);
            if let Some(def) = self.constants.get_mut(id) {
                def.declared = declared;
            }
        }
        self.no_function_to_f64(report);
        self.no_function_to_decimal(report);
        // A policy fails and uses capabilities as its `via` function does (`spec/10 §7`).
        for id in 0..self.functions.len() {
            let Some((module, FnAst::Policy(policy))) =
                self.functions.get(id).map(|f| (f.module, f.ast))
            else {
                continue;
            };
            let via = self.text(module, policy.via.name);
            if let Some(Entity::Function(via)) = self.lower(module, via) {
                let (fails, uses) = self
                    .functions
                    .get(via)
                    .map_or((None, Vec::new()), |f| (f.fails.clone(), f.uses.clone()));
                if let Some(def) = self.functions.get_mut(id) {
                    def.fails = fails;
                    def.uses = uses;
                }
            }
        }
        for id in 0..self.handlers.len() {
            let Some((module, ast)) = self.handlers.get(id).map(|h| (h.module, h.ast)) else {
                continue;
            };
            let params = self.params_of(module, &ast.params, &[], Some("a handler"), report);
            let uses: Vec<Need> = first_of_each(
                ast.uses
                    .iter()
                    .map(|uses| self.lower_uses(module, uses, report))
                    .collect(),
            );
            let name = self.text(module, ast.capability.name);
            let capability = self.upper(module, name).into_iter().find_map(|e| match e {
                Entity::Capability(id) => Some(id),
                _ => None,
            });
            if let Some(def) = self.handlers.get_mut(id) {
                def.params = params;
                def.uses = uses;
                def.capability = capability;
            }
        }
    }

    fn capability_signature(&mut self, id: CapId, report: &mut Report<'_>) {
        let Some((module, ast)) = self.capabilities.get(id).map(|c| (c.module, c.ast)) else {
            return;
        };
        let mut ops = Vec::new();
        for op in &ast.ops {
            let fails = op.clauses.iter().find_map(|clause| match &clause.kind {
                OpClauseKind::Fails(kind) => Some(kind),
                _ => None,
            });
            let has = |pick: fn(&OpClauseKind) -> bool| op.clauses.iter().any(|c| pick(&c.kind));
            let op_name = self.text(module, op.name.name);
            let written = op.clauses.iter().find_map(|clause| match &clause.kind {
                OpClauseKind::Accepts(label) => Some(label),
                _ => None,
            });
            // ADR 0039: `Keys.hmac` accepts `public` for `key_name` and every label for
            // `data`, whose label its result carries (`spec/14 §3`). an
            // operation without `accepts` accepts `public`.
            let hmac = module == self.prelude
                && ast_name(self, module, ast) == "Keys"
                && op_name == "hmac";
            let carried: Vec<usize> = if hmac {
                op.params
                    .iter()
                    .position(|p| self.text(module, p.name.name) == "data")
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            let accepts = match written {
                Some(label) => Some(self.lower_label(module, label, report)),
                None => Some(Label::public()),
            };
            ops.push(OpDef {
                name: self.text(module, op.name.name),
                params: self.params_of(module, &op.params, &[], Some("an operation"), report),
                result: self.lower_type(module, &op.result, &[], report),
                result_span: op.result.span,
                fails: fails.map(|kind| self.error_kind(module, kind, report)),
                requires: has(|kind| matches!(kind, OpClauseKind::Requires(_))),
                ensures: has(|kind| matches!(kind, OpClauseKind::Ensures(_))),
                accepts,
                carried,
            });
        }
        if let Some(def) = self.capabilities.get_mut(id) {
            def.ops = ops;
        }
    }

    /// ADR 0114 rule 4.1: in edition 0.3, `n.to_f64()` on an integer is built in, ahead of a
    /// function of that name, so a package function `to_f64` whose first parameter has an
    /// integer type, a refinement or a measured one, or is a type parameter, is U1002 at its
    /// name: no program changes meaning silently.
    fn no_function_to_f64(&self, report: &mut Report<'_>) {
        if self.edition < Edition::V0_3 {
            return;
        }
        for def in &self.functions {
            let FnAst::Function(function) = def.ast else {
                continue;
            };
            if def.name != "to_f64" || !self.reports(def.module) {
                continue;
            }
            let integer = def.params.first().is_some_and(|param| {
                matches!(
                    param.ty.base.unlabeled(),
                    Ty::Int | Ty::Fixed(_) | Ty::Param(_) | Ty::Measured(..)
                )
            });
            if integer {
                report.error(
                    Code::U1002,
                    function.name.span,
                    "`to_f64` on an integer is a built-in conversion from edition 0.3, so a function cannot take its name for an integer or a type parameter".to_owned(),
                );
            }
        }
    }

    /// ADR 0120 rules 4, 8 and 15: in edition 0.5, `to_decimal` on an integer or a float, and
    /// `to_f32` on an integer or an `F64`, are built in, ahead of a function of that name, so a
    /// package function `to_decimal` whose first parameter has an integer type, a refinement
    /// or a measured one, `F64`, `F32`, or a type parameter, and a package function `to_f32`
    /// whose first parameter has an integer type, `F64`, or a type parameter, is U1002 at its
    /// name, and so is an import of one: no program changes meaning silently.
    fn no_function_to_decimal(&self, report: &mut Report<'_>) {
        if self.edition < Edition::V0_5 {
            return;
        }
        let built_in = |def: &FnDef<'_>| {
            let first = def.params.first().map(|param| param.ty.base.unlabeled());
            let integer = first.is_some_and(|ty| {
                matches!(ty, Ty::Int | Ty::Fixed(_) | Ty::Param(_) | Ty::Measured(..))
            });
            let float = first.is_some_and(|ty| matches!(ty, Ty::F64 | Ty::F32));
            match def.name {
                "to_decimal" => integer || float,
                "to_f32" => integer || first.is_some_and(|ty| matches!(ty, Ty::F64)),
                _ => false,
            }
        };
        // The conversion, and the first parameters it is built in for.
        let what = |name: &str| {
            if name == "to_f32" {
                (
                    "`to_f32` on an integer or an `F64`",
                    "an integer, an `F64` or a type parameter",
                )
            } else {
                (
                    "`to_decimal` on an integer or a float",
                    "an integer, an `F64`, an `F32` or a type parameter",
                )
            }
        };
        for def in &self.functions {
            let FnAst::Function(function) = def.ast else {
                continue;
            };
            if self.reports(def.module) && built_in(def) {
                let (conversion, firsts) = what(def.name);
                let mut diagnostic = report.make(
                    Code::U1002,
                    function.name.span,
                    format!(
                        "{conversion} is a built-in conversion from edition 0.5, so a function cannot take its name for {firsts}"
                    ),
                );
                // ADR 0120 "Migration": the rename is the program's choice, so `una fix` does
                // not apply it.
                diagnostic.repairs.push(super::Report::advice(format!(
                    "rename the function, for example to `as_{}`",
                    def.name.trim_start_matches("to_")
                )));
                report.push(diagnostic);
            }
        }
        // An import of such a function, from a dependency or another module, is U1002 at the
        // name it imports: the built-in would resolve ahead of it there too.
        for module in self.modules.iter().filter(|module| module.reports) {
            for import in &module.tree.imports {
                for ident in &import.names {
                    let name = module.interner.resolve(ident.name);
                    if name != "to_decimal" && name != "to_f32" {
                        continue;
                    }
                    let Some(Entity::Function(id)) = module.imports.get(name) else {
                        continue;
                    };
                    let foreign = self
                        .functions
                        .get(*id)
                        .is_some_and(|def| built_in(def) && !self.reports(def.module));
                    if foreign {
                        let (conversion, firsts) = what(name);
                        report.error(
                            Code::U1002,
                            ident.span,
                            format!(
                                "{conversion} is a built-in conversion from edition 0.5, so an imported function of that name for {firsts} would never be called"
                            ),
                        );
                    }
                }
            }
        }
    }

    fn function_signature(&mut self, id: usize, report: &mut Report<'_>) {
        let Some((module, ast)) = self.functions.get(id).map(|f| (f.module, f.ast)) else {
            return;
        };
        match ast {
            FnAst::Function(function) => {
                let type_params: Vec<&'a str> = function
                    .type_params
                    .iter()
                    .map(|p| self.text(module, p.name))
                    .collect();
                // `spec/20 §2`: a written constraint names `Eq`, `Ord` or `Integer` (U2027).
                let mut constraints = Vec::new();
                let mut constraints_written = Vec::new();
                for written in &function.constraints {
                    let constraint = written.as_ref().and_then(|ident| {
                        let name = self.text(module, ident.name);
                        let found = Constraint::from_name(name);
                        if found.is_none() && self.reports(module) {
                            report.error(
                                Code::U2027,
                                ident.span,
                                format!(
                                    "`{name}` is not a constraint; a type parameter takes `Eq`, `Ord` or `Integer`"
                                ),
                            );
                        }
                        found
                    });
                    constraints_written.push(written.is_some());
                    // After U2027, the parameter allows every operator, so that it reports
                    // nothing more.
                    constraints.push(match (constraint, written) {
                        (Some(constraint), _) => constraint,
                        (None, Some(_)) => Constraint::Integer,
                        (None, None) => Constraint::None,
                    });
                }
                constraints.resize(type_params.len(), Constraint::None);
                constraints_written.resize(type_params.len(), false);
                self.owner = Some(id);
                let params = self.params(module, &function.params, &type_params, report);
                let result = self.lower_type(module, &function.result, &type_params, report);
                self.owner = None;
                let fails = function
                    .clauses
                    .iter()
                    .find_map(|clause| match &clause.kind {
                        ClauseKind::Fails(kind) => Some(kind),
                        _ => None,
                    });
                let fails = fails.map(|kind| self.error_kind(module, kind, report));
                let has =
                    |pick: fn(&ClauseKind) -> bool| function.clauses.iter().any(|c| pick(&c.kind));
                let requires = has(|kind| matches!(kind, ClauseKind::Requires(_)));
                let ensures = has(|kind| matches!(kind, ClauseKind::Ensures(_)));
                let declares_uses =
                    has(|kind| matches!(kind, ClauseKind::Uses(_) | ClauseKind::UsesOf(_)));
                let declares_fails =
                    has(|kind| matches!(kind, ClauseKind::Fails(_) | ClauseKind::FailsOf(_)));
                let (uses_of, fails_of) = self.of_clauses(
                    module,
                    function,
                    &params,
                    declares_uses,
                    declares_fails,
                    report,
                );
                let uses: Vec<Need> = first_of_each(
                    function
                        .clauses
                        .iter()
                        .filter_map(|clause| match &clause.kind {
                            ClauseKind::Uses(uses) => Some(uses),
                            _ => None,
                        })
                        .map(|uses| self.lower_uses(module, uses, report))
                        .collect(),
                );
                if let Some(def) = self.functions.get_mut(id) {
                    def.type_params = type_params;
                    def.constraints = constraints;
                    def.constraints_written = constraints_written;
                    def.params = params;
                    def.result = result;
                    def.fails = fails;
                    def.requires = requires;
                    def.ensures = ensures;
                    // A private function without `uses` or `fails` infers them.
                    def.infer_uses = !function.public && !declares_uses;
                    // A function that passes the failures of a parameter, by `fails of p` written
                    // or inferred, fails with the kind of its argument only (ADR 0062).
                    def.infer_fails = !function.public && !declares_fails && fails_of.is_none();
                    def.uses = uses;
                    def.uses_of = uses_of;
                    def.fails_of = fails_of;
                    def.diverges = function
                        .clauses
                        .iter()
                        .any(|clause| matches!(clause.kind, ClauseKind::Diverges));
                }
            }
            FnAst::Predicate(predicate) => {
                let params =
                    self.params_of(module, &predicate.params, &[], Some("a predicate"), report);
                let result = self.lower_type(module, &predicate.result, &[], report);
                if let Some(def) = self.functions.get_mut(id) {
                    def.params = params;
                    def.result = result;
                }
            }
            FnAst::Policy(policy) => {
                let from = self.lower_type(module, &policy.from, &[], report);
                let to = self.lower_type(module, &policy.to, &[], report);
                if let Some(def) = self.functions.get_mut(id) {
                    def.params = vec![ParamDef {
                        name: "",
                        ty: from,
                        convention: None,
                        span: policy.from.span,
                    }];
                    def.result = to;
                }
            }
        }
    }

    /// The parameters that the `uses of p` and `fails of p` clauses of `function` name, with
    /// U3017 to U3019 for malformed ones (`spec/05 §9` rule 9); for a private function
    /// without such clauses, those inferred from its body (`spec/04 §5`, ADR 0062).
    fn of_clauses(
        &self,
        module: usize,
        function: &'a FunctionDecl,
        params: &[ParamDef<'a>],
        declares_uses: bool,
        declares_fails: bool,
        report: &mut Report<'_>,
    ) -> (Vec<usize>, Option<usize>) {
        // A parameter whose type is a function type without `uses` and `fails`.
        let open = |index: usize| {
            params.get(index).is_some_and(|param| {
                matches!(param.ty.base.unlabeled(), Ty::Fn(f) if f.uses.is_empty() && f.fails.is_none())
            })
        };
        let mut uses_of = Vec::new();
        let mut fails_of = None;
        for clause in &function.clauses {
            let (ident, uses) = match &clause.kind {
                ClauseKind::UsesOf(ident) => (ident, true),
                ClauseKind::FailsOf(ident) => (ident, false),
                _ => continue,
            };
            let name = self.text(module, ident.name);
            let Some(index) = params.iter().position(|param| param.name == name) else {
                report.error(
                    Code::U3017,
                    ident.span,
                    format!("`{name}` is not a parameter of this function; an `of` clause names a parameter of function type"),
                );
                continue;
            };
            let Some(Ty::Fn(f)) = params.get(index).map(|param| param.ty.base.unlabeled()) else {
                report.error(
                    Code::U3018,
                    ident.span,
                    format!("parameter `{name}` is not of function type, so it has no effects for an `of` clause"),
                );
                continue;
            };
            if !f.uses.is_empty() || f.fails.is_some() {
                report.error(
                    Code::U3019,
                    ident.span,
                    format!("the type of parameter `{name}` writes its own `uses` or `fails`, which fix its effects; remove them for an `of` clause"),
                );
                continue;
            }
            if uses {
                if !uses_of.contains(&index) {
                    uses_of.push(index);
                }
            } else {
                fails_of = Some(index);
            }
        }
        // ADR 0062 E3: a private function infers them from a body that only calls the
        // parameter or passes it on.
        if !function.public && self.edition >= Edition::V0_2 && (!declares_uses || !declares_fails)
        {
            let text = |ident: &Ident| self.text(module, ident.name).to_owned();
            let mut run: Vec<usize> = Vec::new();
            let mut passed: Vec<usize> = Vec::new();
            for index in 0..params.len() {
                let Some(param) = params.get(index) else {
                    continue;
                };
                if !open(index) {
                    continue;
                }
                let found = super::effects::mentions(&function.body, param.name, &text);
                if found.run && !found.other {
                    run.push(index);
                    // `fails of p` when the body passes the failure of `p` and fails in no
                    // other way, so its error kind is that of its argument.
                    if found.passed && !found.own_failures {
                        passed.push(index);
                    }
                }
            }
            if !declares_uses {
                uses_of = run;
            }
            if !declares_fails && let [only] = passed.as_slice() {
                fails_of = Some(*only);
            }
        }
        (uses_of, fails_of)
    }

    fn fields(
        &mut self,
        module: usize,
        fields: &[Field],
        params: &[&'a str],
        report: &mut Report<'_>,
    ) -> Vec<FieldDef<'a>> {
        fields
            .iter()
            .map(|field| FieldDef {
                name: self.text(module, field.name.name),
                ty: self.lower_type(module, &field.ty, params, report),
                span: field.ty.span,
            })
            .collect()
    }

    /// Lowers parameters with `type_params` in scope. Only functions declare conventions:
    /// one on a parameter of a handler, an operation or a predicate is U2020 (`spec/17 §3`).
    pub(crate) fn params(
        &mut self,
        module: usize,
        params: &[Param],
        type_params: &[&'a str],
        report: &mut Report<'_>,
    ) -> Vec<ParamDef<'a>> {
        self.params_of(module, params, type_params, None, report)
    }

    /// [`Env::params`] for `owner`, which names the construct that cannot declare conventions,
    /// or `None` for a function, which can.
    pub(crate) fn params_of(
        &mut self,
        module: usize,
        params: &[Param],
        type_params: &[&'a str],
        owner: Option<&str>,
        report: &mut Report<'_>,
    ) -> Vec<ParamDef<'a>> {
        params
            .iter()
            .map(|param| {
                if let (Some(convention), Some(owner)) = (param.convention, owner)
                    && self.reports(module)
                {
                    report.error(
                        Code::U2020,
                        param.span,
                        format!(
                            "a parameter of {owner} cannot be declared `{}`; only functions declare conventions",
                            convention.as_str()
                        ),
                    );
                }
                ParamDef {
                    name: self.text(module, param.name.name),
                    ty: self.lower_type(module, &param.ty, type_params, report),
                    convention: if owner.is_none() { param.convention } else { None },
                    span: param.ty.span,
                }
            })
            .collect()
    }

    /// `uses @name: C[ops]`: the capability, and the operations it permits, all of them when
    /// none is listed, in the capability's declaration order. A name that is not a capability
    /// is U3001, and an operation that the capability lacks U1001.
    pub(crate) fn lower_uses(
        &mut self,
        module: usize,
        uses: &UsesClause,
        report: &mut Report<'_>,
    ) -> Need {
        let name = self.text(module, uses.cap.name).to_owned();
        let cap_name = self.text(module, uses.cap_type.name.name);
        let reports = self.reports(module);
        let capability = match self.upper(module, cap_name).first().copied() {
            Some(Entity::Capability(id)) => Some(id),
            Some(_) => {
                if reports {
                    report.error(
                        Code::U3001,
                        uses.cap_type.name.span,
                        format!("`{cap_name}` is not a capability"),
                    );
                }
                None
            }
            // An unknown name is U1001, reported by M3.
            None => None,
        };
        // The names come from the declaration, not from `CapDef::ops`, which is filled only
        // after aliases are lowered: a kind alias's `uses` lists operations too.
        let declared: Vec<&'a str> = capability
            .and_then(|id| self.capabilities.get(id))
            .map(|c| {
                c.ast
                    .ops
                    .iter()
                    .map(|op| self.text(c.module, op.name.name))
                    .collect()
            })
            .unwrap_or_default();
        let listed: Vec<&'a str> = uses
            .cap_type
            .ops
            .iter()
            .map(|op| self.text(module, op.name))
            .collect();
        let ops: Vec<String> = if capability.is_none() {
            listed.iter().map(|op| (*op).to_owned()).collect()
        } else if listed.is_empty() {
            declared.iter().map(|op| (*op).to_owned()).collect()
        } else {
            for (op, ident) in listed.iter().zip(&uses.cap_type.ops) {
                if reports && !declared.contains(op) {
                    let mut diagnostic = report.make(
                        Code::U1001,
                        ident.span,
                        format!(
                            "capability `{}` has no operation `{op}`",
                            capability
                                .map_or_else(|| cap_name.to_owned(), |id| self.cap_display(id))
                        ),
                    );
                    if let Some(close) = closest(op, declared.iter().copied()) {
                        diagnostic.repairs.push(report.edit(
                            format!("write `{close}`"),
                            ident.span,
                            close,
                        ));
                    }
                    report.push(diagnostic);
                }
            }
            declared
                .iter()
                .filter(|op| listed.contains(op))
                .map(|op| (*op).to_owned())
                .collect()
        };
        // ADR 0094 rule 1: the label each listed operation gives through this binding, in the
        // capability's declaration order, `public` left out; its categories are checked as a
        // type's are (U4007).
        let mut gives: Vec<(String, Label)> = Vec::new();
        for (op, label) in listed.iter().zip(&uses.cap_type.gives) {
            let Some(label) = label else {
                continue;
            };
            let lowered = self.lower_label(module, label, report);
            if !lowered.is_public() && ops.iter().any(|known| known.as_str() == *op) {
                match gives.iter_mut().find(|(known, _)| known == *op) {
                    // An operation listed twice gives the join of its labels.
                    Some((_, known)) => *known = known.join(&lowered),
                    None => gives.push(((*op).to_owned(), lowered)),
                }
            }
        }
        gives.sort_by_key(|(op, _)| ops.iter().position(|listed| listed == op));
        Need {
            usage: FnUse {
                name,
                capability,
                ops,
                gives,
            },
            span: uses.span,
        }
    }

    /// The kind named by `fails E`: a sum kind without type parameters (U2002).
    pub(crate) fn error_kind(
        &mut self,
        module: usize,
        ident: &Ident,
        report: &mut Report<'_>,
    ) -> Ty {
        let name = self.text(module, ident.name);
        let problem = match self.upper(module, name).first() {
            // Unknown names are U1001, reported by M3.
            None => return Ty::unknown(),
            Some(Entity::Kind(id)) => match self.kinds.get(*id) {
                Some(kind) if kind.is_sum() && kind.params.is_empty() => {
                    return Ty::Kind(*id, Vec::new().into());
                }
                Some(kind) if kind.is_sum() => {
                    format!("error kind `{name}` has type parameters, which `fails` cannot give")
                }
                _ => format!("`fails` names a sum kind, and `{name}` is a record"),
            },
            Some(_) => format!("`fails` names a sum kind, and `{name}` is not one"),
        };
        if self.reports(module) {
            report.error(Code::U2002, ident.span, problem);
        }
        Ty::error()
    }

    /// Lowers the base of an alias; an alias reached again while its own base is lowered is
    /// defined through itself (U2002).
    fn lower_alias(&mut self, id: AliasId, report: &mut Report<'_>) {
        if self.alias_state.get(id) != Some(&AliasState::Pending) {
            return;
        }
        if let Some(state) = self.alias_state.get_mut(id) {
            *state = AliasState::Lowering;
        }
        let Some((module, params, ty)) = self
            .aliases
            .get(id)
            .map(|a| (a.module, a.params.clone(), a.ty))
        else {
            return;
        };
        let base = self.lower_type(module, ty, &params, report);
        // ADR 0045 rules 1 and 2: Core IR has no refinement over a function base, so a `where`
        // clause on a kind whose base is a function type is ill-formed, at the clause. The
        // clause is dropped after the report, so an alias of the kind is not reported again
        // and no refinement is lost later.
        let fn_base = matches!(base.base.unlabeled(), Ty::Fn(_));
        let where_clause = self
            .aliases
            .get(id)
            .and_then(|alias| alias.refinement.map(|expr| (alias.name, expr.span)));
        if fn_base && let Some((name, span)) = where_clause {
            if self.reports(module) {
                report.error(
                    Code::U2002,
                    span,
                    format!(
                        "kind `{name}` refines a function type, which cannot be refined; remove the `where` clause, or refine the parameters or the result of the function type"
                    ),
                );
            }
            if let Some(alias) = self.aliases.get_mut(id) {
                alias.refinement = None;
            }
        }
        if let Some(alias) = self.aliases.get_mut(id) {
            alias.base = base;
        }
        if let Some(state) = self.alias_state.get_mut(id) {
            *state = AliasState::Done;
        }
    }

    /// Lowers a written type of module `module`, with `type_params` in scope. Labels wait for
    /// M6.
    /// Whether a unit may follow `ty`, a written base type without its label: an integer
    /// type, `F64` from edition 0.3 (ADR 0114 rule 12.1), and `F32`, `Decimal[s]`,
    /// `Complex[F64]` and `Complex[F32]` in edition 0.5 (ADR 0120 rule 61). Those types exist
    /// only in the editions that have them, so the type alone decides.
    fn measurable(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Int | Ty::Fixed(_) | Ty::F64 | Ty::F32 | Ty::Decimal(_) => true,
            Ty::Kind(id, args) => {
                args.is_empty()
                    && self.kinds.get(*id).is_some_and(|kind| {
                        kind.module == self.prelude
                            && matches!(
                                kind.name,
                                una_core::complex::COMPLEX_F64 | una_core::complex::COMPLEX_F32
                            )
                    })
            }
            _ => false,
        }
    }

    pub(crate) fn lower_type(
        &mut self,
        module: usize,
        ty: &Type,
        type_params: &[&'a str],
        report: &mut Report<'_>,
    ) -> Declared {
        let mut declared = match &ty.core {
            TypeCore::Named { name, args } => {
                let lowered: Vec<Declared> = args
                    .iter()
                    .map(|arg| self.lower_type(module, arg, type_params, report))
                    .collect();
                for (arg, lowered) in args.iter().zip(&lowered) {
                    self.type_argument(module, arg.span, &lowered.base, report);
                    self.refined_argument(module, arg, lowered, type_params, report);
                }
                let lowered = lowered.into_iter().map(|arg| self.nested(arg)).collect();
                self.named(module, name, lowered, ty.span, type_params, report)
            }
            TypeCore::Decimal {
                scale, scale_span, ..
            } => match scale.parse::<u8>() {
                Ok(scale) if scale <= una_core::decimal::MAX_SCALE => {
                    Declared::plain(Ty::Decimal(scale))
                }
                _ => {
                    if self.reports(module) {
                        report.error(
                            Code::U2037,
                            *scale_span,
                            format!(
                                "the scale of `Decimal[{scale}]` is not an integer from 0 to 38, the scales of a decimal"
                            ),
                        );
                    }
                    Declared::error()
                }
            },
            TypeCore::List(element) => {
                let lowered = self.lower_type(module, element, type_params, report);
                self.type_argument(module, element.span, &lowered.base, report);
                self.refined_argument(module, element, &lowered, type_params, report);
                Declared::plain(Ty::List(Shared::new(self.nested(lowered).base)))
            }
            TypeCore::Function {
                params,
                result,
                clauses,
            } => {
                // ADR 0037 rule 1: the refinements of the parameters and the result are part
                // of the function type.
                let (params, param_refinements): (Vec<Ty>, Vec<Vec<Refinement>>) = params
                    .iter()
                    .map(|param| {
                        let lowered = self.lower_type(module, param, type_params, report);
                        (lowered.base, lowered.refinements)
                    })
                    .unzip();
                let result = self.lower_type(module, result, type_params, report);
                let refinements = FnRefinements {
                    params: param_refinements,
                    result: result.refinements,
                };
                let result = result.base;
                let fails = clauses.iter().find_map(|clause| match clause {
                    FnTypeClause::Fails(kind) => Some(kind),
                    FnTypeClause::Uses(_) => None,
                });
                let fails = fails.map(|kind| self.error_kind(module, kind, report));
                let clauses: Vec<&UsesClause> = clauses
                    .iter()
                    .filter_map(|clause| match clause {
                        FnTypeClause::Uses(uses) => Some(uses),
                        FnTypeClause::Fails(_) => None,
                    })
                    .collect();
                if self.reports(module) {
                    let names: Vec<(&str, Span)> = clauses
                        .iter()
                        .map(|uses| (self.text(module, uses.cap.name), uses.cap.span))
                        .collect();
                    super::authority::duplicate_refs(report, &names);
                }
                let uses: Vec<FnUse> = first_of_each(
                    clauses
                        .into_iter()
                        .map(|uses| self.lower_uses(module, uses, report))
                        .collect(),
                )
                .into_iter()
                .map(|need| need.usage)
                .collect();
                Declared::plain(Ty::Fn(Shared::new(FnTy {
                    params,
                    result,
                    uses,
                    fails,
                    refinements,
                })))
            }
        };
        // ADR 0104 rule 2: a unit after `of`, on a number type only: an integer type, `F64`
        // from edition 0.3 (ADR 0114 rule 12.1), and `F32`, `Decimal[s]` and the complex types
        // in edition 0.5 (ADR 0120 rule 61).
        if let Some(unit) = &ty.unit {
            let measurable = self.measurable(declared.base.unlabeled());
            match declared.base.unlabeled() {
                _ if measurable => match self.unit_expr(module, unit, None, report) {
                    Some(unit) if unit.is_one() => {}
                    Some(unit) => {
                        // The label of an alias's base stays outside the unit, where the label
                        // operations see it (ADR 0077); the magnitude holds none.
                        let label = declared.base.top_label();
                        declared.base = Ty::measured(unit, declared.base.unlabeled().clone())
                            .with_label(&label);
                    }
                    // A unit that does not resolve, after its U2029, makes the type an error,
                    // so that the uses of the value report nothing more.
                    None => declared.base = Ty::error(),
                },
                Ty::Error(_) => {}
                _ => {
                    if self.reports(module) {
                        let message = if self.edition >= Edition::V0_5 {
                            "a unit follows only a number type: `Int`, a fixed-width integer type, `F64`, `F32`, `Decimal[s]`, `Complex[F64]` or `Complex[F32]`"
                        } else if self.edition >= Edition::V0_3 {
                            "a unit follows only a number type: `Int`, a fixed-width integer type or `F64`"
                        } else {
                            "a unit follows only an integer type, `Int` or a fixed-width one"
                        };
                        report.error(Code::U2029, unit.span(), message.to_owned());
                    }
                }
            }
        }
        // ADR 0126 rule 9: a bound that names a constant is its value, substituted before the
        // range is read.
        let resolved = ty
            .range
            .as_ref()
            .and_then(|range| self.range_constants(module, range, report));
        if resolved.is_none() && ty.range.as_ref().is_some_and(has_constant_bound) {
            declared.base = Ty::error();
            return declared;
        }
        if let Some(range) = resolved.as_ref() {
            // Range sugar on a fixed-width type: its bounds fit the type (`spec/18 §1`).
            if let Ty::Fixed(int) = declared.base.magnitude()
                && self.reports(module)
            {
                for bound in [&range.low, &range.high] {
                    if !super::fixed::fits(*int, bound.negative, &bound.digits) {
                        let sign = if bound.negative { "-" } else { "" };
                        report.error(
                            Code::U2022,
                            bound.span,
                            format!(
                                "the bound `{sign}{}` does not fit `{}`, which holds `{}` to `{}`",
                                bound.digits,
                                int.as_str(),
                                int.least(),
                                int.greatest()
                            ),
                        );
                    }
                }
            }
            if matches!(
                declared.base.magnitude(),
                Ty::Int | Ty::Fixed(_) | Ty::Error(_)
            ) {
                let bound = |i: &una_syntax::ast::SignedInt| {
                    format!("{}{}", if i.negative { "-" } else { "" }, i.digits)
                };
                let dots = if range.exclusive { "..<" } else { ".." };
                declared.refinements.push(Refinement::Range(
                    format!("in {}{dots}{}", bound(&range.low), bound(&range.high)),
                    range.span,
                ));
            } else if self.reports(module) {
                let message = if self.edition >= Edition::V0_2 {
                    "range sugar applies only to integer types"
                } else {
                    "range sugar applies only to `Int`"
                };
                report.error(Code::U2002, range.span, message);
            }
        }
        if let Some(label) = &ty.label {
            let label = self.lower_label(module, label, report);
            declared.base = declared.base.with_label(&label);
        }
        declared
    }

    /// The range `range` with each bound that names a constant replaced by the constant's
    /// value (ADR 0126 rule 9): U1001 when the name does not resolve (reported by M3), U2002
    /// when it is not a constant of an integer type. `None` when a bound cannot be read; a
    /// range without such bounds is returned as written.
    fn range_constants(
        &self,
        module: usize,
        range: &una_syntax::ast::Range,
        report: &mut Report<'_>,
    ) -> Option<una_syntax::ast::Range> {
        let mut resolved = range.clone();
        for bound in [&mut resolved.low, &mut resolved.high] {
            let Some(ident) = bound.constant.clone() else {
                continue;
            };
            let name = self.text(module, ident.name);
            let entity = self.lower(module, name);
            let value = match entity {
                Some(Entity::Const(id)) => {
                    // An integer constant without a label or a unit: a bound is a plain integer
                    // of the range, so a label or a unit written on the constant would be
                    // dropped there (#1632 review, L1).
                    let integer = self.constants.get(id).is_some_and(|c| {
                        c.ast.ty.label.is_none()
                            && c.ast.ty.unit.is_none()
                            && matches!(&c.ast.ty.core, TypeCore::Named { name, args }
                                if args.is_empty()
                                    && (self.text(c.module, name.name) == "Int"
                                        || IntType::from_name(self.text(c.module, name.name)).is_some()))
                    });
                    if !integer {
                        if self.reports(module) {
                            report.error(
                                Code::U2002,
                                ident.span,
                                format!("the range bound `{name}` names a constant that is not a plain integer: a bound's constant has an integer type without a label or a unit"),
                            );
                        }
                        return None;
                    }
                    // The values are read in dependency order before any type is lowered, so a
                    // chain of any length has its value here (#1632 review, H1).
                    match self.bound_values.get(&id).cloned().flatten() {
                        Some(value) => value,
                        None => {
                            if self.reports(module) {
                                report.error(
                                    Code::U2002,
                                    ident.span,
                                    format!("the range bound `{name}` names a constant whose value is not an integer constant expression the compiler can compute"),
                                );
                            }
                            return None;
                        }
                    }
                }
                // A name M3 did not resolve (U1001), or one it resolved to a local, such as a
                // parameter: never a silent error type.
                None => {
                    if self.reports(module) {
                        report.error(
                            Code::U2002,
                            ident.span,
                            format!("the range bound `{name}` is not a module constant; a bound is an integer literal or the name of an integer constant"),
                        );
                    }
                    return None;
                }
                Some(_) => {
                    if self.reports(module) {
                        report.error(
                            Code::U2002,
                            ident.span,
                            format!("the range bound `{name}` is not a constant; a bound is an integer literal or the name of an integer constant"),
                        );
                    }
                    return None;
                }
            };
            bound.negative = value.sign() == una_core::bigint::Sign::Minus;
            let magnitude = if bound.negative { -value } else { value };
            bound.digits = magnitude.to_string();
        }
        Some(resolved)
    }

    /// The integer value of every module constant whose value is an integer constant
    /// expression, for range bounds (ADR 0126 rule 9), read from the source before any type is
    /// lowered. The constants are taken in dependency order (Kahn), so a chain of any length
    /// is read without recursion across constants; a constant in a cycle, or one that names a
    /// constant without such a value, has `None`, which the evaluation of rule 5 reports.
    /// O(constants + size of their values).
    fn compute_bound_values(&mut self) {
        let count = self.constants.len();
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut users: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (id, def) in self.constants.iter().enumerate() {
            let mut found = std::collections::BTreeSet::new();
            names_of(self, def.module, &def.ast.value, &mut found);
            for &dep in &found {
                if let Some(list) = users.get_mut(dep) {
                    list.push(id);
                }
            }
            if let Some(slot) = deps.get_mut(id) {
                *slot = found.into_iter().collect();
            }
        }
        let mut waiting: Vec<usize> = deps.iter().map(Vec::len).collect();
        let mut ready: Vec<usize> = (0..count)
            .filter(|id| waiting.get(*id) == Some(&0))
            .collect();
        let mut values: BTreeMap<usize, Option<BigInt>> = BTreeMap::new();
        while let Some(id) = ready.pop() {
            let value = self.constants.get(id).and_then(|def| {
                self.constant_int_expr(def.module, &def.ast.value, &values)
                    .ok()
            });
            values.insert(id, value);
            for &user in users.get(id).into_iter().flatten() {
                if let Some(count) = waiting.get_mut(user) {
                    *count -= 1;
                    if *count == 0 {
                        ready.push(user);
                    }
                }
            }
        }
        self.bound_values = values;
    }

    fn constant_int_expr(
        &self,
        module: usize,
        expr: &una_syntax::ast::Expr,
        values: &BTreeMap<usize, Option<BigInt>>,
    ) -> Result<BigInt, bool> {
        use una_syntax::ast::{BinaryOp, ExprKind, Literal, UnaryOp};
        let value = match &expr.kind {
            ExprKind::Paren(inner) => self.constant_int_expr(module, inner, values)?,
            ExprKind::Literal(Literal::Int(digits)) => {
                let clean: String = digits.chars().filter(|c| *c != '_').collect();
                clean.parse::<BigInt>().map_err(|_| false)?
            }
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => -self.constant_int_expr(module, operand, values)?,
            ExprKind::Binary { op, left, right } => {
                let a = self.constant_int_expr(module, left, values)?;
                let b = self.constant_int_expr(module, right, values)?;
                match op {
                    BinaryOp::Add => a + b,
                    BinaryOp::Sub => a - b,
                    BinaryOp::Mul => a * b,
                    BinaryOp::Div | BinaryOp::Mod => {
                        if b.sign() == una_core::bigint::Sign::NoSign {
                            return Err(false);
                        }
                        let (q, r) = super::constants::div_euclid(&a, &b);
                        if *op == BinaryOp::Div { q } else { r }
                    }
                    _ => return Err(false),
                }
            }
            ExprKind::Name(ident) => match self.lower(module, self.text(module, ident.name)) {
                Some(Entity::Const(other)) => values.get(&other).cloned().flatten().ok_or(false)?,
                _ => return Err(false),
            },
            _ => return Err(false),
        };
        // Past the digit cap of rule 5, which the evaluation reports (2^14,300 > 10^4,300).
        if value.bits() > 14_300 {
            return Err(false);
        }
        Ok(value)
    }

    /// U3016: a written type argument that is, or contains, a second-class type, a function
    /// type with `uses` or `fails` (`spec/05 §6`, ADR 0038 D2). D-142: it is
    /// reported at the innermost
    /// type argument that is a function type, once.
    fn type_argument(&self, module: usize, span: Span, ty: &Ty, report: &mut Report<'_>) {
        if self.reports(module) && matches!(ty.unlabeled(), Ty::Fn(_)) && ty.second_class() {
            report.error(
                Code::U3016,
                span,
                "this type argument is, or holds, a function type with `uses` or `fails`, which is second-class, so it cannot instantiate a type parameter; take the function as a parameter of function type instead",
            );
        }
    }

    /// A written type argument as the nested position it is: in edition 0.3, its refinements
    /// stay, as a refined position (ADR 0103 rule 1); in editions 0.1 and 0.2, its base, since
    /// [`Env::refined_argument`] reported it (ADR 0040).
    /// The type a binder declared at `declared` gives its uses in edition `0.3`: the base with
    /// its refinements at the top, inside its label, so that rule 5 of ADR 0103 sees a `Pos`
    /// parameter as a `Pos` (`or_else` is `Pos` when both its arguments are) and an annotated
    /// `let` fixes the refinements of the type arguments it is the expected type of. Every
    /// refinement here is proved where the binder is bound. Earlier editions, the base.
    pub(crate) fn top_refined(&self, declared: &Declared) -> Ty {
        if self.edition < una_project::Edition::V0_3 || declared.refinements.is_empty() {
            return declared.base.clone();
        }
        let label = declared.base.top_label();
        let base = declared.base.unlabeled().clone();
        Ty::Refined(
            declared.refinements.iter().cloned().collect(),
            Shared::new(base),
        )
        .with_label(&label)
    }

    fn nested(&self, lowered: Declared) -> Declared {
        if self.edition < una_project::Edition::V0_3 || lowered.refinements.is_empty() {
            return Declared::plain(lowered.base);
        }
        // The refinements sit inside the position's label, `Labeled(ℓ, Refined(r, base))`.
        let label = lowered.base.top_label();
        let base = lowered.base.unlabeled().clone();
        Declared::plain(
            Ty::Refined(lowered.refinements.into_iter().collect(), Shared::new(base))
                .with_label(&label),
        )
    }

    /// U2002: a written type argument of a refined kind, or with range sugar (ADR 0040). A type
    /// that contains one is reported at the innermost refined type argument, once, since the
    /// lowered argument keeps only its base.
    fn refined_argument(
        &self,
        module: usize,
        arg: &Type,
        lowered: &Declared,
        type_params: &[&'a str],
        report: &mut Report<'_>,
    ) {
        // ADR 0103 rule 1: in edition 0.3 a refined type argument is well-formed.
        if !self.reports(module)
            || !lowered.is_refined()
            || self.edition >= una_project::Edition::V0_3
        {
            return;
        }
        let base = Show(
            &lowered.base,
            &LowerNames {
                env: self,
                type_params,
            },
        )
        .to_string();
        // A refined kind is named as written, not by the kind or range its alias goes
        // through.
        let written = match (&arg.core, &arg.range) {
            (TypeCore::Named { name, .. }, None) => Some(self.text(module, name.name)),
            _ => None,
        };
        let what = match (lowered.refinements.last(), written) {
            (None, _) => return,
            (Some(_), Some(name)) => format!("the refined kind `{name}`"),
            (Some(Refinement::Alias(id, _)), None) => {
                let name = self.aliases.get(*id).map_or("?", |a| a.name);
                format!("the refined kind `{name}`")
            }
            (Some(Refinement::Range(range, _)), None) => {
                format!("the refined type `{base} {range}`")
            }
        };
        // The rule holds in every edition; edition 0.1 keeps its message (#188).
        let version = if self.edition >= Edition::V0_2 {
            ""
        } else {
            " in v0.1"
        };
        let mut diagnostic = report.make(
            Code::U2002,
            arg.span,
            format!(
                "this type argument is {what}, and a refined type cannot instantiate a type parameter{version}; write its base type `{base}` and check the refinement where the value is used"
            ),
        );
        if arg.label.is_none() {
            diagnostic.repairs.push(report.edit(
                format!("write the base type `{base}`"),
                arg.span,
                base.clone(),
            ));
        }
        report.push(diagnostic);
    }

    /// A written label: its level and categories; a category that `una.toml` does not
    /// declare is U4007 and left out (`spec/07 §2`).
    pub(crate) fn lower_label(
        &self,
        module: usize,
        label: &LabelAst,
        report: &mut Report<'_>,
    ) -> Label {
        let mut lowered = Label {
            level: label.level,
            categories: una_core::Names::new(),
            symbols: una_core::Names::new(),
        };
        for category in &label.categories {
            let name = self.text(module, category.name);
            if self.categories.iter().any(|declared| declared == name) {
                lowered.categories.insert(name);
            } else if self.reports(module) {
                let mut diagnostic = report.make(
                    Code::U4007,
                    category.span,
                    format!(
                        "label category `{name}` is not declared in `[labels] categories` of `una.toml`"
                    ),
                );
                if let Some(close) = closest(name, self.categories.iter().map(String::as_str)) {
                    diagnostic.repairs.push(report.edit(
                        format!("write `{close}`"),
                        category.span,
                        close,
                    ));
                }
                report.push(diagnostic);
            }
        }
        lowered
    }

    /// Lowers `name[args]`.
    fn named(
        &mut self,
        module: usize,
        ident: &Ident,
        args: Vec<Declared>,
        span: Span,
        type_params: &[&'a str],
        report: &mut Report<'_>,
    ) -> Declared {
        let name = self.text(module, ident.name);
        let reports = self.reports(module);
        let mut arity = |expected: usize, found: usize| {
            if reports {
                let takes = match expected {
                    0 => "takes no type arguments".to_owned(),
                    1 => "takes 1 type argument".to_owned(),
                    n => format!("takes {n} type arguments"),
                };
                let given = match found {
                    0 => "none is given".to_owned(),
                    1 => "1 is given".to_owned(),
                    n => format!("{n} are given"),
                };
                report.error(
                    Code::U2003,
                    span,
                    format!("type `{name}` {takes}, but {given}"),
                );
            }
            Declared::error()
        };
        if let Some(index) = type_params.iter().position(|param| *param == name) {
            if !args.is_empty() {
                return arity(0, args.len());
            }
            return Declared::plain(Ty::Param(index));
        }
        // Edition 0.5: `Decimal` is the decimal type only with its scale, an integer literal,
        // in brackets (ADR 0120 rule 2); the name alone is the `Json` variant.
        if self.edition >= Edition::V0_5 && name == "Decimal" {
            if reports {
                report.error(
                    Code::U2037,
                    span,
                    "the scale of `Decimal` is an integer literal from 0 to 38 in brackets, as in `Decimal[2]`".to_owned(),
                );
            }
            return Declared::error();
        }
        let bases: Vec<Ty> = args.into_iter().map(|arg| arg.base).collect();
        // Edition 0.5 (ADR 0120 rule 3): `Array`, `Matrix`, `Tensor` and `Lu` are built-in
        // types, which no declaration can hide; the prelude's variant `Json.Array` is a constructor, written
        // with parentheses, so it never meets the type, written with brackets.
        if let Some(dense) = self.dense_name(name) {
            if bases.len() != 1 {
                return arity(1, bases.len());
            }
            let element = bases.into_iter().next().unwrap_or(Ty::error());
            // The prelude declares the methods of `Matrix[T]`, `Tensor[T]` and `Lu[T]` over a
            // type parameter, whose element types its callers' receivers decide.
            let taken = match dense {
                Dense::Array => true,
                Dense::Matrix | Dense::Tensor => dense_numeric(&element),
                Dense::Lu => dense_float(&element),
            };
            if !taken && reports && module != self.prelude {
                let wanted = if dense == Dense::Lu {
                    "a float element type, `F64`"
                } else {
                    "a numeric element type, `F64` or one of `I8` to `U64`"
                };
                report.error(
                    Code::U2002,
                    span,
                    format!(
                        "`{}` takes {wanted}, and `{}` is not one",
                        dense.as_str(),
                        Show(
                            &element,
                            &LowerNames {
                                env: self,
                                type_params,
                            }
                        )
                    ),
                );
                return Declared::error();
            }
            return Declared::plain(Ty::Dense(dense, Shared::new(element)));
        }
        let problem = match self.upper(module, name).first().copied() {
            Some(Entity::Builtin(builtin)) => {
                let expected = match builtin {
                    Builtin::List | Builtin::Complex => 1,
                    Builtin::Map => 2,
                    Builtin::Coll(coll) => coll.arity(),
                    _ => 0,
                };
                if bases.len() != expected {
                    return arity(expected, bases.len());
                }
                if builtin == Builtin::Complex {
                    return self.complex_type(&bases, span, reports, report);
                }
                let base = match builtin {
                    Builtin::Int => Ty::Int,
                    Builtin::Fixed(int) => Ty::Fixed(int),
                    Builtin::Bool => Ty::Bool,
                    Builtin::Text => Ty::Text,
                    Builtin::Unit => Ty::Unit,
                    Builtin::Hashed => Ty::Hashed,
                    Builtin::ReplyRight => Ty::ReplyRight,
                    Builtin::F64 => Ty::F64,
                    Builtin::F32 => Ty::F32,
                    // Lowered above.
                    Builtin::Complex => Ty::error(),
                    Builtin::List => {
                        Ty::List(Shared::new(bases.into_iter().next().unwrap_or(Ty::error())))
                    }
                    Builtin::Map => {
                        let mut bases = bases.into_iter();
                        let key = bases.next().unwrap_or(Ty::error());
                        let value = bases.next().unwrap_or(Ty::error());
                        if reports {
                            self.map_keys.push(MapKey {
                                key: key.clone(),
                                span,
                                owner: self.owner,
                                of: None,
                            });
                        }
                        Ty::Map(Shared::new(key), Shared::new(value))
                    }
                    // ADR 0119: a set's item and a priority must satisfy `Ord`, and a hash key
                    // or item `Eq`, where the type is written, as a map's key must.
                    Builtin::Coll(coll) => {
                        if reports {
                            for (index, arg) in bases.iter().enumerate() {
                                if coll.needs(index) != una_core::Constraint::None {
                                    self.map_keys.push(MapKey {
                                        key: arg.clone(),
                                        span,
                                        owner: self.owner,
                                        of: Some((coll, index)),
                                    });
                                }
                            }
                        }
                        Ty::Coll(coll, bases.into())
                    }
                };
                return Declared::plain(base);
            }
            Some(Entity::Kind(id)) => {
                let expected = self.kinds.get(id).map_or(0, |k| k.params.len());
                if bases.len() != expected {
                    return arity(expected, bases.len());
                }
                return Declared::plain(Ty::Kind(id, bases.into()));
            }
            Some(Entity::Alias(id)) => {
                let expected = self.aliases.get(id).map_or(0, |a| a.params.len());
                if bases.len() != expected {
                    return arity(expected, bases.len());
                }
                return self.alias(id, &bases, ident, reports, report);
            }
            // ADR 0139 rule 7: in edition 0.4, `Array` is still `Json`'s variant; the dense
            // type moved to edition 0.5.
            Some(Entity::Variant(kind, _))
                if self.edition == Edition::V0_4 && matches!(name, "Array" | "Decimal") =>
            {
                format!(
                    "`{name}` is a variant of `{}`, not a type; the type `{name}` exists from edition 0.5 (ADR 0139): write `edition 0.5` in ita.una to use it",
                    self.kind_display(kind)
                )
            }
            Some(Entity::Variant(kind, _)) => {
                format!(
                    "`{name}` is a variant of `{}`, not a type",
                    self.kind_display(kind)
                )
            }
            Some(Entity::Capability(_)) => {
                format!("`{name}` is a capability, not a type; capabilities are not values")
            }
            // ADR 0104: a unit follows `of` after an integer type.
            Some(Entity::Unit(_)) => {
                format!("`{name}` is a unit, not a type; write `Int of {name}`")
            }
            Some(Entity::Constant { .. } | Entity::Const(_)) => return Declared::error(),
            // Unknown names are U1001, reported by M3.
            None => return Declared::unknown(),
            Some(Entity::Function(_) | Entity::Handler(_)) => return Declared::error(),
        };
        if reports {
            report.error(Code::U2002, ident.span, problem);
        }
        Declared::error()
    }

    /// The type an alias denotes with arguments `args`: its base, refined by its own predicate
    /// and those of the aliases it goes through (`spec/03 §2` rule 3).
    fn alias(
        &mut self,
        id: AliasId,
        args: &[Ty],
        ident: &Ident,
        reports: bool,
        report: &mut Report<'_>,
    ) -> Declared {
        match self.alias_state.get(id) {
            Some(AliasState::Lowering) => {
                if reports {
                    let name = self.aliases.get(id).map_or("?", |a| a.name);
                    report.error(
                        Code::U2002,
                        ident.span,
                        format!("kind `{name}` is defined in terms of itself"),
                    );
                }
                return Declared::error();
            }
            Some(AliasState::Pending) => self.lower_alias(id, report),
            _ => {}
        }
        let Some(alias) = self.aliases.get(id) else {
            return Declared::error();
        };
        let mut refinements: Vec<Refinement> = alias
            .base
            .refinements
            .iter()
            .map(|refinement| match refinement {
                Refinement::Alias(inner, inner_args) => Refinement::Alias(
                    *inner,
                    inner_args.iter().map(|arg| arg.substitute(args)).collect(),
                ),
                Refinement::Range(text, span) => Refinement::Range(text.clone(), *span),
            })
            .collect();
        if alias.refinement.is_some() {
            refinements.push(Refinement::Alias(id, args.to_vec()));
        }
        Declared {
            base: alias.base.base.substitute(args),
            refinements,
        }
    }
}

/// Names for printing lowered types in messages of the environment.
struct LowerNames<'e, 'a> {
    env: &'e Env<'a>,
    type_params: &'e [&'a str],
}

impl Names for LowerNames<'_, '_> {
    fn kind_name(&self, id: KindId) -> String {
        self.env.kind_display(id)
    }

    fn param_name(&self, index: usize) -> String {
        self.type_params
            .get(index)
            .map_or_else(|| "?".to_owned(), |name| (*name).to_owned())
    }

    fn refined_name(&self, refinements: &[Refinement], base: &str) -> Option<String> {
        self.env.refined_display(refinements, base)
    }
}

impl Env<'_> {
    /// `ty` as messages print it, outside any declaration with type parameters: for the
    /// shapes of a driven module, which declare none (`spec/26 §2` rule 2).
    pub(crate) fn show_plain(&self, ty: &Ty) -> String {
        Show(
            ty,
            &LowerNames {
                env: self,
                type_params: &[],
            },
        )
        .to_string()
    }
}

/// The first requirement of each name: a reference bound twice is U1002, and the first binding
/// wins, as the first of two declarations does.
fn first_of_each(needs: Vec<Need>) -> Vec<Need> {
    let mut kept: Vec<Need> = Vec::new();
    for need in needs {
        if !kept.iter().any(|k| k.usage.name == need.usage.name) {
            kept.push(need);
        }
    }
    kept
}

/// The name of a capability declaration.
fn ast_name<'a>(env: &Env<'a>, module: usize, ast: &'a CapabilityDecl) -> &'a str {
    env.text(module, ast.name.name)
}

/// Whether a bound of `range` names a constant (ADR 0126 rule 9).
fn has_constant_bound(range: &una_syntax::ast::Range) -> bool {
    range.low.constant.is_some() || range.high.constant.is_some()
}

/// The module constants that `expr`, a value of module `module`, names.
fn names_of(
    env: &Env<'_>,
    module: usize,
    expr: &una_syntax::ast::Expr,
    found: &mut std::collections::BTreeSet<usize>,
) {
    use una_syntax::ast::ExprKind;
    match &expr.kind {
        ExprKind::Name(ident) => {
            if let Some(Entity::Const(id)) = env.lower(module, env.text(module, ident.name)) {
                found.insert(id);
            }
        }
        ExprKind::Paren(inner) | ExprKind::Unary { operand: inner, .. } => {
            names_of(env, module, inner, found);
        }
        ExprKind::Binary { left, right, .. } => {
            names_of(env, module, left, found);
            names_of(env, module, right, found);
        }
        ExprKind::List(items) => {
            for item in items {
                names_of(env, module, item, found);
            }
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                names_of(env, module, &arg.value, found);
            }
        }
        _ => {}
    }
}
