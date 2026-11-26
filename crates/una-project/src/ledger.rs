//! The edition ledger (ADR 0121 rule 9, as its amendment of 2026-11-06 extends it to every
//! preview edition): one entry per construct of a preview edition, with its record, its words,
//! the names it adds, its project-file sections and codes, its clashes with the edition before,
//! its trust, its unsettled points and the releases that landed and changed it.
//!
//! The report's `preview_constructs` (ADR 0121 rule 8), the guide's list of the constructs of
//! the preview editions (`docs/guide/configure/package/edition.md`) and the clash checks of
//! rule 11 read it; a test checks that the guide's list and the ledger agree.
//!
//! The ledger is a static table of k entries, k a few dozen. [`Construct::entry`] is O(1), an
//! index into it; [`in_edition`] walks it once, O(k).

use crate::Edition;

/// A construct of a preview edition, the index of its entry in [`LEDGER`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Construct {
    // Edition 0.4, ADR 0116 (`spec/26`).
    /// `for until c decreases m`.
    ForUntil,
    /// `stop`, `stop when` and `skip`.
    StopSkip,
    /// The header clause `diverges`.
    Diverges,
    /// `for ever`, with `on stop`, and host stops.
    ForEver,
    /// `for ever within d`.
    ForEverWithin,
    /// The host-driven step: the `drive` section and `std.drive`.
    Drive,
    /// A test's `stop after N` line.
    StopAfter,
    /// The standard module `std.time`: its units and constants.
    StdTime,
    // Edition 0.4, ADR 0117 (`spec/27`).
    /// `e within d`, a deadline.
    Within,
    /// `first of`, a race.
    FirstOf,
    /// `tasks` and `start`.
    TasksStart,
    /// Limits on `http` grants.
    GrantLimits,
    /// `std.limit` and its handler `limiter`.
    Limiter,
    // Edition 0.4, ADR 0122 (`spec/30`).
    /// The manifest `ita.una`.
    ItaManifest,
    /// The lock `ita.lock`.
    ItaLock,
    /// Dependencies: `needs`, `sources` and `trust`.
    Needs,
    // Edition 0.4, ADR 0123 (`spec/31`).
    /// The capability `HttpServer` and the `serve:` grant.
    HttpServer,
    /// The host-driven form `serve`.
    Serve,
    // Edition 0.5, ADR 0119 (`spec/28`, `spec/21 §2`).
    /// The module `std.collections`.
    Collections,
    /// `Map.update`.
    MapUpdate,
    // Edition 0.5, ADR 0120 (`spec/29` §16 to §19).
    /// `Decimal[s]`.
    Decimal,
    /// The dense values `Array`, `Matrix` and `Tensor`.
    Dense,
    /// The lane-blocked sums.
    Sum,
    /// The inner product `dot`.
    Dot,
    /// The product, factorization and solve kernels.
    MatrixKernels,
    /// `simd` as a name: no longer reserved.
    SimdName,
    /// The argument labels `with:` and `and:`.
    ArgumentLabels,
    /// `F32` and the finite kinds.
    F32,
    /// `Complex[F64]` and `Complex[F32]`.
    Complex,
    // Edition 0.5, ADR 0137 (`spec/02 §6`, `spec/17 §2`).
    /// The binding words `const` and `var`, in place of `let` and `let edit`.
    ConstVar,
}

impl Construct {
    /// The number of constructs, the length of [`LEDGER`].
    pub const COUNT: usize = Construct::ConstVar as usize + 1;

    /// The construct's ledger entry: O(1).
    #[allow(clippy::indexing_slicing)]
    pub fn entry(self) -> &'static Entry {
        // INVARIANT: `LEDGER` has `Construct::COUNT` entries, one per construct, and the
        // discriminant of the last construct is `COUNT - 1`, so the index is in bounds; a test
        // checks that each entry stands at its construct's index.
        &LEDGER[self as usize]
    }
}

/// A clash with the edition before (ADR 0121 rule 9): a kind of program of that edition whose
/// result changes in the construct's edition.
#[derive(Debug)]
pub struct Clash {
    /// Its id, the record's number and a name, as `0116-within-name`; the edition test's
    /// `Stricter` entries cite it (rule 11).
    pub id: &'static str,
    /// The kind of program of the edition before.
    pub program: &'static str,
    /// Its diagnostic, or its result, in the construct's edition.
    pub diagnostic: &'static str,
}

