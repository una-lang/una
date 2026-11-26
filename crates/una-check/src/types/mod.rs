//! Types (M4): kinds and written types (U2002, U2003), bidirectional checking with unification
//! (U2001), calls and constructors (U2004 to U2006, U2009), `when` (U2007, U2008), statements
//! (U2011), the positions of `result`, `it` and `error` (U2012, U2013), the refinement language
//! (U2014), predicates (U2015) and lambdas (U2010), following `spec/03`, `spec/04`, `spec/06`,
//! `spec/10` and ADR 0024.
//!
//! Capabilities, effects and failures (M5): capability contexts, sufficiency, inference of the
//! headers of private functions, handlers, failures, one effect per line and the second-class
//! rules (U3001 to U3015, U7003), following `spec/05`, `spec/06`, `spec/10 §5, §6, §9` and
//! ADR 0028.
//!
//! The checker also records the proof obligations that refinement subsumption and the other
//! rules of `spec/12 §3` create. They stay pending until the verifier (M8) discharges them.

mod authority;
mod between;
mod bit_set;
mod boundary;
mod call;
mod check;
mod collections;
mod complex;
mod conditions;
mod constants;
mod crossing;
mod dead;
mod decimal;
mod dense;
mod edit_labels;
mod effects;
mod elab;
mod env;
mod expr;
mod facts;
mod fixed;
mod float;
mod flow;
mod generic;
mod hash;
mod hover;
mod index;
mod label;
mod locals;
mod lock;
mod map;
mod measured;
mod memory;
mod priority;
mod records;
mod seq;
mod server;
mod set;
mod shared;
#[cfg(test)]
mod sharing_tests;
mod stmt;
mod summary;
mod switch;
mod timing;
mod ty;
mod unify;
mod units;
mod via;
mod within;

pub use between::{Between, between_summaries};
pub use hover::{Hover, HoverStats, HoverType};
pub use index::Index;
pub use timing::{TimingEdge, TimingGraph, TimingNode, TimingPlace, TimingTarget};
use una_diag::{
    Code, Diagnostic, Edit, Escape, Location, ObligationKind, Related, Repair, Severity,
};
use una_project::Edition;
use una_syntax::ast::File;
use una_syntax::{FileId, Interner, KeywordSet, SourceDb, Span, lex_in, parse};

use crate::names::PackageInput;
use env::{Env, Source};

/// The prelude (`spec/14 §1, §3`) as Una source.
const PRELUDE: &str = include_str!("../std/prelude.una");
/// The declarations that edition 0.2 adds to the prelude (`spec/17 §8`), appended to it.
const PRELUDE_0_2: &str = include_str!("../std/prelude_0_2.una");
/// The declarations that edition 0.3 adds to the prelude: the functions of `F64` (`spec/29`,
/// ADR 0114 rule 3.9), appended after those of edition 0.2.
const PRELUDE_0_3: &str = include_str!("../std/prelude_0_3.una");
/// The declarations that edition 0.5 adds to the prelude: `Rounding` and the functions of
/// `Decimal` (`spec/29 §16`), the constructors of dense values and the functions their
/// methods resolve to (`spec/29 §17`, ADR 0120), then the functions of `F32` and the finite
/// kinds (ADR 0120 rules 5 and 11), appended after those of edition 0.3. ADR 0139 rule 6 moves
/// them from edition 0.4.
const PRELUDE_0_4: &str = include_str!("../std/prelude_0_4.una");

/// The declarations of `HttpServer` that edition 0.4 adds to `std.host`, which is the prelude's
/// host part (ADR 0123 rules 0 and 1, `spec/31 §1`), appended last.
/// Only an import from `std.host` brings their names into scope (`spec/10 §11` rule 10).
const HOST_0_4: &str = include_str!("../std/host_0_4.una");

/// The declaration of `HttpError` in `PRELUDE_0_2`, and the one edition 0.4 replaces it with,
/// which adds `Interrupted` (ADR 0116 rule 12) and then `RateLimited` (ADR 0117 rule 8) last,
/// so the earlier variants keep their indices. The text occurs exactly once in `PRELUDE_0_2` (a
/// test checks it).
const HTTP_ERROR_0_4: (&str, &str) = (
    "| Tls | Protocol\n",
    "| Tls | Protocol | Interrupted | RateLimited\n",
);

/// The rewrites of the prelude in edition 0.2, each a text of `PRELUDE` and its replacement:
/// `map`, `filter`, `first_where` and `fold` take effectful and failing functions (ADR 0062,
/// `spec/14 §1`), and `min`, `max`, `contains` and `sum` are generic over constrained types
/// (ADR 0066, `spec/20 §6`). Each text occurs exactly once in `PRELUDE` (a test checks it), so
/// an edit of the prelude can never turn a rewrite off silently.
const OF: [(&str, &str); 8] = [
    (
        "pub min(a: Int, b: Int) -> Int",
        "pub min[T: Ord](a: T, b: T) -> T",
    ),
    (
        "pub max(a: Int, b: Int) -> Int",
        "pub max[T: Ord](a: T, b: T) -> T",
    ),
    (
        "pub contains[T](xs: List[T], item: T) -> Bool",
        "pub contains[T: Eq](xs: List[T], item: T) -> Bool",
    ),
    (
        "pub sum(xs: List[Int]) -> Int = 0",
        "pub sum[T: Integer](xs: List[T]) -> Int = 0",
    ),
    (
        "  ensures result.len == xs.len\n=\n  []",
        "  ensures result.len == xs.len\n  uses of each\n  fails of each\n=\n  []",
    ),
    (
        "  ensures result.len <= xs.len\n=\n  xs",
        "  ensures result.len <= xs.len\n  uses of keep\n  fails of keep\n=\n  xs",
    ),
    (
        "keep: fn(T) -> Bool) -> Option[T] = None",
        "keep: fn(T) -> Bool) -> Option[T]\n  uses of keep\n  fails of keep\n=\n  None",
    ),
    (
        "step: fn(U, T) -> U) -> U = initial",
        "step: fn(U, T) -> U) -> U\n  uses of step\n  fails of step\n=\n  initial",
    ),
];

/// The prelude of edition 0.2: `PRELUDE` with the rewrites of `OF`.
fn of_prelude() -> String {
    OF.iter().fold(PRELUDE.to_owned(), |text, (from, to)| {
        text.replace(from, to)
    })
}

/// `std.testing` (`spec/14 §4`) as Una source.
const TESTING: &str = include_str!("../std/testing.una");

/// What edition 0.4 adds to `std.testing`: `elapse` (ADR 0116 rule 19), whose duration is in
/// `Millisecond` of `std.time`, and `test_request` and `test_exchange` (ADR 0123 rule 21),
/// over the server's kinds of `std.host`.
const TESTING_0_4: &str = include_str!("../std/testing_0_4.una");

/// The line of `std.testing` after which edition 0.4 imports `Millisecond` from `std.time` and
/// the server's kinds from `std.host`.
const TESTING_MODULE_LINE: &str = "module std.testing\n";

/// `std.testing` in `edition`: in edition 0.4, it imports `Millisecond` and the server's kinds,
/// and adds `elapse`, `test_request` and `test_exchange`.
fn testing_source(edition: Edition) -> String {
    if edition >= Edition::V0_4 {
        let imported = TESTING.replacen(
            TESTING_MODULE_LINE,
            "module std.testing\n\nimport std.host (Exchange, Method, Request)\nimport std.time (Millisecond)\n",
            1,
        );
        format!("{imported}{TESTING_0_4}")
    } else {
        TESTING.to_owned()
    }
}

/// `std.time` (`spec/26 §2`), edition 0.4, as Una source.
const TIME: &str = include_str!("../std/time.una");

/// `std.time` in edition 0.5: its constants `millisecond`, `second` and `minute` are declared
/// in Una (ADR 0126 rule 12), not built in.
const TIME_0_5: &str = concat!(
    include_str!("../std/time.una"),
    include_str!("../std/time_0_5.una")
);

/// `std.drive` (`spec/26 §2`), edition 0.4, as Una source.
const DRIVE: &str = include_str!("../std/drive.una");

/// `std.collections` (ADR 0119) as Una source: its kind `Ranked` and its functions. The checker
/// loads it only for a package of edition 0.4 that imports it.
const COLLECTIONS: &str = include_str!("../std/collections.una");

/// The built-in method `update` of maps in edition 0.4 (ADR 0119 rule 18) as Una source, in a
/// module that no scope holds: method syntax on a map reaches it, its name never does.
const MAP_0_4: &str = include_str!("../std/map_0_4.una");

/// `std.limit` (ADR 0117 rule 9, `spec/27 §10`) as Una source: the capability `Limit`, its
/// failure kind and the handler `limiter`, which the runtime provides. The checker loads it only
/// for a package of edition 0.4 that imports it.
const LIMIT: &str = include_str!("../std/limit.una");

/// The standard modules as the checker reads them in one edition: the prelude's source, with
/// the declarations edition 0.2 rewrites and adds, `std.testing`'s source, and the keyword
/// set the prelude is lexed with (`spec/14`). Read-only: the checker loads exactly this text
/// (#648, the standard library reference).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardSource {
    /// The prelude's source in this edition.
    pub prelude: String,
    /// `std.testing`'s source in this edition: edition 0.4 adds `elapse` (ADR 0116 rule 19).
    pub testing: String,
    /// The keywords the prelude and `std.testing` are lexed with.
    pub keywords: KeywordSet,
    /// The standard modules of edition 0.4 after `std.testing`, as `(path, source)`:
    /// `std.time`, then `std.drive`, which imports it (ADR 0116). Empty before 0.4.
    pub later: Vec<(&'static str, &'static str)>,
}

/// The built-in methods and properties that the checker resolves by the type of their
/// receiver, as `(receiver, name)`: `.len` on `Text`, `List`, `Map` (`spec/14 §2`) and the
/// collections, the methods of maps (`spec/21 §2`), of `Stack` and of `Queue` (`spec/28 §5`), of
/// sets (`spec/28 §6`), of hash maps and hash sets (`spec/28 §7`), of priority queues
/// (`spec/28 §8`) and of bit sets with their `.size` (`spec/28 §9`), the conversions
/// (`spec/18 §4`), and the wrapping, saturating and bit operations of the fixed-width integers
/// (`spec/18 §5`). The receivers are `"len"`, `"Map"`, `"Stack"`, `"Queue"`, `"Set"`,
/// `"HashMap"`, `"HashSet"`, `"PriorityQueue"`, `"BitSet"`, `"conversion"` and `"fixed"`.
/// Read-only: it is built from the tables the checker resolves them with, for the standard
/// library reference (#690).
pub fn builtin_methods() -> Vec<(&'static str, String)> {
    let mut methods = vec![("len", "len".to_owned())];
    methods.extend(
        map::MAP_METHODS
            .iter()
            .map(|(name, ..)| ("Map", (*name).to_owned())),
    );
    // Edition 0.4: `update`, which method syntax on a map reaches (ADR 0119 rule 18).
    methods.push(("Map", "update".to_owned()));
    // Edition 0.4: the methods of `Stack` and `Queue` (`spec/28 §5`).
    methods.extend(
        seq::STACK_METHODS
            .iter()
            .map(|(name, ..)| ("Stack", (*name).to_owned())),
    );
    methods.extend(
        seq::QUEUE_METHODS
            .iter()
            .map(|(name, ..)| ("Queue", (*name).to_owned())),
    );
    // Edition 0.4: the methods of `Set` (ADR 0119 rule 7).
    methods.extend(
        set::SET_METHODS
            .iter()
            .map(|(name, ..)| ("Set", (*name).to_owned())),
    );
    // Edition 0.4: the methods of `HashMap`, with `update`, and of `HashSet` (ADR 0119 rule
    // 12).
    methods.extend(
        hash::HASH_MAP_METHODS
            .iter()
            .map(|(name, ..)| ("HashMap", (*name).to_owned())),
    );
    methods.push(("HashMap", "update".to_owned()));
    methods.extend(
        hash::HASH_SET_METHODS
            .iter()
            .map(|(name, ..)| ("HashSet", (*name).to_owned())),
    );
    // Edition 0.4: the methods of `PriorityQueue` (ADR 0119 rule 13).
    methods.extend(
        priority::PRIORITY_METHODS
            .iter()
            .map(|(name, ..)| ("PriorityQueue", (*name).to_owned())),
    );
    // Edition 0.4: `.size` and the methods of `BitSet` (ADR 0119 rule 14).
    methods.push(("BitSet", "size".to_owned()));
    methods.extend(
        bit_set::BIT_SET_METHODS
            .iter()
            .map(|(name, ..)| ("BitSet", (*name).to_owned())),
    );
    methods.extend(una_core::IntType::ALL.iter().map(|t| {
        (
            "conversion",
            format!("to_{}", t.as_str().to_ascii_lowercase()),
        )
    }));
    methods.push(("conversion", "to_int".to_owned()));
    methods.extend(
        una_core::FixedOp::ALL
            .iter()
            .map(|op| ("fixed", op.as_str().to_owned())),
    );
    methods
}