/// What a construct adds to the trusted base (ADR 0121 rule 2).
#[derive(Debug)]
pub struct Trust {
    /// Rule 2 (a): trust a package adds itself, with its opt-in; empty for none.
    pub package: &'static str,
    /// Rule 2 (b): trust the toolchain adds for everyone, with its review, and the entries of
    /// the report's `out_of_model` that come with the construct; empty for none.
    pub toolchain: &'static str,
}

/// A part of a construct whose meaning may still change while its edition is a preview
/// (ADR 0121 rule 5).
#[derive(Debug)]
pub struct Unsettled {
    /// Its name, as the report lists it.
    pub name: &'static str,
    /// Why it is unsettled.
    pub reason: &'static str,
}

/// One entry of the ledger.
#[derive(Debug)]
pub struct Entry {
    /// The construct, the entry's index.
    pub id: Construct,
    /// Its name, as the report and the guide write it.
    pub name: &'static str,
    /// The preview edition it belongs to.
    pub edition: Edition,
    /// Its record, as `ADR 0116`.
    pub record: &'static str,
    /// The section of the specification that states it.
    pub spec: &'static str,
    /// Its reserved keywords.
    pub reserved: &'static [&'static str],
    /// Its contextual keywords.
    pub contextual: &'static [&'static str],
    /// The names it adds to the prelude.
    pub prelude: &'static [&'static str],
    /// The built-in methods it adds, as (receiver type, method).
    pub methods: &'static [(&'static str, &'static str)],
    /// The standard modules it adds, and the names it adds to one, as `std.host.HttpServer`.
    pub modules: &'static [&'static str],
    /// The variants it adds to existing kinds, as (kind, variant).
    pub variants: &'static [(&'static str, &'static str)],
    /// Its sections of `ita.una` and `ita.lock`, and the forms it adds to a grant.
    pub manifest: &'static [&'static str],
    /// The codes it registers.
    pub codes: &'static [&'static str],
    /// Its clashes with the edition before.
    pub clashes: &'static [Clash],
    /// Its trust.
    pub trust: Trust,
    /// Its unsettled points.
    pub unsettled: &'static [Unsettled],
    /// The release that landed it, as `v0.4.0`.
    pub landed: &'static str,
    /// Each release that changed it since.
    pub changed: &'static [&'static str],
}

/// The receiver of a built-in method of ADR 0119: each type of `std.collections` that has the
/// method (ADR 0119 rule 0).
pub const COLLECTION: &str = "std.collections";

const NO_TRUST: Trust = Trust {
    package: "",
    toolchain: "",
};

/// The accumulation order of ADR 0120's kernels, declared unsettled in the entries of `sum`,
/// `dot` and the matrix kernels (ADR 0120, section "Edition `0.4`", moved to edition `0.5` by
/// ADR 0139).
const ACCUMULATION_ORDER: &[Unsettled] = &[Unsettled {
    name: "the accumulation order of 0120's kernels (L, KB)",
    reason: "the lane width L of rule 45 and the block size KB of rule 46: the benchmark plan, \
             run before the edition leaves preview, may show a better value",
}];

/// The trust of ADR 0120's kernels, under ADR 0121 rule 2 (b).
const KERNEL_TRUST: Trust = Trust {
    package: "",
    toolchain: "the dispatch macro in `ffi.rs` (ADR 0120 rule 56) and the FMA allowlist of the \
                release gate, each with a security review",
};