/// The standard modules' source in `edition`, as [`Standard::load`] parses it.
pub fn standard_source(edition: Edition) -> StandardSource {
    let (prelude, keywords) = if edition >= Edition::V0_5 {
        (
            format!(
                "{}\n{}{PRELUDE_0_3}{PRELUDE_0_4}{HOST_0_4}",
                of_prelude(),
                PRELUDE_0_2.replace(HTTP_ERROR_0_4.0, HTTP_ERROR_0_4.1)
            ),
            KeywordSet::V0_5,
        )
    } else if edition >= Edition::V0_4 {
        (
            format!(
                "{}\n{}{PRELUDE_0_3}{HOST_0_4}",
                of_prelude(),
                PRELUDE_0_2.replace(HTTP_ERROR_0_4.0, HTTP_ERROR_0_4.1)
            ),
            KeywordSet::V0_4,
        )
    } else if edition >= Edition::V0_3 {
        (
            format!("{}\n{PRELUDE_0_2}{PRELUDE_0_3}", of_prelude()),
            KeywordSet::V0_3,
        )
    } else if edition >= Edition::V0_2 {
        (format!("{}\n{PRELUDE_0_2}", of_prelude()), KeywordSet::V0_2)
    } else {
        (PRELUDE.to_owned(), KeywordSet::V0_1)
    };
    let later = if edition >= Edition::V0_5 {
        vec![("std.time", TIME_0_5), ("std.drive", DRIVE)]
    } else if edition >= Edition::V0_4 {
        vec![("std.time", TIME), ("std.drive", DRIVE)]
    } else {
        Vec::new()
    };
    StandardSource {
        prelude,
        testing: testing_source(edition),
        keywords,
        later,
    }
}

/// A proof obligation that the type checker found (`spec/12 §3`). It stays pending until the
/// verifier generates it over Core IR and discharges it (M8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingObligation {
    /// The kind of obligation.
    pub kind: ObligationKind,
    /// The primary span of `spec/12 §3`: the value, the call, the returned expression, the
    /// division or the `when` keyword.
    pub location: Location,
    /// The declared type the obligation comes from, when it is written in the package.
    pub origin: Option<Location>,
}

/// A capability that a header uses: `uses @cap: capability[ops]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderUse {
    /// The reference, without `@`.
    pub cap: String,
    /// The capability's name.
    pub capability: String,
    /// Whether the capability is the host's, declared by the prelude (`spec/14 §3`), not one
    /// of the package's of the same name.
    pub host: bool,
    /// The operations, in the capability's declaration order.
    pub ops: Vec<String>,
    /// The label each operation gives through the binding, in the order of `ops`, as written
    /// after `gives` (ADR 0094 rule 1); an operation absent here gives `public`.
    pub gives: Vec<(String, String)>,
}

/// The `uses` of a handler of the package, with the labels its operations give (ADR 0094
/// rule 1), which the build report lists under H5 (ADR 0094 rule 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerHeader {
    /// The module path, for example `events.checkin`.
    pub module: String,
    /// The handler's name.
    pub name: String,
    /// Its `uses`, in order.
    pub uses: Vec<HeaderUse>,
}

/// The authority and failure contract of a function of the package after inference
/// (`spec/04 §5`), which `una contract` prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The module path, for example `events.checkin`.
    pub module: String,
    /// The function's name.
    pub name: String,
    /// Its `uses`, declared or inferred, in order.
    pub uses: Vec<HeaderUse>,
    /// Whether its `uses` are inferred: a private function without `uses` clauses.
    pub uses_inferred: bool,
    /// The name of its error kind, declared or inferred, when it can fail.
    pub fails: Option<String>,
    /// Whether its `fails` is inferred: a private function without a `fails` clause.
    pub fails_inferred: bool,
    /// The parameters of its `uses of` clauses, written or inferred (ADR 0062).
    pub uses_of: Vec<String>,
    /// The parameter of its `fails of` clause, written or inferred.
    pub fails_of: Option<String>,
    /// Its constrained type parameters, in order: the parameter, the constraint, and whether
    /// it is inferred (`spec/20 §8`).
    pub constraints: Vec<(String, String, bool)>,
    /// Its result summary: each position of its declared result whose summary is not empty,
    /// written as `spec/13 §5` writes it, with the type parameters there (ADR 0067).
    pub result_labels: Vec<(String, Vec<String>)>,
    /// Whether it has type parameters.
    pub generic: bool,
    /// Each parameter that an `of` clause names whose calls depend on decisions on type
    /// parameters, with those type parameters: callers run their argument under them
    /// (ADR 0067 amendment).
    pub of_raises: Vec<(String, Vec<String>)>,
}

/// The outcome of [`check_types`].
#[derive(Clone, Debug)]
pub struct TypeCheck {
    /// The type diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// The pending proof obligations, by position, then kind.
    pub obligations: Vec<PendingObligation>,
    /// Where each `let` whose right-hand side is in the refinement language binds its name.
    /// Such a binding has the selfified type `{v: B | v == e}` (`spec/03 §7`).
    pub selfified: Vec<Location>,
    /// The lambdas given to `of` parameters (ADR 0062), by span: whether the body performs
    /// effects and whether it can fail. Their function types in Core carry neither, since they
    /// are their caller's; `una core` exports them (#443).
    pub of_lambdas: Vec<(Span, bool, bool)>,
    /// The package's file-state label (ADR 0085 decision 1): what decides its `Files.write`
    /// calls, which labels the failure of every `Files` operation; `public` when it writes no
    /// file. `una core` exports it (#467), and the formal track's re-check checks every write
    /// against it.
    pub file_state: una_core::Label,
    /// The headers of the functions of the modules checked, by module, then declaration.
    pub headers: Vec<Header>,
    /// The `uses` of the handlers of the modules checked, by module, then declaration.
    pub handler_headers: Vec<HandlerHeader>,
    /// The Core IR of the declarations without diagnostics (`spec/09`).
    pub program: una_core::Program,
    /// What the checker resolved, by span, for the language server (ADR 0063).
    pub index: Index,
    /// The sites whose deadlines turn timing into values, in source order, with their timing
    /// labels: each `for ever within` (ADR 0116 rule 21), for the report's `timing` list; and
    /// each `task switch`, a check point whose count data above `public`, at most `internal`,
    /// decide in a program that starts tasks (`spec/27 §8` rule 5, #1708).
    pub timing: Vec<TimingSite>,
    /// `L_lim` (ADR 0117 IF4): the timing label of the package's `Http` requests, which a grant
    /// with limits needs at most `internal`; `public` without such a request. `una build`
    /// embeds it, and `una run` checks a limited `--grant` against it (`spec/27 §9` rule 5).
    pub limit_label: una_core::Label,
    /// The units of measure of the state types of the package's `[drive]` module, which Core
    /// IR erases, for the shape check of `una run` and `una build` (`spec/26 §2` rule 2).
    pub drive_units: Option<DriveUnits>,
    /// The graph of ADR 0116 P1 and the construct trees of P3 as checking recorded them, for
    /// the completeness test of P1.
    pub timing_graph: TimingGraph,
    /// The constructs of the edition ledger that elaboration lowers into other Core IR, each
    /// with the span of its site, in span order: the report counts them with what it reads in
    /// Core IR (ADR 0121 rule 8).
    pub preview_sites: Vec<(una_project::ledger::Construct, Span)>,
}

/// The units of measure at each position of the state types of a `[drive]` module: the
/// result of `init` and the state parameters of `step` and `on_stop`, each as the list of its
/// measured positions with the normal form of their units (ADR 0104 rule 2). The host hands
/// the state on unchanged, so these MUST be equal (`spec/26 §2` rule 2); the checker reports
/// a difference as U2028, and `una run` and `una build` refuse one as a usage error.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriveUnits {
    /// The result of `init`.
    pub init: String,
    /// The state parameter of `step`, when the module has one.
    pub step: Option<String>,
    /// The state parameter of `on_stop`, when it takes one parameter.
    pub on_stop: Option<String>,
}

/// A site with a deadline and its timing label (ADR 0116 IF5, rule 21).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingSite {
    /// The construct, as `for ever within`.
    pub construct: &'static str,
    /// The qualified name of the function that holds it, or empty outside a function.
    pub function: String,
    /// Where it is.
    pub location: Location,
    /// Its timing label.
    pub label: una_core::Label,
}

impl Default for TypeCheck {
    /// An empty outcome, whose file-state label is that of a package that writes no file.
    fn default() -> Self {
        TypeCheck {
            diagnostics: Vec::new(),
            obligations: Vec::new(),
            selfified: Vec::new(),
            of_lambdas: Vec::new(),
            file_state: una_core::Label::public(),
            headers: Vec::new(),
            handler_headers: Vec::new(),
            program: una_core::Program::default(),
            index: Index::default(),
            timing: Vec::new(),
            limit_label: una_core::Label::public(),
            timing_graph: TimingGraph::default(),
            drive_units: None,
            preview_sites: Vec::new(),
        }
    }
}

/// A declaration that the checker rejected, with its Core IR (ADR 0091): its type pass reported
/// a diagnostic and its elaboration is complete. `una core --with-rejected` exports the ones
/// whose every diagnostic is in the `security` domain, and nothing else reads them.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    /// The path of its module.
    pub module: String,
    /// The span of the declaration, which holds its diagnostics.
    pub span: Span,
    /// Its Core IR.
    pub decl: una_core::Decl,
}

/// Type-checks the modules of `package` that have no syntax errors.
pub fn check_types(package: &PackageInput<'_>) -> TypeCheck {
    check_types_and(package, false).0
}

/// [`check_types`], and the Core IR of each rejected declaration whose elaboration is complete
/// (ADR 0091), for `una core --with-rejected`. The [`TypeCheck`] is the one [`check_types`]
/// returns: the rejected declarations are elaborated apart, after it, from what the checks
/// recorded, and change nothing in it.
pub fn check_types_with_rejected(package: &PackageInput<'_>) -> (TypeCheck, Vec<Rejected>) {
    check_types_and(package, true)
}

fn check_types_and(package: &PackageInput<'_>, with_rejected: bool) -> (TypeCheck, Vec<Rejected>) {
    let mut report = Report::new(package.sources);
    let sources = package
        .modules
        .iter()
        .map(|module| Source {
            tree: module.tree,
            interner: package.interner,
            reports: !module.broken,
            mount: package
                .settings
                .mount_of(package.sources.file(module.id).path())
                .map(|mount| mount.local.as_str()),
        })
        .collect();
    let categories = package
        .settings
        .categories
        .iter()
        .map(|category| category.value.clone())
        .collect();
    // ADR 0119 rule 0: `std.collections` is in scope only for a package of edition 0.5 (ADR
    // 0139 rule 6) that imports it, so no other package's checks or Core IR change; `std.limit`
    // the same from edition 0.4 (ADR 0117 rule 9).
    let imports = |standard: &str, from: Edition| {
        package.settings.edition >= from
            && package.modules.iter().any(|module| {
                module.tree.imports.iter().any(|import| {
                    let path: Vec<&str> = import
                        .module
                        .segments
                        .iter()
                        .map(|segment| package.interner.resolve(segment.name))
                        .collect();
                    path.join(".") == standard
                })
            })
    };
    let imports_collections = imports(una_core::COLLECTIONS_MODULE, Edition::V0_5);
    let imports_limit = imports(una_core::LIMIT_MODULE, Edition::V0_4);
    // The optional modules are parsed only when this package takes them (#1894).
    let standard = Standard::load(package.settings.edition, imports_collections, imports_limit);
    let mut env = Env::build(
        sources,
        standard.source(&standard.prelude),
        standard.after_prelude(),
        standard.map_0_4.as_ref().map(|tree| standard.source(tree)),
        standard
            .collections
            .as_ref()
            .map(|tree| standard.source(tree)),
        standard.limit.as_ref().map(|tree| standard.source(tree)),
        categories,
        package.settings.edition,
        package.settings.profile,
        &mut report,
    );
    env.trusted_listed = package
        .settings
        .trusted
        .iter()
        .map(|entry| entry.value.clone())
        .collect();
    env.project_code = package.settings.format.code();
    env.manifest_file = package.settings.format.file();
    env.serve_driver = package
        .settings
        .serve
        .as_ref()
        .zip(package.manifest)
        .map(|(serve, manifest)| Span::new(manifest, serve.at.start, serve.at.end));
    env.drive_driver = package
        .settings
        .drive
        .as_ref()
        .zip(package.manifest)
        .map(|(drive, manifest)| Span::new(manifest, drive.at.start, drive.at.end));
    env.trusted_dependencies = env
        .modules
        .iter()
        .enumerate()
        .take(package.modules.len())
        .filter(|(_, module)| package.settings.dependency_trusted.contains(&module.path))
        .map(|(index, _)| index)
        .collect();
    // Edition 0.3: the boundaries of `una.toml`, checked against the package (ADR 0093).
    env.boundaries = boundary::resolve(
        &env,
        package.settings,
        package.manifest,
        package.modules.len(),
        &mut report,
    );
    if !env.boundaries.list.is_empty() {
        env.boundaries.any_policy = env
            .functions
            .iter()
            .any(|f| matches!(f.ast, env::FnAst::Policy(_)));
        env.boundaries.function_kinds = crossing::function_kinds(&env);
    }
    // Every package, for the timing label of a handler's parameters (#1436): a parameter that
    // holds a function runs code its type does not bound.
    env.holds_function_kinds = crossing::function_kinds(&env);
    // ADR 0123: the kinds that can hold a reply right (IF6), and the capabilities whose
    // handlers can perform an operation of `HttpServer` (U3033).
    env.holds_reply_kinds = server::reply_kinds(&env);
    env.serving_capabilities = server::serving_capabilities(&env);
    let vias = via::plain_vias(&env, &report);
    env.plain_vias = vias.plain;
    env.blocked_vias = vias.blocked;
    let facts = facts::KindFacts::new(&env);
    facts::kind_recursion(&env, &mut report);
    // Edition 0.5: functions named like a built-in method of a collection (ADR 0119 rules 0
    // and 18, moved to edition 0.5 by ADR 0139).
    if package.settings.edition >= Edition::V0_5 {
        collections::method_names(&env, &mut report);
    }
    authority::stored_effects(&env, &mut report);
    // Pass 1 collects what inference needs, and its diagnostics are dropped;
    // pass 2 checks with the inferred headers.
    // Pass 1 also records the result summaries of generic functions (ADR 0067); in edition
    // 0.2 they then reach their least fixpoint, below.
    let mut collected = authority::Collected::default();
    let mut edges = Vec::new();
    let withins;
    let clocks;
    {
        let mut scratch = Report::new(package.sources);
        let mut findings = Findings::default();
        for module in 0..package.modules.len() {
            if env.reports(module) {
                check::module(
                    &mut env,
                    &facts,
                    &mut scratch,
                    &mut findings,
                    &mut collected,
                    authority::Pass::Collect,
                    module,
                );
            }
        }
        edges.extend(
            findings
                .edges
                .iter()
                .map(|(caller, callee, _)| (*caller, *callee)),
        );
        withins = !findings.withins.is_empty();
        // ADR 0116 amendment of 2026-11-09 (#898): edition 0.4 records its readings of `Clock`.
        clocks = !findings.clock_sites.is_empty();
        env.opaque_plain = opaque_plain(&env, &findings.edges);
    }
    if package.settings.edition >= Edition::V0_2 {
        summary_fixpoint(&mut env, &facts, package.sources, &edges);
        // ADR 0075 rule 5: once the summaries are final, pass 1 runs once more over the
        // declarations, so the label variables of editable bindings count every callee's final
        // summary. Its diagnostics and what it collects are dropped. Should that change a
        // solution, the summaries that read it are brought to their fixpoint again, so none
        // keeps a stale label; solutions only grow, so this ends.
        // Each round that goes on raises at least one solution, which rises only through the
        // finitely many labels the package can form; a round past the number of variables and
        // two more would mean they do not rise, so it is an internal error, not a silent stop
        // that could keep a stale label.
        let mut rounds = 0;
        while env.edit_labels_seen {
            rounds += 1;
            // Each round that goes on raises some variable by at least one atom, or records a
            // variable's shape once; atoms: the four levels above `public`, the categories, and
            // the symbolic labels the solutions name (ADR 0075 rule 5).
            let atoms = 4
                + env.categories.len()
                + env
                    .edit_labels
                    .values()
                    .flat_map(|label| label.symbols.iter())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
            if rounds > env.edit_labels.len() * (atoms + 1) + 2 {
                let span = env
                    .edit_labels_changed_name
                    .as_ref()
                    .and_then(|name| env.edit_label_spans.get(name).copied())
                    .or_else(|| env.edit_label_spans.values().next().copied());
                if let Some(span) = span {
                    report.error(
                        Code::U8001,
                        span,
                        "internal error: the label variables of this editable binding did not reach a fixpoint (ADR 0075)".to_owned(),
                    );
                }
                break;
            }
            env.edit_labels_changed = false;
            let mut scratch = Report::new(package.sources);
            let mut findings = Findings::default();
            let mut collected = authority::Collected::default();
            for module in 0..package.modules.len() {
                if env.reports(module) {
                    check::module(
                        &mut env,
                        &facts,
                        &mut scratch,
                        &mut findings,
                        &mut collected,
                        authority::Pass::Collect,
                        module,
                    );
                }
            }
            if !env.edit_labels_changed {
                break;
            }
            summary_fixpoint(&mut env, &facts, package.sources, &edges);
        }
    }
    authority::infer(&mut env, &collected, &mut report);
    // Edition 0.3 (ADR 0100 rule 3): the functions whose inferred failure rests on code in
    // error, once the capabilities of their operations are inferred.
    let in_error = dead::in_error(&env, &collected);
    // Edition 0.3 (ADR 0092 rule 8, as amended): the locks of `[contracts]`, and the bindings
    // and operations of the locked headers that they do not grant.
    lock::resolve(&mut env, package.locks, &mut report);
    lock::drift(&env, &mut report);
    // Two labels need a pass of their own before pass 2 checks with them, and each can raise
    // the other, so they are probed together, pass 2 run with its diagnostics dropped, until
    // neither rises:
    // - ADR 0117 IF2: the timing label of each `within` needs the whole package's call graph;
    // - S-14 (ADR 0085): a package that writes files labels the failure of every `Files`
    //   operation with what decides its writes, so that a read checked before a write has the
    //   label too, and a failure so labeled can raise a program counter that a `within`'s
    //   timing label reads.
    // Labels only rise, and each round that goes on raises one by at least one atom, so a round
    // past that bound is an internal error, never a silent stop with a label too low. Both are
    // confirmed after pass 2 (`authority::within_timing`, `file_state`).
    // - the amendment of ADR 0116 of 2026-11-09 (#898, C5): the label `L_clk` of each reading
    //   of `Clock` needs the call graph too, and a labeled reading raises `τ⁰` of the bodies
    //   that compute from it, so of other readings and of `within`s.
    if withins || clocks || env.files_written {
        let atoms = 4 + env.categories.len() + 1;
        let mut rounds = 0usize;
        loop {
            rounds += 1;
            let mut scratch = Report::new(package.sources);
            let mut probe = Findings::default();
            let mut unused = authority::Collected::default();
            for module in 0..package.modules.len() {
                if env.reports(module) {
                    check::module(
                        &mut env,
                        &facts,
                        &mut scratch,
                        &mut probe,
                        &mut unused,
                        authority::Pass::Check,
                        module,
                    );
                }
            }
            let (labels, residue) = if withins {
                authority::within_labels(&env, &probe)
            } else {
                (std::collections::BTreeMap::new(), timing::Residue::none())
            };
            // The readings' labels, joined with what earlier rounds found, as for `within`.
            let clock_labels: std::collections::BTreeMap<Span, label::Label> = if clocks {
                let residue = if withins {
                    residue.clone()
                } else {
                    timing::residue_of(&env, &probe)
                };
                authority::clock_labels(&env, &probe, package.settings, &residue)
                    .into_iter()
                    .map(|(key, label)| match env.clock_timing.get(&key) {
                        Some(before) => (key, before.join(&label)),
                        None => (key, label),
                    })
                    .collect()
            } else {
                std::collections::BTreeMap::new()
            };
            let clocks_stable = clock_labels == env.clock_timing;
            env.clock_timing = clock_labels;
            // ADR 0116 P9: what the messages of pass 2 name when `L_res` raises a label.
            env.residue = residue;
            let state = if env.files_written {
                env.file_state.join(&file_state(&env).0)
            } else {
                env.file_state.clone()
            };
            env.file_writes.clear();
            env.file_calls.clear();
            env.file_escaped.clear();
            let joined: std::collections::BTreeMap<Span, label::Label> = labels
                .into_iter()
                .map(|(key, label)| match env.within_timing.get(&key) {
                    Some(before) => (key, before.join(&label)),
                    None => (key, label),
                })
                .collect();
            let stable = joined == env.within_timing && state == env.file_state && clocks_stable;
            env.within_timing = joined;
            env.file_state = state;
            // Without a `within`, one probe gives the file-state label's least fixpoint, as
            // before (ADR 0085): what the label raises in pass 2 is below it.
            if stable || !(withins || clocks) {
                break;
            }
            if rounds > (env.within_timing.len() + env.clock_timing.len() + 1) * atoms + 2 {
                if let Some(span) = probe
                    .withins
                    .first()
                    .map(|site| site.span)
                    .or_else(|| probe.clock_sites.first().map(|site| site.span))
                {
                    report.error(
                        Code::U8001,
                        span,
                        "internal error: the timing labels of `within`, `first of` and `Clock` readings and the file-state label did not reach a fixpoint (ADR 0117 IF2, IF3, ADR 0116, ADR 0085)".to_owned(),
                    );
                }
                break;
            }
        }
    }
    let mut findings = Findings::default();
    let mut unused = authority::Collected::default();
    for module in 0..package.modules.len() {
        if env.reports(module) {
            check::module(
                &mut env,
                &facts,
                &mut report,
                &mut findings,
                &mut unused,
                authority::Pass::Check,
                module,
            );
        }
    }
    // The label pass 2 found is never above the probe's: an internal error otherwise, never a
    // read whose failure the label would have missed.
    let (found, raised) = file_state(&env);
    if !found.flows_to(&env.file_state)
        && let Some(span) = raised
    {
        report.error(
            Code::U8001,
            span,
            format!(
                "internal error: this write raises the file-state label to `{found}`, above the `{}` that the checks used (ADR 0085)",
                env.file_state
            ),
        );
    }
    // The standard modules are checked silently, for the records that their
    // contracts are elaborated from.
    {
        let mut scratch = Report::new(&standard.sources);
        let mut standard_findings = Findings::default();
        for module in env.prelude..env.modules.len() {
            check::module(
                &mut env,
                &facts,
                &mut scratch,
                &mut standard_findings,
                &mut unused,
                authority::Pass::Check,
                module,
            );
        }
        // ADR 0126 rule 12: the constants of the standard modules (`std.time`'s).
        let ids: Vec<usize> = (0..env.constants.len())
            .filter(|id| {
                env.constants
                    .get(*id)
                    .is_some_and(|c| c.module >= env.prelude)
            })
            .collect();
        findings.standard_constants = constants::evaluate(
            &env,
            &standard_findings.records,
            &ids,
            &standard_findings.refused_constants,
            None,
            &mut scratch,
            false,
        );
        findings.standard_records = standard_findings.records;
    }
    // ADR 0126 rules 4 and 5: the package's constants, evaluated once, in dependency order.
    {
        let ids: Vec<usize> = (0..env.constants.len())
            .filter(|id| {
                env.constants
                    .get(*id)
                    .is_some_and(|c| c.module < env.prelude && env.reports(c.module))
            })
            .collect();
        let refused = std::mem::take(&mut findings.refused_constants);
        findings.constants = constants::evaluate(
            &env,
            &findings.records,
            &ids,
            &refused,
            Some(&findings.standard_constants),
            &mut report,
            true,
        );
        findings.refused_constants = refused;
        constants::substitution_budget(&env, &findings.records, &findings.constants, &mut report);
    }
    authority::testing_installs(&findings, &mut report, env.edition);
    // Edition 0.3: no release is reachable from a call into a boundary (ADR 0093 rule 8).
    crossing::releases(&env, &findings, &mut report);
    facts::predicate_recursion(&env, &findings, &mut report);
    if package.settings.edition >= Edition::V0_2 {
        facts::generic_constraints(&mut env, &facts, &findings, &mut report);
    } else {
        facts::generic_equality(&env, &facts, &findings, &mut report);
    }
    authority::grants(&env, package.settings, package.run, &mut report);
    authority::drive_failures(&env, package.settings, &mut report);
    authority::drive_state_labels(&env, package.settings, &mut report);
    authority::drive_state_units(&env, package.settings, &mut report);
    let drive_units = authority::drive_units(&env, package.settings);
    authority::serve_functions(&env, package.settings, &mut report);
    // `L_res` of the checked package, computed once for every site that joins it (ADR 0116 P9).
    let residue = timing::residue_of(&env, &findings);
    authority::drive_timing(&env, &findings, package.settings, &residue, &mut report);
    authority::serve_within(&env, &findings, package.settings, &residue, &mut report);
    let limit_label = authority::limit_timing(
        &env,
        &findings,
        package.settings,
        &residue,
        (package.sources, &standard.sources),
        &mut report,
    );
    authority::limiter_timing(&env, &findings, &residue, &mut report);
    let mut timing_sites = authority::deadline_timing(&env, &findings, &residue, &mut report);
    // ADR 0116 amendment of 2026-11-09 (#898, C4, C5): the readings of `Clock`.
    timing_sites.extend(authority::clock_timing(
        &env,
        &findings,
        package.settings,
        &residue,
        &mut report,
    ));
    timing_sites.extend(authority::within_timing(
        &env,
        &findings,
        &residue,
        &mut report,
    ));
    // `spec/07 §5` rule 7: the timing sites a `via` function reaches (#1673).
    authority::via_timing(&env, &findings, &mut report);
    // ADR 0116 amendment of 2026-11-09 (#898, C7): the readings of `Clock` a `via` reaches.
    authority::via_clock_timing(&env, &findings, &mut report);
    // ADR 0123 IF6: the auto-answers of the reply rights each `start` moves.
    server::start_rights(&env, &findings, &mut report);
    // `spec/27 §8` rule 5: the check points that move the schedule of started tasks (#1684);
    // those that data at most `internal` decide go to the report (#1708).
    timing_sites.extend(switch::switch_points(
        &env,
        &findings,
        package.settings.serve.is_some(),
        &mut report,
    ));
    timing_sites.sort_by_key(|(_, _, span, _)| (span.file, span.start, span.end));
    authority::budget(&env, &findings, package.settings, &mut report);
    // Edition 0.3 (ADR 0100): last, once every other error of the package is known.
    dead::judge(
        &mut findings,
        &in_error,
        package.earlier_errors,
        &mut report,
    );
    // Pending obligations by position, then kind.
    findings
        .obligations
        .sort_by_key(|(kind, span, _)| (span.file, span.start, span.end, *kind));
    // A span may be charged an `effect_condition` by more than one route, such as a call's
    // callee and a value it is passed (#615): one pending entry each.
    findings
        .obligations
        .dedup_by(|a, b| a.0 == ObligationKind::EffectCondition && a == b);
    let headers = headers(&env);
    let handler_headers = handler_headers(&env);
    // S-39: the calls and operations under a raised program counter whose running can append
    // events of its own are `quiet` too.
    flow::close_quiet(&env, &mut findings);
    let (program, gaps) = elab::program(&env, &findings, package.sources, &standard.sources);
    // ADR 0103: the pending `refinement` obligations of the flows into refined positions,
    // decided by the rule tier 1 uses (#615).
    let kinds = una_core::flow::Kinds::new(&program);
    let flows = elab::refined_flows(&env, &findings, package.sources, &standard.sources);
    let before = findings.obligations.len();
    for (span, target, source) in flows {
        if kinds.pending(&source, &target) {
            findings
                .obligations
                .push((ObligationKind::Refinement, span, None));
        }
    }
    if findings.obligations.len() != before {
        findings
            .obligations
            .sort_by_key(|(kind, span, _)| (span.file, span.start, span.end, *kind));
    }
    // ADR 0038 D1: a component of a checker type that Core IR lost is an internal error, never
    // a silent pass (`spec/10 §2`). D-144: switched to B, ADR 0045: the
    // check runs in every build, not only in debug builds, and reports at the declaration.
    for (span, what) in gaps.lost {
        report.error(
            Code::U8001,
            span,
            format!(
                "internal error: the Core IR of this declaration lost {what} of its checked types"
            ),
        );
    }
    // #1050: a declaration that type-checked but has no Core IR would get no obligations, so
    // its contracts and its arithmetic checks would never be verified. Unless an error already
    // rejects the package, as one of its own diagnostics does, that is an internal error, never
    // an accepted package. The same holds for a contract of the standard library, without which
    // a call into it would be verified without its `requires`.
    let rejected_already = package.earlier_errors
        || report
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error);
    if !rejected_already {
        for (span, what) in gaps.missing {
            report.error(
                Code::U8001,
                span,
                format!(
                    "internal error: this {what} type-checked but did not elaborate to Core IR, so none of its obligations could be verified; the package is rejected"
                ),
            );
        }
        // A signature whose type is unknown in Core IR, in a package with no error, would be
        // a position checked against the error type, which accepts everything: an internal
        // error, never a silent pass (#1632 review, H1).
        let paths: std::collections::BTreeSet<&str> = (0..env.prelude)
            .filter_map(|m| env.modules.get(m).map(|module| module.path.as_str()))
            .collect();
        for module in program
            .modules
            .iter()
            .filter(|m| paths.contains(m.path.as_str()))
        {
            for decl in &module.decls {
                if let una_core::Decl::Fun(fun) = decl
                    && (fun.params.iter().any(|(_, ty)| unknown_in(ty)) || unknown_in(&fun.result))
                {
                    report.error(
                        Code::U8001,
                        fun.span,
                        "internal error: a type of this function's signature is unknown in Core IR, though no error was reported; the package is rejected".to_owned(),
                    );
                }
            }
        }
        for (span, what) in gaps.standard {
            report.push(Diagnostic::error(
                Code::U8001,
                format!(
                    "internal error: this {what} of the standard library did not elaborate to Core IR, so calls into it could not be verified; the package is rejected"
                ),
                standard.sources.location(span),
            ));
        }
    }
    let index = index::build(&env, &findings);
    // ADR 0091 decision 5: elaborated only on request, after the program, and read by nothing
    // but `una core --with-rejected`.
    let rejected = if with_rejected {
        elab::rejected(&env, &findings, package.sources, &standard.sources)
    } else {
        Vec::new()
    };
    let timing_graph = timing::recorded_graph(&env, &findings);
    let check = TypeCheck {
        preview_sites: findings.preview_sites.iter().copied().collect(),
        limit_label,
        drive_units,
        timing_graph,
        timing: timing_sites
            .into_iter()
            .map(|(construct, function, span, label)| TimingSite {
                construct,
                function,
                location: report.location(span),
                label,
            })
            .collect(),
        index,
        headers,
        handler_headers,
        program,
        obligations: findings
            .obligations
            .iter()
            .map(|(kind, span, origin)| PendingObligation {
                kind: *kind,
                location: report.location(*span),
                origin: origin.map(|origin| report.location(origin)),
            })
            .collect(),
        selfified: findings
            .selfified
            .iter()
            .map(|span| report.location(*span))
            .collect(),
        of_lambdas: findings.of_lambdas.clone(),
        file_state: env.file_state.clone(),
        diagnostics: report.diagnostics,
    };
    (check, rejected)
}