/// The ledger, in the order of [`Construct`]: by edition, then record.
pub static LEDGER: [Entry; Construct::COUNT] = [
    Entry {
        id: Construct::ForUntil,
        name: "for until",
        edition: Edition::V0_4,
        record: "ADR 0116",
        spec: "spec/26 §4",
        reserved: &[],
        contextual: &["until", "decreases"],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::StopSkip,
        name: "stop and skip",
        edition: Edition::V0_4,
        record: "ADR 0116",
        spec: "spec/26 §3",
        reserved: &[],
        contextual: &["stop", "skip"],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &["U2034", "U2040"],
        clashes: &[Clash {
            id: "0116-stop-skip-statement",
            program: "a line that is the name `stop` or `skip` alone",
            diagnostic: "the statement of `spec/26 §3`: U2034 outside a loop body, U2040 before \
                         a statement of its block",
        }],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Diverges,
        name: "diverges",
        edition: Edition::V0_4,
        record: "ADR 0116",
        spec: "spec/26 §5",
        reserved: &["diverges"],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &["U3027", "U3028"],
        clashes: &[Clash {
            id: "0116-diverges-name",
            program: "a name `diverges`",
            diagnostic: "U0103",
        }],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::ForEver,
        name: "for ever",
        edition: Edition::V0_4,
        record: "ADR 0116",
        spec: "spec/26 §5",
        reserved: &[],
        contextual: &["ever", "on"],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[("HttpError", "Interrupted")],
        manifest: &[],
        codes: &["U6007"],
        clashes: &[Clash {
            id: "0116-interrupted-when",
            program: "an exhaustive `when` over `HttpError` with no other arm",
            diagnostic: "U2007",
        }],
        trust: Trust {
            package: "",
            toolchain: "the signal handlers of `una-sys` that turn a host stop into a stop, with \
                        a security review; `out_of_model`: the time of a host stop, interference \
                        of the host's load with the timing of an expression, and uncatchable \
                        termination",
        },
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::ForEverWithin,
        name: "for ever within",
        edition: Edition::V0_4,
        record: "ADR 0116",
        spec: "spec/26 §7",
        reserved: &["within"],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[Clash {
            id: "0116-within-name",
            program: "a name `within`",
            diagnostic: "U0103",
        }],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Drive,
        name: "drive",
        edition: Edition::V0_4,
        record: "ADR 0116",
        spec: "spec/26 §2",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &["std.drive"],
        variants: &[],
        manifest: &["drive"],
        codes: &[],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::StopAfter,
        name: "stop after",
        edition: Edition::V0_4,
        record: "ADR 0116",
        spec: "spec/26 §8",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::StdTime,
        name: "std.time",
        edition: Edition::V0_4,
        record: "ADR 0116",
        spec: "spec/26 §2",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &["std.time"],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Within,
        name: "within",
        edition: Edition::V0_4,
        record: "ADR 0117",
        spec: "spec/27 §5",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &["std.time.Timed", "std.time.Deadline"],
        variants: &[],
        manifest: &[],
        codes: &["U3029"],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::FirstOf,
        name: "first of",
        edition: Edition::V0_4,
        record: "ADR 0117",
        spec: "spec/27 §6",
        reserved: &[],
        contextual: &["first", "of"],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &["U2035"],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::TasksStart,
        name: "tasks and start",
        edition: Edition::V0_4,
        record: "ADR 0117",
        spec: "spec/27 §7",
        reserved: &[],
        contextual: &["tasks", "start"],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &["U2035"],
        clashes: &[],
        trust: Trust {
            package: "",
            toolchain: "the trusted crate `una-sys` with the stack switch of coroutines, and the \
                        native runtime rebuilt around coroutines, for packages of every edition; \
                        each change to `una-sys` gets a security review",
        },
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::GrantLimits,
        name: "grant limits",
        edition: Edition::V0_4,
        record: "ADR 0117",
        spec: "spec/27 §9",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[("HttpError", "RateLimited")],
        manifest: &["the limits of an `http` grant"],
        codes: &[],
        clashes: &[Clash {
            id: "0117-rate-limited-when",
            program: "an exhaustive `when` over `HttpError` with no other arm",
            diagnostic: "U2007",
        }],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Limiter,
        name: "limiter",
        edition: Edition::V0_4,
        record: "ADR 0117",
        spec: "spec/27 §10",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &["std.limit"],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::ItaManifest,
        name: "ita.una",
        edition: Edition::V0_4,
        record: "ADR 0122",
        spec: "spec/30 §2",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &["ita.una"],
        codes: &["U7008"],
        clashes: &[],
        trust: Trust {
            package: "",
            toolchain: "the parser of `ita.una`, `ita.lock` and `itaty.una`, with a security \
                        review",
        },
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::ItaLock,
        name: "ita.lock",
        edition: Edition::V0_4,
        record: "ADR 0122",
        spec: "spec/30 §7",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &["ita.lock"],
        codes: &["U7009"],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Needs,
        name: "needs",
        edition: Edition::V0_4,
        record: "ADR 0122",
        spec: "spec/30 §12",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &["needs", "sources", "trust"],
        codes: &["U1010", "U7010", "U7011", "U7012", "U7013", "U7014"],
        clashes: &[],
        trust: Trust {
            package: "the registry and the packages a root names, opted in by its `needs` and \
                      `sources` sections; a dependency's trusted modules, by the root's `trust` \
                      lines, bound to an identity and pinned by digest in the lock",
            toolchain: "the archive reader, the resolver, the verification of signatures, roots, \
                        checkpoints and proofs, Itaty's built-in root metadata, and the use of \
                        `una-http` by `una ita`, each with a security review",
        },
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::HttpServer,
        name: "HttpServer",
        edition: Edition::V0_4,
        record: "ADR 0123",
        spec: "spec/31 §1",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[
            "std.host.HttpServer",
            "std.host.Method",
            "std.host.Param",
            "std.host.Request",
            "std.host.Exchange",
            "std.host.Status",
            "std.host.Reply",
            "std.host.ServeError",
            "std.testing.test_request",
            "std.testing.test_exchange",
            "std.testing.TestRequestError",
        ],
        variants: &[],
        manifest: &["a `serve:` grant"],
        codes: &["U3033"],
        clashes: &[],
        trust: Trust {
            package: "the `serve:` grant",
            toolchain: "the server code of `una-http`, with a security review; `out_of_model`: \
                        the entries of ADR 0123 rule 22, for packages that serve",
        },
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Serve,
        name: "serve",
        edition: Edition::V0_4,
        record: "ADR 0123",
        spec: "spec/31 §7",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &["serve"],
        codes: &[],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Collections,
        name: "std.collections",
        edition: Edition::V0_5,
        record: "ADR 0119",
        spec: "spec/28 §2",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[
            (COLLECTION, "push"),
            (COLLECTION, "pop"),
            (COLLECTION, "peek"),
            (COLLECTION, "push_front"),
            (COLLECTION, "pop_back"),
            (COLLECTION, "peek_back"),
            (COLLECTION, "items"),
            (COLLECTION, "has"),
            (COLLECTION, "add"),
            (COLLECTION, "remove"),
            (COLLECTION, "with"),
            (COLLECTION, "without"),
            (COLLECTION, "first"),
            (COLLECTION, "last"),
            (COLLECTION, "between"),
            (COLLECTION, "union"),
            (COLLECTION, "intersection"),
            (COLLECTION, "difference"),
            (COLLECTION, "is_subset"),
            (COLLECTION, "get"),
            (COLLECTION, "set"),
            (COLLECTION, "update"),
            (COLLECTION, "keys"),
            (COLLECTION, "values"),
            (COLLECTION, "entries"),
            (COLLECTION, "same_entries"),
            (COLLECTION, "same_items"),
            (COLLECTION, "pop_ranked"),
            (COLLECTION, "peek_priority"),
        ],
        modules: &["std.collections"],
        variants: &[],
        manifest: &[],
        codes: &["U6009"],
        clashes: &[Clash {
            id: "0119-method-names",
            program: "a package function named like a built-in method of a collection, whose \
                      first parameter has that type or is a type parameter",
            diagnostic: "U1002",
        }],
        trust: Trust {
            package: "",
            toolchain: "the `getrandom` crate for the process key of keyed hashing, pinned and \
                        audited",
        },
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::MapUpdate,
        name: "Map.update",
        edition: Edition::V0_5,
        record: "ADR 0119",
        spec: "spec/21 §2",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[("Map", "update")],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[Clash {
            id: "0119-map-update",
            program: "a package function `update` whose first parameter is a `Map`",
            diagnostic: "U1002",
        }],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Decimal,
        name: "Decimal",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/29 §16",
        reserved: &[],
        contextual: &[],
        prelude: &[
            "Decimal",
            "parse_decimal",
            "as_decimal",
            "json_decimal",
            "Rounding",
        ],
        methods: &[
            ("Int", "to_decimal"),
            ("F64", "to_decimal"),
            ("F32", "to_decimal"),
        ],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &["U2037"],
        clashes: &[
            Clash {
                id: "0120-to-decimal-integer",
                program: "a package function `to_decimal` whose first parameter has an integer \
                          type, or an import of one",
                diagnostic: "U1002",
            },
            Clash {
                id: "0120-to-decimal-float",
                program: "the same, first parameter `F64` or `F32`",
                diagnostic: "U1002",
            },
            Clash {
                id: "0120-to-decimal-parameter",
                program: "the same, first parameter a type parameter",
                diagnostic: "U1002",
            },
        ],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Dense,
        name: "Array, Matrix and Tensor",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/29 §17",
        reserved: &[],
        contextual: &[],
        prelude: &[
            "Array",
            "Matrix",
            "Tensor",
            "Lu",
            "array",
            "array_filled",
            "matrix",
            "matrix_filled",
            "matrix_diagonal",
            "tensor",
            "tensor_filled",
        ],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &["U2038"],
        clashes: &[Clash {
            id: "0120-type-names",
            program: "a kind, variant, capability or unit named `F32`, `Decimal`, `Complex`, \
                      `Array`, `Matrix`, `Tensor` or `Lu`",
            diagnostic: "U1002",
        }],
        trust: Trust {
            package: "",
            toolchain: "the atomically counted buffers shared across task heaps (ADR 0120 rule \
                        28), with a security review",
        },
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Sum,
        name: "sum",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/29 §17",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[
            ("Array", "sum"),
            ("Matrix", "sum"),
            ("Matrix", "row_sums"),
            ("Matrix", "column_sums"),
            ("Tensor", "sum"),
        ],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: KERNEL_TRUST,
        unsettled: ACCUMULATION_ORDER,
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Dot,
        name: "dot",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/29 §17",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[("Array", "dot")],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: KERNEL_TRUST,
        unsettled: ACCUMULATION_ORDER,
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::MatrixKernels,
        name: "matrix kernels",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/29 §17",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[
            ("Matrix", "multiply"),
            ("Matrix", "add_product"),
            ("Matrix", "multiply_array"),
            ("Matrix", "lu"),
            ("Matrix", "cholesky"),
            ("Matrix", "solve_lower"),
            ("Matrix", "solve_upper"),
            ("Matrix", "solve"),
            ("Matrix", "solve_array"),
            ("Lu", "solve"),
            ("Tensor", "contract"),
        ],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: KERNEL_TRUST,
        unsettled: ACCUMULATION_ORDER,
        landed: "v0.4.0",
        changed: &[],
    },
    // The clause `simd required` of ADR 0120 rule 58 is refused until it lands (`spec/29
    // §17`); its entry joins the ledger with the release that accepts it. What has landed is
    // the word: `simd` is a name in edition 0.5.
    Entry {
        id: Construct::SimdName,
        name: "simd as a name",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/01 §5",
        reserved: &[],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[Clash {
            id: "0120-simd-word",
            program: "`simd` as an identifier: U0103",
            diagnostic: "accepted outside the clause: `Looser`, a message change only",
        }],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::ArgumentLabels,
        name: "with: and and: labels",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/29 §17",
        reserved: &[],
        contextual: &["with", "and"],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::F32,
        name: "F32",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/29 §18",
        reserved: &[],
        contextual: &[],
        prelude: &["F32", "parse_f32", "f32_from_bits", "Finite64", "Finite32"],
        methods: &[("Int", "to_f32"), ("F64", "to_f32")],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[
            Clash {
                id: "0120-to-f32-integer",
                program: "a package function `to_f32` whose first parameter has an integer type, \
                          or an import of one",
                diagnostic: "U1002",
            },
            Clash {
                id: "0120-to-f32-f64",
                program: "the same, first parameter `F64`",
                diagnostic: "U1002",
            },
            Clash {
                id: "0120-to-f32-parameter",
                program: "the same, first parameter a type parameter",
                diagnostic: "U1002",
            },
            Clash {
                id: "0120-prelude-names",
                program: "a declaration named like a new prelude name",
                diagnostic: "none: the declaration wins",
            },
        ],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::Complex,
        name: "Complex",
        edition: Edition::V0_5,
        record: "ADR 0120",
        spec: "spec/29 §19",
        reserved: &[],
        contextual: &[],
        prelude: &["Complex", "complex"],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[],
        trust: NO_TRUST,
        unsettled: &[],
        landed: "v0.4.0",
        changed: &[],
    },
    Entry {
        id: Construct::ConstVar,
        name: "const and var",
        edition: Edition::V0_5,
        record: "ADR 0137",
        spec: "spec/02 §6",
        reserved: &["const", "var"],
        contextual: &[],
        prelude: &[],
        methods: &[],
        modules: &[],
        variants: &[],
        manifest: &[],
        codes: &[],
        clashes: &[
            Clash {
                id: "0137-let",
                program: "a binding written `let` or `let edit`",
                diagnostic: "U7007, the form edition 0.5 changed, with the exact rewrite to `const` or `var`",
            },
            Clash {
                id: "0137-const-name",
                program: "`const` as a name",
                diagnostic: "U0101",
            },
        ],
        trust: NO_TRUST,
        unsettled: &[],
        landed: una_syntax::keywords::LET_CHANGED_IN,
        changed: &[],
    },
];

/// The ledger entries a package of `edition` may use: those of every preview edition up to it,
/// since editions are cumulative, in ledger order; none for a stable edition, and none for
/// edition `0.3`, which keeps its own terms (ADR 0121 rule 13). O(k).
pub fn in_edition(edition: Edition) -> impl Iterator<Item = &'static Entry> {
    LEDGER
        .iter()
        .filter(move |entry| edition.is_preview() && entry.edition <= edition)
}