/// The headers of the functions of the modules that report, after inference.
/// A `uses` clause of a header as the outcome lists it, with the labels its operations give.
fn header_use(env: &Env<'_>, need: &env::Need) -> HeaderUse {
    let capability = need.usage.capability.and_then(|c| env.capabilities.get(c));
    HeaderUse {
        cap: need.usage.name.clone(),
        capability: capability.map_or_else(String::new, |c| c.name.to_owned()),
        host: capability.is_some_and(|c| c.module == env.prelude),
        ops: need.usage.ops.clone(),
        gives: need
            .usage
            .gives
            .iter()
            .map(|(op, label)| (op.clone(), label.to_string()))
            .collect(),
    }
}

/// The `uses` of the handlers of the modules that report, in declaration order.
fn handler_headers(env: &Env<'_>) -> Vec<HandlerHeader> {
    env.handlers
        .iter()
        .filter(|h| env.reports(h.module))
        .map(|h| HandlerHeader {
            module: env
                .modules
                .get(h.module)
                .map(|m| m.path.clone())
                .unwrap_or_default(),
            name: env.text(h.module, h.ast.name.name).to_owned(),
            uses: h.uses.iter().map(|need| header_use(env, need)).collect(),
        })
        .collect()
}

fn headers(env: &Env<'_>) -> Vec<Header> {
    env.functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.kind == env::FnKind::Function && env.reports(f.module))
        .map(|(id, f)| Header {
            generic: !f.type_params.is_empty(),
            of_raises: env
                .of_raises
                .range((id, 0)..=(id, usize::MAX))
                .filter(|(_, params)| !params.is_empty())
                .filter_map(|((_, index), params)| {
                    let param = f.params.get(*index)?;
                    Some((
                        param.name.to_owned(),
                        params
                            .iter()
                            .filter_map(|p| f.type_params.get(*p))
                            .map(|p| (*p).to_owned())
                            .collect(),
                    ))
                })
                .collect(),
            result_labels: env
                .summaries
                .get(&id)
                .map(|summary| {
                    summary
                        .iter()
                        .filter(|(_, params)| !params.is_empty())
                        .map(|(path, params)| {
                            (
                                env::position_text(path),
                                params
                                    .iter()
                                    .filter_map(|index| f.type_params.get(*index))
                                    .map(|param| (*param).to_owned())
                                    .collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            module: env
                .modules
                .get(f.module)
                .map(|m| m.path.clone())
                .unwrap_or_default(),
            name: f.name.to_owned(),
            constraints: f
                .type_params
                .iter()
                .zip(&f.constraints)
                .zip(&f.constraints_written)
                .filter(|((_, constraint), _)| **constraint != env::Constraint::None)
                .map(|((param, constraint), written)| {
                    ((*param).to_owned(), constraint.name().to_owned(), !written)
                })
                .collect(),
            uses: f.uses.iter().map(|need| header_use(env, need)).collect(),
            uses_inferred: f.infer_uses,
            fails: match &f.fails {
                Some(ty::Ty::Kind(id, _)) => Some(env.kind_name(*id).to_owned()),
                _ => None,
            },
            fails_inferred: f.infer_fails,
            uses_of: f
                .uses_of
                .iter()
                .filter_map(|index| f.params.get(*index))
                .map(|param| param.name.to_owned())
                .collect(),
            fails_of: f
                .fails_of
                .and_then(|index| f.params.get(index))
                .map(|param| param.name.to_owned()),
        })
        .collect()
}

/// What checking the bodies finds besides diagnostics.
#[derive(Default)]
pub(crate) struct Findings {
    /// The module constants of the package evaluated at check time, edition 0.5 (ADR 0126).
    pub(crate) constants: constants::Evaluated,
    /// The same for the constants of the standard modules, whose spans are kept apart.
    pub(crate) standard_constants: constants::Evaluated,
    /// The module constants whose check reported a diagnostic: they are not evaluated.
    pub(crate) refused_constants: std::collections::BTreeSet<usize>,
    /// Pending obligations: kind, primary span and the span of the declared type.
    pub(crate) obligations: Vec<(ObligationKind, Span, Option<Span>)>,
    /// The `Int` literals of a `systems` package outside the tagged range, each an `overflow`
    /// site (ADR 0109 rule 3): elaboration binds each to its own temporary at its span, so the
    /// verifier finds the site in Core IR. A negated literal is recorded at the negation, which
    /// Core IR keeps as the negation of a literal.
    pub(crate) int_literals: std::collections::BTreeSet<Span>,
    /// The edits of a callee's condition that the sites of `effect_condition` obligations ask
    /// for, each with its callee, before condition (d)'s function values are known (ADR 0092,
    /// amendment of 2026-10-14).
    pub(crate) condition_edits: Vec<(conditions::EditTarget, una_core::ConditionEdit)>,
    /// The functions named as values anywhere in the package (ADR 0092 rule 1).
    pub(crate) function_values: std::collections::BTreeSet<usize>,
    /// The functions whose bodies, their lambdas' included, call a function value: a parameter,
    /// a local or a field of function type. Their running time depends on code the timing label
    /// cannot see (ADR 0116 IF5 item 4).
    pub(crate) value_callers: std::collections::BTreeSet<usize>,
    /// Each function, with each prelude function that applies a function argument
    /// (`timing::applies_function`) that its body, its lambdas' included, calls with an
    /// argument whose code is not known at the call, or names as a value (ADR 0116 IF5 item 4,
    /// #1023). A call whose every function argument is a lambda written at the call is not
    /// one.
    pub(crate) opaque_applies: std::collections::BTreeSet<(usize, usize)>,
    /// The handlers each function installs with `with` or `given` (ADR 0116 IF5 item 3).
    pub(crate) installs: std::collections::BTreeSet<(usize, usize)>,
    /// The functions whose bodies hold a task construct, `together` or `together for` (ADR 0116
    /// IF6).
    pub(crate) task_functions: std::collections::BTreeSet<usize>,
    /// The functions called or named inside a part of a task construct, lambdas written there
    /// included (ADR 0116 IF6).
    pub(crate) task_edges: std::collections::BTreeSet<usize>,
    /// The functions called or named inside a lambda, which may run wherever the lambda is
    /// called (ADR 0116 IF6).
    pub(crate) lambda_edges: std::collections::BTreeSet<usize>,
    /// The functions called or named inside a handler's operations, which run wherever the
    /// handler serves an operation (ADR 0116 IF6).
    pub(crate) handler_edges: std::collections::BTreeSet<usize>,
    /// What each handler's operation bodies run, their lambdas and tasks included, as a site's
    /// seed records it (ADR 0116 IF5 item 3, "the effect set of its operation bodies taken in
    /// the same way"): the functions they call or name, the operations they perform, the
    /// handlers they install, whether they call a function value, and the prelude functions
    /// they apply to arguments whose code is not known at the call (#1436). Its `sources` and
    /// `unplaced` are not used.
    pub(crate) handler_seeds: std::collections::BTreeMap<usize, timing::TimingSeed>,
    /// The join of `ℓ*` of every value each function's body computes, its lambdas' included,
    /// symbolic labels dropped, by function; `None` or no entry while every value is `public`
    /// (#1685). A site counts it for every function it reaches, so a value a callee labels
    /// from literals raises the site's timing label as the same value passed in would.
    pub(crate) body_labels: std::collections::BTreeMap<usize, Option<label::Label>>,
    /// The `for ever within` loops checked (ADR 0116 rule 11, IF8): each loop's span, its
    /// function, and the seed of its body's timing label.
    pub(crate) deadline_loops: Vec<DeadlineLoop>,
    /// The `for ever` loops, and the calls of functions that say `diverges`, that stand under a
    /// program counter above `public`, by their spans: Core IR marks each `quiet`, so a host stop
    /// that ends a loop they run appends no `stop` event to the trace (S-39, `spec/26 §6` rule 4).
    pub(crate) quiet: std::collections::BTreeSet<Span>,
    /// The calls under a raised program counter, by span, with their callees: [`flow::close_quiet`]
    /// adds to `quiet` those whose callee can run a construct that appends events of its own.
    pub(crate) quiet_calls: std::collections::BTreeMap<Span, usize>,
    /// The operations under a raised program counter, by span: [`flow::close_quiet`] adds them
    /// to `quiet`, since each appends an event of its own (S-42, #1515).
    pub(crate) quiet_ops: std::collections::BTreeSet<Span>,
    /// The spans of `quiet` that stand under a raised program counter or effect floor, not only
    /// a timing floor: the operations they run append no event (S-42, #1515). Core marks the
    /// others `quiet` with `timing`.
    pub(crate) hushed: std::collections::BTreeSet<Span>,
    /// The calls under a timing floor alone, by span, with their callees, which
    /// [`flow::close_quiet`] marks `quiet` as S-39 did, and not `hushed`.
    pub(crate) timing_calls: std::collections::BTreeMap<Span, usize>,
    /// The operations under a timing floor alone, by span, which [`flow::close_quiet`] marks
    /// `quiet`, and not `hushed`, when the package declares a handler: Core marks them `quiet`
    /// with `timing`, and their events stay (S-42, `spec/27 §5` rule 7).
    pub(crate) timing_ops: std::collections::BTreeSet<Span>,
    /// The `within`s checked (ADR 0117 rule 5, IF2): each one's deadline, span, function and
    /// the seed of its timing label.
    pub(crate) withins: Vec<within::WithinSite>,
    /// The operations of the host's `Http` checked, for `L_lim` (ADR 0117 IF4).
    pub(crate) http_sites: Vec<timing::HttpSite>,
    /// The readings of the host's `Clock` checked, in edition 0.4, for `L_clk` (ADR 0116
    /// amendment of 2026-11-09, #898, C1).
    pub(crate) clock_sites: Vec<timing::HttpSite>,
    /// The operations of the host's `HttpServer` checked, the hand-off sites of `L_srv` (ADR
    /// 0123, S4 amendment IF-S1).
    pub(crate) server_sites: Vec<timing::HttpSite>,
    /// The `start`s whose captures can hold a reply right or a function value (ADR 0123 IF6).
    pub(crate) server_starts: Vec<server::StartSite>,
    /// For each `start`, by its word `start`, the source names of the captures that can hold a
    /// reply right, which the engines walk for the rights that move (ADR 0123 rule 4.2).
    pub(crate) start_rights: std::collections::BTreeMap<Span, std::collections::BTreeSet<String>>,
    /// The `tasks` statements checked, each with the seed of `κ` of the whole statement, for
    /// the residue label `L_res` (ADR 0116 P9).
    pub(crate) tasks_sites: Vec<timing::TasksSite>,
    /// The check points that data above `public` can decide, reported when the package's
    /// program can run started tasks (`spec/27 §8` rule 5, #1684).
    pub(crate) switch_sites: Vec<switch::SwitchSite>,
    /// The calls of generic functions, with the labels their type arguments can carry, for
    /// the sites of `switch_sites` that only a type parameter decides (#1684).
    pub(crate) switch_calls: Vec<switch::GenericCall>,
    /// The functions whose body evaluates a `checked` predicate, in the refinement language,
    /// whose decisions record no check point with its label (#1682).
    pub(crate) unlabeled_checks: std::collections::BTreeSet<usize>,
    /// The installations of `limiter` of `std.limit` by `with` or `given` (ADR 0117 rule 9,
    /// IF5): each one's binding, its function and the seed of the timing label of the statement
    /// it scopes over.
    pub(crate) limiter_sites: Vec<LimiterSite>,
    /// The edges of the call graph of ADR 0116 P1, recorded at their call sites, each with the node whose
    /// code holds it (a function's body, a handler's operations, or `Λ` for a lambda's body)
    /// and the innermost task construct whose part holds it (P3).
    pub(crate) graph_edges: Vec<timing::GraphEdge>,
    /// Each policy called, with its `via` function, which the call runs (`spec/07 §5`): an
    /// edge from the policy to its `via` for the searches over `edges` that the timing label
    /// (ADR 0116 IF5, IF6) and the quiet calls of S-39 make (#1673).
    pub(crate) policy_vias: std::collections::BTreeSet<(usize, usize)>,
    /// For each node of the graph of ADR 0116 P1, the join of the deep labels of what its code
    /// binds, parameters included, and of the answers of the operations it performs: the data
    /// it can hold, for the reach of a `start` (#1674).
    pub(crate) reach_labels: std::collections::BTreeMap<timing::Caller, una_core::Label>,
    /// The frames of the timing sites checked, a tree by nesting per body, each item recorded
    /// once (ADR 0116 amendment of 2026-10-25, E); a site keeps its frame.
    pub(crate) frames: timing::TimingFrames,
    /// The task constructs checked, the nodes of each body's construct tree (ADR 0116 P2, P3),
    /// by the ids that edges and sites record.
    pub(crate) constructs: Vec<timing::ConstructNode>,
    /// Names bound by selfified `let`s.
    pub(crate) selfified: Vec<Span>,
    /// The conditions of `for until` loops that are in the refinement language, which the
    /// verifier assumes false in the body and true after the loop (ADR 0116 V1, V3).
    pub(crate) refinable_conditions: Vec<Span>,
    /// Calls of, and references to, functions and predicates, from the body of a function or
    /// predicate: caller, callee, and where.
    pub(crate) edges: Vec<(usize, usize, Span)>,
    /// Edition 0.3 (ADR 0093 rule 8): what each function and handler may run, by named call,
    /// function value or installation, for the releases reachable from a boundary call.
    pub(crate) graph: Vec<(crossing::Node, crossing::Node)>,
    /// The releases, `declassify` and policy calls, of each function and handler.
    pub(crate) releases: Vec<(crossing::Node, Span)>,
    /// The handlers given a function value whose code is not known where they are installed.
    pub(crate) opaque: Vec<(crossing::Node, Span)>,
    /// What the arguments of each installation of a handler outside every boundary grant it,
    /// by the span that its binding records: a boundary that runs the handler runs them too.
    pub(crate) installed: std::collections::BTreeMap<Span, crossing::Installed>,
    /// The calls into boundary modules.
    pub(crate) crossings: Vec<crossing::Crossing>,
    /// Edition 0.3 (ADR 0093 rule 15): the summary of what each binding leads to. An edge
    /// `(from, to)` says that a call into a boundary that reaches `from` reaches `to`.
    pub(crate) summary_edges: Vec<(crossing::Source, crossing::Source)>,
    /// What each call into a boundary grants by name: the binding, the boundary and the call.
    pub(crate) summary_seeds: Vec<(crossing::Source, usize, Span)>,
    /// The handlers installed outside every boundary, checked once the summary is closed.
    pub(crate) install_sites: Vec<crossing::InstallSite>,
    /// Instantiations of generic functions: the caller when it is a function, the callee, its
    /// type arguments and where.
    pub(crate) instances: Vec<(Option<usize>, usize, Vec<ty::Ty>, Span)>,
    /// The lambdas given to `of` parameters, checked in their caller's context (ADR 0062): the
    /// span of each, whether its body performs effects, and whether it can fail.
    pub(crate) of_lambdas: Vec<(Span, bool, bool)>,
    /// For each generic function, the type parameters whose values its body compares with
    /// `==` or `!=`.
    pub(crate) compared: std::collections::BTreeMap<usize, std::collections::BTreeSet<usize>>,
    /// Edition 0.2: for each private function, the constraint its body needs on each type
    /// parameter without a written one (`spec/20 §3` rule 4).
    pub(crate) needs:
        std::collections::BTreeMap<usize, std::collections::BTreeMap<usize, env::Constraint>>,
    /// The uses of `checked`, by module, in order (`spec/06 §6`).
    pub(crate) checked: Vec<(usize, Span)>,
    /// What pass 2 resolves in the package's modules, for elaboration.
    pub(crate) records: records::Records,
    /// What the silent check of the standard modules resolves, for the elaboration of their
    /// contracts. The standard modules have a source database of their own, whose file ids
    /// coincide with the package's, so their records by span are kept apart (flow sweep).
    pub(crate) standard_records: records::Records,
    /// The declarations of pass 2 with a diagnostic, by the span of their name.
    pub(crate) failed: std::collections::BTreeSet<Span>,
    /// For each handler whose operation bodies use `std.testing`, the first use (ADR 0043).
    pub(crate) testing_handlers: std::collections::BTreeMap<usize, Span>,
    /// The handlers installed by `with` bindings, and the span of each binding (ADR 0043).
    pub(crate) with_installs: Vec<(usize, Span)>,
    /// Edition 0.3: the `if error` clauses and `attempt`s on code that cannot fail (ADR 0100),
    /// judged once the package is checked.
    pub(crate) dead: Vec<dead::Site>,
    /// The calls whose failure stops where they are: handled by `if error use`, by `if error
    /// fail` or by `attempt` (ADR 0100 rule 3).
    pub(crate) contained: std::collections::BTreeSet<Span>,
    /// The spans of the calls of declared functions, apart from their mentions as values.
    pub(crate) named_calls: std::collections::BTreeSet<Span>,
    /// The constructs of the edition ledger that only the checker sees, by span: those that
    /// elaboration lowers into other Core IR, such as `Map.update` (ADR 0121 rule 8).
    pub(crate) preview_sites: std::collections::BTreeSet<(una_project::ledger::Construct, Span)>,
    /// The functions whose bodies have a site of U3007, U3008 or U3009, held ones included.
    pub(crate) family: std::collections::BTreeSet<usize>,
}

/// The parsed standard modules.
struct Standard {
    sources: SourceDb,
    interner: Interner,
    prelude: File,
    testing: File,
    /// The standard modules of edition 0.4 after `std.testing`, in their order.
    later: Vec<File>,
    /// `std.collections` (ADR 0119), parsed when [`Standard::load`] is asked for it: a package
    /// of edition 0.5 that imports it; `None` otherwise.
    collections: Option<File>,
    /// The hidden module of the built-in `update` of maps in edition 0.5 (ADR 0119 rule 18,
    /// ADR 0139 rule 6), parsed for that edition; `None` before it.
    map_0_4: Option<File>,
    /// `std.limit` (ADR 0117 rule 9), parsed when [`Standard::load`] is asked for it: a package
    /// of edition 0.4 that imports it; `None` otherwise.
    limit: Option<File>,
}

impl Standard {
    /// Parses the prelude, `std.testing` and the later standard modules of the edition, and of
    /// the optional modules only those asked for: `std.collections` when `collections`,
    /// `std.limit` when `limit`, and the map module of edition 0.5 for that edition. A module
    /// not asked for is not parsed and not in `sources` (#1894: the optional modules are a
    /// third of the standard text of editions 0.1 to 0.3, and no package of those editions can
    /// take them). The cost of a load is linear in the text parsed. The modules are part of
    /// the compiler and parse without errors (a test checks it); their spans belong to a
    /// database of their own and are never reported.
    fn load(edition: Edition, collections: bool, limit: bool) -> Standard {
        let mut sources = SourceDb::new();
        let mut interner = Interner::new();
        let StandardSource {
            prelude: text,
            testing,
            keywords,
            later,
        } = standard_source(edition);
        let prelude = sources.add("std/prelude.una", text.as_bytes());
        let testing = sources.add("std/testing.una", testing.as_bytes());
        let prelude = parse_standard(&sources, prelude, &mut interner, keywords);
        let testing = parse_standard(&sources, testing, &mut interner, keywords);
        // The modules of edition 0.4 declare measures, a construct of edition 0.3; in edition
        // 0.5, `std.time` declares constants with `const` (ADR 0126 rule 12).
        let later_keywords = keywords.max(KeywordSet::V0_4);
        let later = later
            .into_iter()
            .map(|(path, text)| {
                let file = format!("std/{}.una", path.trim_start_matches("std."));
                let id = sources.add(&file, text.as_bytes());
                parse_standard(&sources, id, &mut interner, later_keywords)
            })
            .collect();
        // `empty_set[T: Ord]` writes a constraint, which edition 0.2's syntax has.
        let collections = collections.then(|| {
            let id = sources.add("std/collections.una", COLLECTIONS.as_bytes());
            parse_standard(&sources, id, &mut interner, KeywordSet::V0_2)
        });
        let map_0_4 = (edition >= Edition::V0_5).then(|| {
            let id = sources.add("std/map_0_4.una", MAP_0_4.as_bytes());
            parse_standard(&sources, id, &mut interner, KeywordSet::V0_2)
        });
        // `std.limit` writes a measure, `Int of Millisecond`, and a handler of edition 0.4.
        let limit = limit.then(|| {
            let id = sources.add("std/limit.una", LIMIT.as_bytes());
            parse_standard(&sources, id, &mut interner, KeywordSet::V0_4)
        });
        Standard {
            sources,
            interner,
            prelude,
            testing,
            later,
            collections,
            map_0_4,
            limit,
        }
    }

    /// `std.testing`, then the later standard modules, in the order `Env::build` takes them.
    fn after_prelude(&self) -> Vec<Source<'_>> {
        std::iter::once(&self.testing)
            .chain(&self.later)
            .map(|tree| self.source(tree))
            .collect()
    }

    fn source<'a>(&'a self, tree: &'a File) -> Source<'a> {
        Source {
            tree,
            interner: &self.interner,
            reports: false,
            mount: None,
        }
    }
}

/// Whether `ty` holds, at any depth, the unknown base type that Core IR gives a checker error
/// type.
fn unknown_in(ty: &una_core::Type) -> bool {
    match ty {
        una_core::Type::Base { base, .. } => match base {
            una_core::Base::Unknown => true,
            una_core::Base::Named { args, .. } | una_core::Base::Tuple(args) => {
                args.iter().any(unknown_in)
            }
            _ => false,
        },
        una_core::Type::Fn { params, result, .. } => {
            params.iter().any(unknown_in) || unknown_in(result)
        }
    }
}

fn parse_standard(
    sources: &SourceDb,
    id: FileId,
    interner: &mut Interner,
    keywords: KeywordSet,
) -> File {
    let lexed = lex_in(sources.file(id), id, interner, keywords);
    parse(&lexed, id, interner).file
}

/// The least fixpoint of the result summaries of the generic functions (ADR 0067 L2): over
/// the strongly connected components of their calls, callees first, each component's bodies
/// are checked again until none of their summaries grows. Summaries only grow, over finitely
/// many positions and type parameters, so each loop ends, and a function is checked again
/// only with its own component.
fn summary_fixpoint(
    env: &mut Env<'_>,
    facts: &facts::KindFacts,
    sources: &SourceDb,
    edges: &[(usize, usize)],
) {
    let generic: std::collections::BTreeSet<usize> = env
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            f.module < env.prelude && f.kind == env::FnKind::Function && !f.type_params.is_empty()
        })
        .map(|(id, _)| id)
        .collect();
    let components = summary::components(&generic, edges);
    // The callers of each function within its own component, by component, from one pass over
    // the edges in their order (#375): O(E log n) in all, not a pass per component.
    let mut component_of: std::collections::BTreeMap<usize, usize> =
        std::collections::BTreeMap::new();
    for (index, component) in components.iter().enumerate() {
        for &function in component {
            component_of.insert(function, index);
        }
    }
    let mut callers_in: Vec<std::collections::BTreeMap<usize, Vec<usize>>> =
        vec![std::collections::BTreeMap::new(); components.len()];
    for &(caller, callee) in edges {
        if let (Some(&from), Some(&to)) = (component_of.get(&caller), component_of.get(&callee))
            && from == to
            && let Some(callers) = callers_in.get_mut(to)
        {
            callers.entry(callee).or_default().push(caller);
        }
    }
    // Where each function is declared, so that checking one does not scan the package (#375).
    let places = check::places(env);
    for (component, callers) in components.iter().zip(&callers_in) {
        // A worklist: a function is checked again only when the summary of a function it
        // calls has grown, so a cycle of n functions takes about n checks, not n rounds of n.
        let members: std::collections::BTreeSet<usize> = component.iter().copied().collect();
        let mut queue: std::collections::VecDeque<usize> = component.iter().copied().collect();
        let mut queued = members.clone();
        while let Some(function) = queue.pop_front() {
            queued.remove(&function);
            let before = summary::size(env, function);
            // A check that changes the solution of a label variable is followed by one more,
            // which reads the solution, so the summary counts the labels it gives (ADR 0075).
            for _ in 0..2 {
                env.edit_labels_changed = false;
                let mut scratch = Report::new(sources);
                let mut findings = Findings::default();
                let mut collected = authority::Collected::default();
                check::declarations_at(
                    env,
                    facts,
                    &mut scratch,
                    &mut findings,
                    &mut collected,
                    authority::Pass::Collect,
                    places.get(&function).map_or(&[][..], Vec::as_slice),
                );
                if !env.edit_labels_changed {
                    break;
                }
            }
            if summary::size(env, function) == before {
                continue;
            }
            for &caller in callers.get(&function).map_or(&[][..], Vec::as_slice) {
                if queued.insert(caller) {
                    queue.push_back(caller);
                }
            }
        }
    }
}

/// Collects diagnostics, with the sources their spans point into.
pub(crate) struct Report<'s> {
    sources: &'s SourceDb,
    diagnostics: Vec<Diagnostic>,
}

impl<'s> Report<'s> {
    fn new(sources: &'s SourceDb) -> Report<'s> {
        Report {
            sources,
            diagnostics: Vec::new(),
        }
    }

    /// The location of `span`, a span of a package module.
    pub(crate) fn location(&self, span: Span) -> Location {
        self.sources.location(span)
    }

    /// The source text of `span`.
    pub(crate) fn text(&self, span: Span) -> &'s str {
        self.sources
            .file(span.file)
            .text()
            .get(span.start..span.end)
            .unwrap_or("")
    }

    /// A diagnostic, to complete with repairs or related spans before [`Report::push`].
    pub(crate) fn make(&self, code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic::error(code, message, self.location(span))
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn error(&mut self, code: Code, span: Span, message: impl Into<String>) {
        let diagnostic = self.make(code, span, message);
        self.push(diagnostic);
    }

    /// Adds `repair` to the diagnostic with `code` at `span` among those reported since the
    /// first `mark` ones.
    pub(crate) fn repair_since(&mut self, mark: usize, code: Code, span: Span, repair: Repair) {
        let location = self.location(span);
        if let Some(diagnostic) = self
            .diagnostics
            .iter_mut()
            .skip(mark)
            .find(|d| d.code == code && d.primary == location)
        {
            diagnostic.repairs.push(repair);
        }
    }

    /// Whether a diagnostic with `code` is already reported at `span`.
    pub(crate) fn has(&self, code: Code, span: Span) -> bool {
        let location = self.location(span);
        self.diagnostics
            .iter()
            .any(|d| d.code == code && d.primary == location)
    }

    /// How many diagnostics have been reported so far.
    pub(crate) fn count(&self) -> usize {
        self.diagnostics.len()
    }

    /// A repair that replaces `span` with `replacement`.
    pub(crate) fn edit(
        &self,
        description: impl Into<String>,
        span: Span,
        replacement: impl Into<String>,
    ) -> Repair {
        Repair {
            widens: None,
            rank: 1,
            escape: None,
            description: description.into(),
            edit: Some(Edit {
                location: self.location(span),
                replacement: replacement.into(),
            }),
        }
    }

    /// A repair described in words, without an edit.
    pub(crate) fn advice(description: impl Into<String>) -> Repair {
        Repair {
            widens: None,
            rank: 1,
            escape: None,
            description: description.into(),
            edit: None,
        }
    }

    /// A repair described in words that takes the escape hatch `escape`: output ranks it after
    /// every repair that keeps the guarantee (ADR 0095).
    pub(crate) fn escape(escape: Escape, description: impl Into<String>) -> Repair {
        Repair {
            escape: Some(escape),
            ..Self::advice(description)
        }
    }

    pub(crate) fn related(&self, span: Span, message: impl Into<String>) -> Related {
        Related {
            location: self.location(span),
            message: message.into(),
        }
    }
}

/// The opaque predicates of the checked modules whose bodies call no other opaque predicate
/// (ADR 0095): those of other packages, whose bodies were not checked, are left out. Linear in
/// the functions and the call edges.
fn opaque_plain(
    env: &Env<'_>,
    edges: &[(usize, usize, Span)],
) -> std::collections::BTreeSet<usize> {
    let opaque = |id: usize| {
        env.functions
            .get(id)
            .is_some_and(|def| def.kind == (env::FnKind::Predicate { reflect: false }))
    };
    let composite: std::collections::BTreeSet<usize> = edges
        .iter()
        .filter(|(caller, callee, _)| opaque(*caller) && opaque(*callee))
        .map(|(caller, _, _)| *caller)
        .collect();
    env.functions
        .iter()
        .enumerate()
        .filter(|(id, def)| opaque(*id) && env.reports(def.module) && !composite.contains(id))
        .map(|(id, _)| id)
        .collect()
}

/// The candidate closest to `name` by edit distance, when it is close enough to be the
/// intended name, for repairs.
pub(crate) fn closest<'n>(
    name: &str,
    candidates: impl IntoIterator<Item = &'n str>,
) -> Option<&'n str> {
    let limit = (name.chars().count() / 3).clamp(1, 3);
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let distance = edit_distance(name, candidate);
        if distance <= limit && best.is_none_or(|(known, _)| distance < known) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The candidates close enough to `name` to be the intended name, by edit distance within the
/// limit of [`closest`], each once, in a fixed order: by distance, then bytewise; at most
/// three, each with its distance. U1001 offers them (#419).
pub(crate) fn nearest<'n>(
    name: &str,
    candidates: impl IntoIterator<Item = &'n str>,
) -> Vec<(usize, &'n str)> {
    let limit = (name.chars().count() / 3).clamp(1, 3);
    let mut close: Vec<(usize, &'n str)> = candidates
        .into_iter()
        .filter(|candidate| *candidate != name)
        .map(|candidate| (edit_distance(name, candidate), candidate))
        .filter(|(distance, _)| *distance <= limit)
        .collect();
    close.sort_unstable();
    close.dedup();
    close.truncate(3);
    close
}

/// The edit distance between `a` and `b` over scalar values, counting an insertion, a
/// deletion, a substitution or a transposition of two neighbors as one edit.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    // rows[i][j] is the distance between the first i scalars of `a` and the first j of `b`.
    let mut rows: Vec<Vec<usize>> = (0..=a.len())
        .map(|i| {
            (0..=b.len())
                .map(|j| {
                    if i == 0 {
                        j
                    } else if j == 0 {
                        i
                    } else {
                        0
                    }
                })
                .collect()
        })
        .collect();
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let at = |i: usize, j: usize, rows: &Vec<Vec<usize>>| {
                rows.get(i)
                    .and_then(|row| row.get(j))
                    .copied()
                    .unwrap_or(usize::MAX)
            };
            let same = a.get(i - 1) == b.get(j - 1);
            let mut best = at(i - 1, j, &rows)
                .saturating_add(1)
                .min(at(i, j - 1, &rows).saturating_add(1))
                .min(at(i - 1, j - 1, &rows).saturating_add(usize::from(!same)));
            if i > 1 && j > 1 && a.get(i - 1) == b.get(j - 2) && a.get(i - 2) == b.get(j - 1) {
                best = best.min(at(i - 2, j - 2, &rows).saturating_add(1));
            }
            if let Some(cell) = rows.get_mut(i).and_then(|row| row.get_mut(j)) {
                *cell = best;
            }
        }
    }
    rows.last().and_then(|row| row.last()).copied().unwrap_or(0)
}

/// The file-state label of the package from what a check of pass 2 recorded (S-14, ADR 0085),
/// and the first write that raises it above the label the checks used.
///
/// Each write contributes its path, its program counter and the highest program counter its
/// code can be entered under. A function's is the join of the program counters of its direct
/// calls, each raised by the caller's own, up to a fixpoint over the call graph; a function
/// used as a value, or a policy's `via` function, may be called anywhere under its effect
/// bound (`spec/07 §7` rule 4), `public` when it can fail; a handler operation's is its effect
/// floor, a lambda's the cap, an example's `public`. Everything is capped at what `Files.write`
/// accepts. The fixpoint ends: each round raises some function's label, within a finite
/// lattice.
fn file_state(env: &env::Env<'_>) -> (label::Label, Option<Span>) {
    use env::Entry;
    let cap = env.file_cap.clone();
    let mut entries: std::collections::BTreeMap<usize, label::Label> =
        std::collections::BTreeMap::new();
    for &f in env.file_escaped.iter().chain(env.vias.keys()) {
        let label = match env.functions.get(f) {
            Some(def) if def.fails.is_some() => label::Label::public(),
            Some(def) => {
                let uses: Vec<ty::FnUse> = def.uses.iter().map(|n| n.usage.clone()).collect();
                flow::bound_of(env, &uses).map_or_else(|| cap.clone(), |b| b.meet(&cap))
            }
            None => cap.clone(),
        };
        let current = entries.entry(f).or_insert_with(label::Label::public);
        *current = current.join(&label).meet(&cap);
    }
    let entry_of = |entries: &std::collections::BTreeMap<usize, label::Label>, e: &Entry| match e {
        Entry::Function(f) => entries.get(f).cloned().unwrap_or_else(label::Label::public),
        Entry::Floor(floor) => floor.meet(&cap),
        Entry::Lambda => cap.clone(),
        Entry::Root => label::Label::public(),
    };
    loop {
        let mut changed = false;
        for (caller, callee, pc) in &env.file_calls {
            let raised = pc.join(&entry_of(&entries, caller)).meet(&cap);
            let current = entries
                .get(callee)
                .cloned()
                .unwrap_or_else(label::Label::public);
            if !raised.flows_to(&current) {
                entries.insert(*callee, current.join(&raised).meet(&cap));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut state = label::Label::public();
    let mut raised = None;
    for (entry, site, span) in &env.file_writes {
        let contribution = site.join(&entry_of(&entries, entry)).meet(&cap);
        if raised.is_none() && !contribution.flows_to(&env.file_state) {
            raised = Some(*span);
        }
        state = state.join(&contribution);
    }
    (state, raised)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_and_rate_limited_are_variants_of_http_error_in_edition_0_4_only() {
        // ADR 0116 rule 12 and ADR 0117 rule 8: added last, so the eight variants keep their
        // indices.
        assert_eq!(PRELUDE_0_2.matches(HTTP_ERROR_0_4.0).count(), 1);
        let line = |edition| {
            standard_source(edition)
                .prelude
                .lines()
                .find(|line| line.starts_with("pub kind HttpError ="))
                .map(str::to_owned)
        };
        assert_eq!(
            line(Edition::V0_4).as_deref(),
            Some(
                "pub kind HttpError = Denied | BadUrl | BadRequest | Unreachable | TimedOut | TooLarge | Tls | Protocol | Interrupted | RateLimited"
            )
        );
        for edition in [Edition::V0_2, Edition::V0_3] {
            assert!(line(edition).is_some_and(|line| !line.contains("Interrupted")));
            assert!(line(edition).is_some_and(|line| !line.contains("RateLimited")));
        }
    }

    #[test]
    fn every_rewrite_of_the_prelude_applies_exactly_once() {
        // `str::replace` does nothing when its text is missing, so an edit of the prelude could
        // turn a rewrite of edition 0.2 off without a word (#648).
        let rewritten = of_prelude();
        for (from, to) in OF {
            assert_eq!(
                PRELUDE.matches(from).count(),
                1,
                "the prelude holds {from:?}"
            );
            assert_eq!(rewritten.matches(to).count(), 1, "edition 0.2 holds {to:?}");
            assert_eq!(
                rewritten.matches(from).count(),
                0,
                "edition 0.2 still holds {from:?}"
            );
        }
    }

    #[test]
    fn the_optional_standard_modules_are_parsed_only_when_asked_for_1894() {
        // #1894: `std.collections`, `std.limit` and the map module of edition 0.5 are parsed,
        // and are in the standard sources, only when `load` is asked for them (the checker
        // asks when the package's edition and imports take them), so a check of a package
        // that cannot take them never parses them.
        for edition in &Edition::ALL {
            let bare = Standard::load(*edition, false, false);
            assert!(bare.collections.is_none(), "{edition:?}");
            assert!(bare.limit.is_none(), "{edition:?}");
            assert_eq!(
                bare.map_0_4.is_some(),
                *edition >= Edition::V0_5,
                "{edition:?}"
            );
            let texts = bare.sources.texts();
            assert!(!texts.contains_key("std/collections.una"), "{edition:?}");
            assert!(!texts.contains_key("std/limit.una"), "{edition:?}");
            assert_eq!(
                texts.contains_key("std/map_0_4.una"),
                *edition >= Edition::V0_5,
                "{edition:?}"
            );
            let full = Standard::load(*edition, true, true);
            assert!(full.collections.is_some(), "{edition:?}");
            assert!(full.limit.is_some(), "{edition:?}");
            let texts = full.sources.texts();
            assert_eq!(
                texts.get("std/collections.una").map(String::as_str),
                Some(COLLECTIONS),
                "{edition:?}"
            );
            assert_eq!(
                texts.get("std/limit.una").map(String::as_str),
                Some(LIMIT),
                "{edition:?}"
            );
            // What every load parses is the same, whatever the optional modules.
            assert_eq!(
                bare.sources.texts().get("std/prelude.una"),
                full.sources.texts().get("std/prelude.una"),
                "{edition:?}"
            );
            assert_eq!(bare.later.len(), full.later.len(), "{edition:?}");
        }
    }

    #[test]
    fn the_standard_source_is_the_text_the_checker_loads() {
        // #648: the reference listing reads `standard_source`; it must be exactly the text
        // `Standard::load` parses, for every edition.
        for edition in &Edition::ALL {
            let source = standard_source(*edition);
            let (prelude, keywords) = if *edition >= Edition::V0_4 {
                // `HttpError` gains `Interrupted` and `RateLimited` (ADR 0116 rule 12, ADR 0117
                // rule 8), and `std.host` the server (ADR 0123 rule 1); edition 0.5 adds the
                // numeric declarations (ADR 0120, ADR 0139 rule 6).
                let http = PRELUDE_0_2.replace(HTTP_ERROR_0_4.0, HTTP_ERROR_0_4.1);
                let (numeric, keywords) = if *edition >= Edition::V0_5 {
                    (PRELUDE_0_4, KeywordSet::V0_5)
                } else {
                    ("", KeywordSet::V0_4)
                };
                (
                    format!("{}\n{http}{PRELUDE_0_3}{numeric}{HOST_0_4}", of_prelude()),
                    keywords,
                )
            } else if *edition >= Edition::V0_3 {
                (
                    format!("{}\n{PRELUDE_0_2}{PRELUDE_0_3}", of_prelude()),
                    KeywordSet::V0_3,
                )
            } else if *edition >= Edition::V0_2 {
                (format!("{}\n{PRELUDE_0_2}", of_prelude()), KeywordSet::V0_2)
            } else {
                (PRELUDE.to_owned(), KeywordSet::V0_1)
            };
            assert_eq!(source.prelude, prelude, "{edition:?}");
            let testing = testing_source(*edition);
            assert_eq!(source.testing, testing, "{edition:?}");
            assert_eq!(
                source.testing.contains("pub elapse("),
                *edition >= Edition::V0_4,
                "{edition:?}"
            );
            assert_eq!(source.keywords, keywords, "{edition:?}");
            let texts = Standard::load(*edition, false, false).sources.texts();
            assert_eq!(texts.get("std/prelude.una"), Some(&prelude), "{edition:?}");
            assert_eq!(
                texts.get("std/testing.una").map(String::as_str),
                Some(testing.as_str()),
                "{edition:?}"
            );
        }
    }

    #[test]
    fn the_standard_modules_parse_in_their_editions() {
        // A syntax error in a standard module would drop its declarations without a word, as
        // a keyword used as a name does.
        let v0_2 = format!("{}\n{PRELUDE_0_2}", of_prelude());
        let v0_3 = format!("{v0_2}{PRELUDE_0_3}");
        let v0_4 = format!("{v0_3}{HOST_0_4}");
        let v0_5 = format!("{v0_3}{PRELUDE_0_4}{HOST_0_4}");
        let testing_0_4 = testing_source(Edition::V0_4);
        assert!(testing_0_4.contains("import std.time (Millisecond)"));
        for (name, text, keywords) in [
            ("prelude", PRELUDE, KeywordSet::V0_1),
            ("prelude 0.2", v0_2.as_str(), KeywordSet::V0_2),
            ("prelude 0.3", v0_3.as_str(), KeywordSet::V0_3),
            ("prelude 0.4", v0_4.as_str(), KeywordSet::V0_4),
            ("prelude 0.5", v0_5.as_str(), KeywordSet::V0_5),
            ("testing", TESTING, KeywordSet::V0_1),
            ("testing 0.4", testing_0_4.as_str(), KeywordSet::V0_4),
        ] {
            let mut sources = SourceDb::new();
            let mut interner = Interner::new();
            let id = sources.add(name, text.as_bytes());
            let lexed = lex_in(sources.file(id), id, &mut interner, keywords);
            let parsed = parse(&lexed, id, &interner);
            assert!(lexed.errors.is_empty(), "{name}: {:?}", lexed.errors);
            assert!(parsed.errors.is_empty(), "{name}: {:?}", parsed.errors);
        }
    }

    #[test]
    fn the_prelude_error_kinds_the_formal_model_reads_as_codes_have_no_payload() {
        // #508: the formal re-check reads `FileError`, `KeyError` and `HttpError` as a
        // failure's code, the index of the variant (`formal/UnaFormal/F1/Import.lean`,
        // `codeKinds`), and renders such a value as that index. Both readings distinguish two
        // values exactly when their variants differ, which holds only while no variant carries
        // a field. Each kind must stay one line of bare variants, without type parameters, in
        // each edition's prelude; if one gains a payload, `codeKinds` must drop it first.
        let v0_2 = format!("{}\n{PRELUDE_0_2}", of_prelude());
        for (edition, text, kinds) in [
            ("0.1", PRELUDE, &["FileError", "KeyError"][..]),
            (
                "0.2",
                v0_2.as_str(),
                &["FileError", "KeyError", "HttpError"][..],
            ),
        ] {
            for kind in kinds {
                // Every declaration of the kind, in any form: `kind K =`, `kind K[T]`, or a
                // record kind whose fields follow on their own lines.
                let decl = format!("kind {kind}");
                let lines: Vec<&str> = text
                    .lines()
                    .filter(|line| {
                        line.match_indices(decl.as_str()).any(|(i, _)| {
                            !line[i + decl.len()..]
                                .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                        })
                    })
                    .collect();
                assert_eq!(
                    lines.len(),
                    1,
                    "{edition}: {kind} is declared once: {lines:?}"
                );
                let head = format!("pub kind {kind} =");
                let variants = lines[0]
                    .strip_prefix(head.as_str())
                    .unwrap_or_else(|| panic!("{edition}: `{}` is not `{head} ...`", lines[0]));
                let variants: Vec<&str> = variants
                    .split('|')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .collect();
                assert!(!variants.is_empty(), "{edition}: {kind} has variants");
                for v in variants {
                    assert!(
                        v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                        "{edition}: {kind}'s variant `{v}` carries a payload"
                    );
                }
            }
        }
    }

    #[test]
    fn every_kind_of_the_prelude_is_declared_in_core_or_known_to_every_engine() {
        // #407: the verifier and the native backend stop on a `Named` kind they cannot
        // resolve, so a sum or record of the prelude that Core does not declare must be one
        // each engine knows itself. A new kind of the prelude fails here until it is one or
        // the other, never both.
        use una_syntax::ast::{DeclKind, KindBody};
        let v0_2 = format!("{}\n{PRELUDE_0_2}", of_prelude());
        let mut kinds = Vec::new();
        for text in [PRELUDE, v0_2.as_str()] {
            let mut sources = SourceDb::new();
            let mut interner = Interner::new();
            let id = sources.add("prelude", text.as_bytes());
            let lexed = lex_in(sources.file(id), id, &mut interner, KeywordSet::V0_2);
            let parsed = parse(&lexed, id, &interner);
            for decl in &parsed.file.decls {
                if let DeclKind::Kind(kind) = &decl.kind
                    && !matches!(kind.body, KindBody::Alias { .. })
                {
                    kinds.push(interner.resolve(kind.name.name).to_owned());
                }
            }
        }
        assert!(kinds.contains(&"Entry".to_owned()), "{kinds:?}");
        for kind in &kinds {
            let core = elab::PRELUDE_CORE_KINDS.contains(&kind.as_str());
            let engine = una_core::PRELUDE_ENGINE_KINDS.contains(&kind.as_str());
            assert!(
                core != engine,
                "`{kind}`: declared in Core {core}, known to the engines {engine}"
            );
        }
        for kind in una_core::PRELUDE_ENGINE_KINDS {
            assert!(
                kinds.iter().any(|k| k == kind)
                    || ["List", "Map", "Array", "Matrix", "Tensor", "Lu"].contains(&kind),
                "`{kind}` is not a kind of the prelude"
            );
        }
    }

    #[test]
    fn the_engines_table_of_prelude_kinds_is_the_prelude_s_declarations() {
        // #415: the interpreter names, and the native backend describes, the fields of the
        // prelude's kinds that Core does not declare by `una_core::prelude_variants`. It must
        // be what the prelude declares: each variant, its fields, and each field's type, a
        // type parameter of the kind by position or `Int`, with no label.
        use una_core::PreludeField;
        use una_syntax::ast::{DeclKind, Field, KindBody, TypeCore};
        type Variants = Vec<(String, Vec<(String, Option<PreludeField>)>)>;
        let v0_2 = format!("{}\n{PRELUDE_0_2}", of_prelude());
        let mut sources = SourceDb::new();
        let mut interner = Interner::new();
        let id = sources.add("prelude", v0_2.as_bytes());
        let lexed = lex_in(sources.file(id), id, &mut interner, KeywordSet::V0_2);
        let parsed = parse(&lexed, id, &interner);
        let mut seen = Vec::new();
        for decl in &parsed.file.decls {
            let DeclKind::Kind(kind) = &decl.kind else {
                continue;
            };
            let name = interner.resolve(kind.name.name);
            let Some(table) = una_core::prelude_variants(name) else {
                continue;
            };
            seen.push(name.to_owned());
            let params: Vec<&str> = kind
                .params
                .iter()
                .map(|p| interner.resolve(p.name))
                .collect();
            let field = |field: &Field| -> (String, Option<PreludeField>) {
                assert!(field.ty.label.is_none(), "`{name}` declares a label");
                let of = match &field.ty.core {
                    TypeCore::Named { name: ty, args } if args.is_empty() => {
                        let ty = interner.resolve(ty.name);
                        match params.iter().position(|p| *p == ty) {
                            Some(position) => Some(PreludeField::Arg(position)),
                            None => (ty == "Int").then_some(PreludeField::Int),
                        }
                    }
                    _ => None,
                };
                (interner.resolve(field.name.name).to_owned(), of)
            };
            let declared: Variants = match &kind.body {
                KindBody::Sum(variants) => variants
                    .iter()
                    .map(|v| {
                        let fields = v.fields.iter().map(field).collect();
                        (interner.resolve(v.name.name).to_owned(), fields)
                    })
                    .collect(),
                KindBody::Record(fields) => {
                    vec![(name.to_owned(), fields.iter().map(field).collect())]
                }
                KindBody::Alias { .. } => panic!("`{name}` is an alias"),
            };
            let tabled: Variants = table
                .iter()
                .map(|(variant, fields)| {
                    let fields = fields
                        .iter()
                        .map(|(field, of)| ((*field).to_owned(), Some(*of)))
                        .collect();
                    ((*variant).to_owned(), fields)
                })
                .collect();
            assert_eq!(tabled, declared, "`{name}`");
        }
        let mut expected: Vec<String> = una_core::PRELUDE_ENGINE_KINDS
            .iter()
            .filter(|kind| !["List", "Map", "Array", "Matrix", "Tensor", "Lu"].contains(kind))
            .map(|kind| (*kind).to_owned())
            .collect();
        expected.sort();
        seen.sort();
        assert_eq!(
            seen, expected,
            "every kind of the table is declared in the prelude"
        );
        assert!(una_core::prelude_variants("List").is_none());
        assert!(una_core::prelude_variants("Map").is_none());
    }

    /// The diagnostics of the standard modules of `edition`, checked as a package's are.
    fn standard_diagnostics(edition: Edition) -> Vec<String> {
        let standard = Standard::load(edition, edition >= Edition::V0_5, edition >= Edition::V0_4);
        let mut report = Report::new(&standard.sources);
        let source = |tree| Source {
            tree,
            interner: &standard.interner,
            reports: true,
            mount: None,
        };
        let mut env = Env::build(
            Vec::new(),
            source(&standard.prelude),
            std::iter::once(&standard.testing)
                .chain(&standard.later)
                .map(source)
                .collect(),
            standard.map_0_4.as_ref().map(source),
            standard.collections.as_ref().map(source),
            standard.limit.as_ref().map(source),
            Vec::new(),
            edition,
            una_project::Profile::App,
            &mut report,
        );
        let facts = facts::KindFacts::new(&env);
        facts::kind_recursion(&env, &mut report);
        let mut collected = authority::Collected::default();
        {
            let mut scratch = Report::new(&standard.sources);
            let mut findings = Findings::default();
            for module in env.prelude..env.modules.len() {
                check::module(
                    &mut env,
                    &facts,
                    &mut scratch,
                    &mut findings,
                    &mut collected,
                    authority::Pass::Collect,
                    module,
                );
            }
        }
        authority::infer(&mut env, &collected, &mut report);
        let mut findings = Findings::default();
        let mut unused = authority::Collected::default();
        for module in env.prelude..env.modules.len() {
            check::module(
                &mut env,
                &facts,
                &mut report,
                &mut findings,
                &mut unused,
                authority::Pass::Check,
                module,
            );
        }
        report
            .diagnostics
            .iter()
            .map(|d| format!("{:?} {}", d.code, d.message))
            .collect()
    }

    #[test]
    fn the_standard_modules_check_without_diagnostics_in_every_edition() {
        // A type or label error in a standard module would drop or weaken what agents call,
        // without a word; checked as a package's, they report nothing.
        for edition in Edition::ALL {
            assert_eq!(
                standard_diagnostics(edition),
                Vec::<String>::new(),
                "{edition:?}"
            );
        }
    }

    #[test]
    fn nearest_names_come_by_distance_then_bytewise_three_at_most() {
        // #419: U1001's candidates, deterministic whatever the order of the scopes.
        let scope = ["car", "can", "cap", "cat", "caz", "cart", "zebra"];
        assert_eq!(
            nearest("caz", scope),
            vec![(1, "can"), (1, "cap"), (1, "car")]
        );
        let reversed: Vec<&str> = scope.iter().rev().copied().collect();
        assert_eq!(nearest("caz", reversed), nearest("caz", scope));
        // The limit grows with the name, a third of its length, from 1 to 3.
        assert_eq!(nearest("cr", ["car", "cat"]), vec![(1, "car")]);
        assert_eq!(
            nearest("totals", ["total", "tots"]),
            vec![(1, "total"), (2, "tots")]
        );
        assert_eq!(nearest("zzzzzzzz", scope), Vec::new());
        // A repeated candidate counts once, and the name itself is never offered.
        assert_eq!(nearest("cat", ["cat", "bat", "bat"]), vec![(1, "bat")]);
    }

    #[test]
    fn closest_names_are_within_a_third_of_the_length() {
        assert_eq!(closest("nme", ["name", "seats"]), Some("name"));
        assert_eq!(closest("saets", ["name", "seats"]), Some("seats"));
        assert_eq!(closest("value", ["name", "seats"]), None);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("ab", "ba"), 1);
    }
}

/// #1050: a declaration that type-checked but did not elaborate to Core IR rejects the package
/// with U8001, through the final cross-check, whatever path of the elaboration dropped it.
#[cfg(test)]
mod elaboration_gaps {
    use una_project::Manifest;

    use super::elab::hook::Break;
    use super::*;
    use crate::names::ModuleInput;

    const SOURCE: &str = "module gaps

pub kind Positive = Int where it > 0

pub capability Log
  op write(line: Text) -> Unit

pub handler quiet() for Log
  op write(line) = unit

pub predicate reflect small(n: Int) -> Bool = n < 100

pub step(n: Int) -> Int
  requires n > 0
  ensures result > n
=
  n + 1

pub halve(n: Int) -> Int = n / 2

example \"step adds one\"
  do step(1)
  expect result == 2
";

    /// The type check of `text` in `edition`, its codes and the lines they are at.
    fn run(text: &str, edition: Edition) -> (TypeCheck, Vec<(Code, u32)>) {
        let mut sources = SourceDb::new();
        let mut interner = Interner::new();
        let id = sources.add("gaps.una", text.as_bytes());
        let keywords = match edition {
            Edition::V0_1 => KeywordSet::V0_1,
            Edition::V0_2 => KeywordSet::V0_2,
            Edition::V0_3 => KeywordSet::V0_3,
            Edition::V0_4 => KeywordSet::V0_4,
            Edition::V0_5 => KeywordSet::V0_5,
        };
        let lexed = lex_in(sources.file(id), id, &mut interner, keywords);
        let parsed = parse(&lexed, id, &interner);
        assert!(lexed.errors.is_empty(), "{:?}", lexed.errors);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let settings = Manifest {
            edition,
            ..Manifest::default()
        };
        let input = PackageInput {
            sources: &sources,
            interner: &interner,
            single: true,
            modules: vec![ModuleInput {
                id,
                tree: &parsed.file,
                broken: false,
            }],
            manifest: None,
            settings: &settings,
            run: None,
            earlier_errors: false,
            locks: &[],
        };
        let check = check_types(&input);
        let codes = check
            .diagnostics
            .iter()
            .map(|d| (d.code, d.primary.start.line))
            .collect();
        (check, codes)
    }

    fn names(check: &TypeCheck) -> Vec<String> {
        check
            .program
            .modules
            .iter()
            .filter(|m| m.path == "gaps")
            .flat_map(|m| {
                m.decls
                    .iter()
                    .map(|d| elab::decl_key(d).1.to_owned())
                    .chain(m.examples.iter().map(|e| e.name.clone()))
            })
            .collect()
    }

    #[test]
    fn the_package_checks_and_elaborates_without_a_failure() {
        for edition in Edition::ALL {
            let source = SOURCE.replace("\nexample \"", &format!("\n{} \"", edition.test_word()));
            let (check, codes) = run(&source, edition);
            assert_eq!(codes, Vec::new(), "{edition:?}");
            let module = check
                .program
                .modules
                .iter()
                .find(|m| m.path == "gaps")
                .expect("the module");
            assert_eq!(module.decls.len(), 6, "{edition:?}: {:?}", names(&check));
            assert_eq!(module.examples.len(), 1, "{edition:?}");
        }
    }

    #[test]
    fn a_declaration_that_does_not_elaborate_is_an_internal_error() {
        // Each declaration, its line, broken in turn: none is accepted without its Core IR.
        for (name, line) in [
            ("Positive", 3),
            ("Log", 5),
            ("quiet", 8),
            ("small", 11),
            ("step", 13),
            ("halve", 19),
            ("step adds one", 21),
        ] {
            for edition in Edition::ALL {
                let _broken = Break::of(name);
                let source =
                    SOURCE.replace("\nexample \"", &format!("\n{} \"", edition.test_word()));
                let (check, codes) = run(&source, edition);
                assert!(
                    codes.contains(&(Code::U8001, line)),
                    "{name} in {edition:?}: {codes:?}"
                );
                assert!(
                    codes.iter().all(|(code, _)| *code == Code::U8001),
                    "{name} in {edition:?}: {codes:?}"
                );
                let message = &check
                    .diagnostics
                    .iter()
                    .find(|d| d.primary.start.line == line)
                    .expect("the diagnostic")
                    .message;
                assert!(message.contains("did not elaborate"), "{message}");
            }
        }
    }

    #[test]
    fn a_reflected_predicate_whose_local_has_no_resolution_is_an_internal_error() {
        // The regression found while adding the host-driven step: a parameter named in a
        // reflected predicate's formula looked up as a resolution, which the records never
        // hold for a local, so `Fx::res` marked the predicate broken with no diagnostic and it
        // vanished from Core IR; a refinement naming it was then refuted, or worse proven
        // without it. The lookup is forced here; the package must be rejected with U8001.
        let text = "module gaps

pub predicate reflect small(n: Int) -> Bool = n < 100

pub kind Small = Int where small(it)

pub keep(n: Small) -> Small = n
";
        for edition in Edition::ALL {
            let (check, codes) = run(text, edition);
            assert_eq!(codes, Vec::new(), "{edition:?}");
            assert!(names(&check).iter().any(|d| d == "small"));
            let _eager = Break::eager_resolution();
            let (check, codes) = run(text, edition);
            assert!(codes.contains(&(Code::U8001, 3)), "{edition:?}: {codes:?}");
            assert!(!names(&check).iter().any(|d| d == "small"));
        }
    }

    #[test]
    fn a_standard_contract_that_does_not_elaborate_is_an_internal_error() {
        // `abs` is a function of the prelude: without its signature a call into it would be
        // verified without its contract.
        let _broken = Break::of("abs");
        let (check, codes) = run(SOURCE, Edition::V0_1);
        assert_eq!(codes.len(), 1, "{codes:?}");
        let diagnostic = check.diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code, Code::U8001);
        assert_eq!(diagnostic.primary.file, "std/prelude.una");
        assert!(diagnostic.message.contains("standard function"));
    }

    #[test]
    fn an_error_that_rejects_the_package_already_needs_no_internal_error() {
        // A type error rejects the package; the declaration it leaves out of Core IR, and any
        // other, adds nothing.
        let text = format!("{SOURCE}\npub wrong() -> Int = true\n");
        let _broken = Break::of("step");
        let (_, codes) = run(&text, Edition::V0_1);
        assert!(!codes.is_empty());
        assert!(
            codes.iter().all(|(code, _)| *code != Code::U8001),
            "{codes:?}"
        );
    }

    const DECIMAL: &str = "module gaps

pub kind Amount = Decimal[2] where it >= 0.00

pub total(a: Amount, b: Amount) -> Amount = a + b

pub scaled(a: Decimal[2], rate: Decimal[4]) -> Bool
  requires rate >= 0.0000
=
  a + rate >= 0.0
";

    #[test]
    fn a_decimal_declaration_that_does_not_elaborate_is_an_internal_error() {
        // Edition 0.4's `Decimal` (`spec/29 §16`): its literals, aligned operands and
        // refinements elaborate on their own paths, which the cross-check covers as any other.
        let (check, codes) = run(DECIMAL, Edition::V0_5);
        assert_eq!(codes, Vec::new());
        assert_eq!(names(&check), ["Amount", "total", "scaled"]);
        for (name, line) in [("Amount", 3), ("total", 5), ("scaled", 7)] {
            let _broken = Break::of(name);
            let (_, codes) = run(DECIMAL, Edition::V0_5);
            assert_eq!(codes, [(Code::U8001, line)], "{name}");
        }
    }

    #[test]
    fn a_decimal_declaration_of_the_prelude_that_does_not_elaborate_is_an_internal_error() {
        // The functions of `Decimal` and the kind `Rounding`, which edition 0.4's prelude
        // declares: each is in Core IR, and one that does not elaborate rejects the package.
        let (check, codes) = run(DECIMAL, Edition::V0_5);
        assert_eq!(codes, Vec::new());
        let functions = ["parse_decimal", "as_decimal", "json_decimal"];
        for name in functions {
            assert!(
                check
                    .program
                    .library
                    .iter()
                    .any(|s| s.path.module == "prelude" && s.path.name == name),
                "{name}"
            );
        }
        assert!(check.program.modules.iter().any(|m| {
            m.path == una_core::PRELUDE_MODULE
                && m.decls
                    .iter()
                    .any(|d| elab::decl_key(d) == ("type", "Rounding"))
        }));
        for (name, what) in functions
            .iter()
            .map(|name| (*name, "standard function"))
            .chain([("Rounding", "prelude kind")])
        {
            let _broken = Break::of(name);
            let (check, codes) = run(DECIMAL, Edition::V0_5);
            assert_eq!(codes.len(), 1, "{name}: {codes:?}");
            let diagnostic = check.diagnostics.first().expect("one diagnostic");
            assert_eq!(diagnostic.code, Code::U8001, "{name}");
            assert_eq!(diagnostic.primary.file, "std/prelude.una", "{name}");
            assert!(
                diagnostic.message.contains(what),
                "{name}: {}",
                diagnostic.message
            );
        }
    }

    #[test]
    fn every_standard_declaration_elaborates_in_every_edition() {
        // The audit of #1050: no contract of the standard library, host capability or prelude
        // kind of Core IR is dropped today, in any edition.
        for edition in Edition::ALL {
            let (_, codes) = run("module gaps\n\npub one() -> Int = 1\n", edition);
            assert_eq!(codes, Vec::new(), "{edition:?}");
        }
    }
}

/// A `for ever within d` loop, for IF8 and the report's `timing` list (ADR 0116 rules 11 and
/// 21).
#[derive(Clone, Debug)]
pub(crate) struct DeadlineLoop {
    /// The loop.
    pub(crate) span: Span,
    /// The function whose body holds it.
    pub(crate) function: Option<usize>,
    /// The frame of the timing label of its body, the deadline's label included.
    pub(crate) frame: timing::TimingFrame,
}

/// An installation of `limiter` of `std.limit` (ADR 0117 rule 9, IF5): whether a token is left
/// depends on the timing of the code that shares the instance, so `τ` of the statement the
/// `with` scopes over, or of the example's `do` line for a `given` line, must flow to
/// `internal`, the label `Limited` carries.
#[derive(Clone, Debug)]
pub(crate) struct LimiterSite {
    /// The `with` binding, or the `given` line.
    pub(crate) span: Span,
    /// The function whose body holds it.
    pub(crate) function: Option<usize>,
    /// The frame of the timing label of the statement, the handlers it installs included.
    pub(crate) frame: timing::TimingFrame,
}
