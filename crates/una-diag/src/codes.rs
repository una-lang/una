//! The diagnostic code registry, mirroring the table of `spec/13 §4`.

use std::fmt;
use std::str::FromStr;

use serde::{Serialize, Serializer};

/// The domain of a diagnostic (`spec/13 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Domain {
    /// Characters, tokens, indentation and grammar.
    Syntax,
    /// Names, modules, visibility and requirements.
    Names,
    /// Kinds, types, calls and the refinement language.
    Types,
    /// Capabilities, effects and failures.
    Authority,
    /// Information flow.
    Security,
    /// Proof obligations and the solver.
    Verification,
    /// Execution of programs and examples.
    Runtime,
    /// Project files, formatting and escape budgets.
    Project,
    /// Bugs in the compiler.
    Internal,
}

impl Domain {
    /// The name of the domain in diagnostics, for example `syntax`.
    pub fn as_str(self) -> &'static str {
        match self {
            Domain::Syntax => "syntax",
            Domain::Names => "names",
            Domain::Types => "types",
            Domain::Authority => "authority",
            Domain::Security => "security",
            Domain::Verification => "verification",
            Domain::Runtime => "runtime",
            Domain::Project => "project",
            Domain::Internal => "internal",
        }
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Domain {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A registered diagnostic code (`spec/13 §4`).
///
/// Variants are declared in the order of the registry table, so the derived ordering is the
/// numeric order of the codes. Gaps in the numbering, such as U4006 and U6002, are unassigned
/// numbers, not retired codes (ADR 0012).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Code {
    /// Invalid character, byte order mark or lone carriage return.
    U0001,
    /// Tab in indentation.
    U0002,
    /// Inconsistent indentation.
    U0003,
    /// Unterminated text literal.
    U0004,
    /// Invalid escape sequence.
    U0005,
    /// Text literal inside interpolation.
    U0006,
    /// Malformed literal.
    U0007,
    /// Unexpected token.
    U0101,
    /// Clause out of order or repeated.
    U0102,
    /// Reserved keyword not available in the package's edition.
    U0103,
    /// Unresolved name.
    U1001,
    /// Duplicate definition.
    U1002,
    /// Import cycle.
    U1003,
    /// Use of a private declaration from another module.
    U1004,
    /// Naming convention violation.
    U1005,
    /// Undeclared requirement in `derived from`.
    U1006,
    /// Generated requirement marked active.
    U1007,
    /// Module path does not match file path.
    U1008,
    /// Reserved module name.
    U1009,
    /// A module whose first path segment is a dependency's local name.
    U1010,
    /// Type mismatch.
    U2001,
    /// Unknown or ill-formed type.
    U2002,
    /// Wrong number of type arguments.
    U2003,
    /// Missing argument label.
    U2004,
    /// Unknown argument label.
    U2005,
    /// Duplicate argument.
    U2006,
    /// Non-exhaustive `when` over variants.
    U2007,
    /// Unreachable arm or misplaced `else`.
    U2008,
    /// Missing or unknown field in constructor.
    U2009,
    /// Cannot infer lambda parameter type.
    U2010,
    /// Statement value is not `Unit`.
    U2011,
    /// `result` outside `ensures` or `expect`.
    U2012,
    /// `it` or `error` outside its allowed position.
    U2013,
    /// Expression outside the refinement language.
    U2014,
    /// Invalid predicate body (impure, failing, recursive or outside RL for `reflect`).
    U2015,
    /// Opaque predicate called outside `checked`.
    U2016,
    /// Use of a binding after it was passed by `take`.
    U2017,
    /// An `edit` argument together with another use of its binding in the same call.
    U2018,
    /// Assignment to a binding that is not editable.
    U2019,
    /// A parameter convention (`edit`, `take`) not matched at the call site, or declared where none is allowed.
    U2020,
    /// A function with `edit` or `take` parameters used as a value.
    U2021,
    /// Integer literal does not fit its type.
    U2022,
    /// A binding declared outside a `together` branch assigned, or passed as `edit` or `take`, inside it.
    U2023,
    /// A `together` body with a statement other than a binding statement, or fewer than two branches.
    U2024,
    /// A type argument does not satisfy the constraint of its parameter.
    U2025,
    /// An operation on a type parameter that its constraint does not allow.
    U2026,
    /// An unknown constraint name.
    U2027,
    /// Values of different units meet, or a measured and a unitless value (ADR 0104).
    U2028,
    /// An invalid unit declaration or unit expression (ADR 0104).
    U2029,
    /// A unit conversion without an expected unit, or not exact (ADR 0104).
    U2030,
    /// A float literal rounds to an infinity, or is not zero and rounds to zero (ADR 0114).
    U2031,
    /// A float literal other than zero, or a float range, as a pattern (ADR 0114).
    U2032,
    /// A comparison of an `F64` with a float literal zero (ADR 0114).
    U2033,
    /// `stop` or `skip` outside a loop body, or across a task boundary.
    U2034,
    /// `first of` with fewer than two branches, or a line that is not an expression; `start`
    /// outside the block of a `tasks`, inside a task within it, or with a `with` clause (ADR
    /// 0117 rules 6 and 7).
    U2035,
    /// A decimal scale out of place (ADR 0120).
    U2037,
    /// A tensor shape, index, permutation or axis out of place (ADR 0120).
    U2038,
    /// A `map`, `zip_map` or `update` in a function with `simd required` whose lambda cannot be
    /// vectorized for a listed target (ADR 0120 rule 58). Registered by ADR 0109 slice S1;
    /// reported once the clause lands with its slice.
    U2039,
    /// A statement after `stop` or `skip` in the same block, which never runs.
    U2040,
    /// Capability not in scope.
    U3001,
    /// Operation not permitted by capability type.
    U3002,
    /// Effects exceed declared `uses`.
    U3003,
    /// Capability used in a pure function.
    U3004,
    /// Capability reference used as a value.
    U3005,
    /// Effectful or failing lambda escapes.
    U3006,
    /// Fallible expression without handling.
    U3007,
    /// `if error pass` with a different error kind.
    U3008,
    /// Failure variant not in the declared error kind.
    U3009,
    /// Handler does not match its capability.
    U3010,
    /// Handler missing an operation.
    U3011,
    /// Host capability in `main` not granted by `una run`.
    U3012,
    /// Effectful or fallible call nested inside an expression.
    U3013,
    /// Host capability used in an example.
    U3014,
    /// `std.testing` used outside a handler or example.
    U3015,
    /// Second-class type used as a type argument.
    U3016,
    /// An `of` clause that names no parameter.
    U3017,
    /// An `of` clause that names a parameter not of function type.
    U3018,
    /// An `of` clause that names a function type with its own `uses` or `fails`.
    U3019,
    /// A call into a boundary module whose capability is not granted with `with` (edition 0.3,
    /// ADR 0093).
    U3020,
    /// A call that grants a boundary module more than its ceiling (edition 0.3, ADR 0093).
    U3021,
    /// A labeled operation passed to code that reads its answers lower (edition 0.3, ADR
    /// 0094).
    U3022,
    /// A locked function's header grants more than its lock.
    U3023,
    /// A call into a boundary module from which a release is reachable (edition 0.3, ADR 0093).
    U3024,
    /// An `if error` clause on an expression that cannot fail (edition 0.3, ADR 0100).
    U3025,
    /// `attempt` of a call that cannot fail (edition 0.3, ADR 0100).
    U3026,
    /// Code that may run forever where it is not allowed to.
    U3027,
    /// A function that says `diverges`, used as a value.
    U3028,
    /// A failure that must be handled where it is not.
    U3029,
    /// A `simd required` clause outside a `systems` package of edition `0.5` (ADR 0120 rule
    /// 58, ADR 0109 rule 5). Registered by ADR 0109 slice S1; reported once the clause lands
    /// with its slice.
    U3032,
    /// An operation of `HttpServer` in a branch of `together`, `together for` or `first of`
    /// (ADR 0123 rule 5).
    U3033,
    /// Flow to a lower label.
    U4001,
    /// Implicit flow through control, failure or call under raised pc.
    U4002,
    /// Sink label exceeded.
    U4003,
    /// `policy` outside a trusted module.
    U4004,
    /// Invalid policy (public `via`, or type mismatch).
    U4005,
    /// Unknown label category.
    U4007,
    /// A text literal with interpolation in a pattern.
    U4008,
    /// Obligation refuted.
    U5001,
    /// Obligation unknown.
    U5002,
    /// Solver missing or version mismatch.
    U5003,
    /// `main` failed.
    U6001,
    /// Call depth or memory limit exceeded.
    U6003,
    /// A header value contains a control character.
    U6004,
    /// A host value is not valid UTF-8.
    U6005,
    /// The floating-point environment of the platform is not the default one (ADR 0114).
    U6006,
    /// The host stopped the run before it finished (ADR 0116 rule 16).
    U6007,
    /// The host gave no random bytes for keyed hashing.
    U6009,
    /// Non-canonical formatting.
    U7001,
    /// Invalid `una.toml`.
    U7002,
    /// Escape budget exceeded.
    U7003,
    /// Missing or invalid `verification.lock`.
    U7004,
    /// Trusted module not declared in both header and `una.toml`.
    U7005,
    /// A form of a preview edition that a release changed or removed (ADR 0121 rule 19).
    U7007,
    /// Invalid `ita.una` or `itaty.una`.
    U7008,
    /// Missing, invalid or inconsistent `ita.lock`.
    U7009,
    /// A package, archive, statement, root, checkpoint or log proof that fails verification.
    U7010,
    /// A locked package that is neither in the vendor directory nor in Itaoca.
    U7011,
    /// A dependency the root may not use as written.
    U7012,
    /// No resolution: no release meets a need, or a bound of resolution is exceeded.
    U7013,
    /// A locked release yanked, under an advisory or signed after a stated compromise, or a
    /// pending recovery, a key change or a transfer of its owner.
    U7014,
    /// The executable cannot be laid out: a branch beyond its target's range, or a section
    /// larger than its format allows (ADR 0124).
    U7015,
    /// A message of the editor that `una lsp` refuses: past a bound of its transport, not
    /// framed or not JSON (#2156).
    U7016,
    /// Internal compiler error (always a bug).
    U8001,
}

impl Code {
    /// Every registered code, in the order of `spec/13 §4`.
    pub const ALL: [Code; 123] = [
        Code::U0001,
        Code::U0002,
        Code::U0003,
        Code::U0004,
        Code::U0005,
        Code::U0006,
        Code::U0007,
        Code::U0101,
        Code::U0102,
        Code::U0103,
        Code::U1001,
        Code::U1002,
        Code::U1003,
        Code::U1004,
        Code::U1005,
        Code::U1006,
        Code::U1007,
        Code::U1008,
        Code::U1009,
        Code::U1010,
        Code::U2001,
        Code::U2002,
        Code::U2003,
        Code::U2004,
        Code::U2005,
        Code::U2006,
        Code::U2007,
        Code::U2008,
        Code::U2009,
        Code::U2010,
        Code::U2011,
        Code::U2012,
        Code::U2013,
        Code::U2014,
        Code::U2015,
        Code::U2016,
        Code::U2017,
        Code::U2018,
        Code::U2019,
        Code::U2020,
        Code::U2021,
        Code::U2022,
        Code::U2023,
        Code::U2024,
        Code::U2025,
        Code::U2026,
        Code::U2027,
        Code::U2028,
        Code::U2029,
        Code::U2030,
        Code::U2031,
        Code::U2032,
        Code::U2033,
        Code::U2034,
        Code::U2035,
        Code::U2037,
        Code::U2038,
        Code::U2039,
        Code::U2040,
        Code::U3001,
        Code::U3002,
        Code::U3003,
        Code::U3004,
        Code::U3005,
        Code::U3006,
        Code::U3007,
        Code::U3008,
        Code::U3009,
        Code::U3010,
        Code::U3011,
        Code::U3012,
        Code::U3013,
        Code::U3014,
        Code::U3015,
        Code::U3016,
        Code::U3017,
        Code::U3018,
        Code::U3019,
        Code::U3020,
        Code::U3021,
        Code::U3022,
        Code::U3023,
        Code::U3024,
        Code::U3025,
        Code::U3026,
        Code::U3027,
        Code::U3028,
        Code::U3029,
        Code::U3032,
        Code::U3033,
        Code::U4001,
        Code::U4002,
        Code::U4003,
        Code::U4004,
        Code::U4005,
        Code::U4007,
        Code::U4008,
        Code::U5001,
        Code::U5002,
        Code::U5003,
        Code::U6001,
        Code::U6003,
        Code::U6004,
        Code::U6005,
        Code::U6006,
        Code::U6007,
        Code::U6009,
        Code::U7001,
        Code::U7002,
        Code::U7003,
        Code::U7004,
        Code::U7005,
        Code::U7007,
        Code::U7008,
        Code::U7009,
        Code::U7010,
        Code::U7011,
        Code::U7012,
        Code::U7013,
        Code::U7014,
        Code::U7015,
        Code::U7016,
        Code::U8001,
    ];

    /// The code as written in diagnostics, for example `U0001`.
    pub fn as_str(self) -> &'static str {
        self.entry().code
    }

    /// The domain of the code.
    pub fn domain(self) -> Domain {
        self.entry().domain
    }

    /// The meaning of the code, as written in `spec/13 §4`.
    pub fn meaning(self) -> &'static str {
        self.entry().meaning
    }

    fn entry(self) -> &'static Entry {
        match self {
            Code::U0001 => &Entry {
                code: "U0001",
                domain: Domain::Syntax,
                meaning: "Invalid character, byte order mark or lone carriage return",
            },
            Code::U0002 => &Entry {
                code: "U0002",
                domain: Domain::Syntax,
                meaning: "Tab in indentation",
            },
            Code::U0003 => &Entry {
                code: "U0003",
                domain: Domain::Syntax,
                meaning: "Inconsistent indentation",
            },
            Code::U0004 => &Entry {
                code: "U0004",
                domain: Domain::Syntax,
                meaning: "Unterminated text literal",
            },
            Code::U0005 => &Entry {
                code: "U0005",
                domain: Domain::Syntax,
                meaning: "Invalid escape sequence",
            },
            Code::U0006 => &Entry {
                code: "U0006",
                domain: Domain::Syntax,
                meaning: "Text literal inside interpolation",
            },
            Code::U0007 => &Entry {
                code: "U0007",
                domain: Domain::Syntax,
                meaning: "Malformed literal",
            },
            Code::U0101 => &Entry {
                code: "U0101",
                domain: Domain::Syntax,
                meaning: "Unexpected token",
            },
            Code::U0102 => &Entry {
                code: "U0102",
                domain: Domain::Syntax,
                meaning: "Clause out of order or repeated",
            },
            Code::U0103 => &Entry {
                code: "U0103",
                domain: Domain::Syntax,
                meaning: "Reserved keyword not available in the package's edition",
            },
            Code::U1001 => &Entry {
                code: "U1001",
                domain: Domain::Names,
                meaning: "Unresolved name",
            },
            Code::U1002 => &Entry {
                code: "U1002",
                domain: Domain::Names,
                meaning: "Duplicate definition",
            },
            Code::U1003 => &Entry {
                code: "U1003",
                domain: Domain::Names,
                meaning: "Import cycle",
            },
            Code::U1004 => &Entry {
                code: "U1004",
                domain: Domain::Names,
                meaning: "Use of a private declaration from another module",
            },
            Code::U1005 => &Entry {
                code: "U1005",
                domain: Domain::Names,
                meaning: "Naming convention violation",
            },
            Code::U1006 => &Entry {
                code: "U1006",
                domain: Domain::Names,
                meaning: "Undeclared requirement in `derived from`",
            },
            Code::U1007 => &Entry {
                code: "U1007",
                domain: Domain::Names,
                meaning: "Generated requirement marked active",
            },
            Code::U1008 => &Entry {
                code: "U1008",
                domain: Domain::Names,
                meaning: "Module path does not match file path",
            },
            Code::U1009 => &Entry {
                code: "U1009",
                domain: Domain::Names,
                meaning: "Reserved module name",
            },
            Code::U1010 => &Entry {
                code: "U1010",
                domain: Domain::Names,
                meaning: "A module whose first path segment is a dependency's local name",
            },
            Code::U2001 => &Entry {
                code: "U2001",
                domain: Domain::Types,
                meaning: "Type mismatch",
            },
            Code::U2002 => &Entry {
                code: "U2002",
                domain: Domain::Types,
                meaning: "Unknown or ill-formed type",
            },
            Code::U2003 => &Entry {
                code: "U2003",
                domain: Domain::Types,
                meaning: "Wrong number of type arguments",
            },
            Code::U2004 => &Entry {
                code: "U2004",
                domain: Domain::Types,
                meaning: "Missing argument label",
            },
            Code::U2005 => &Entry {
                code: "U2005",
                domain: Domain::Types,
                meaning: "Unknown argument label",
            },
            Code::U2006 => &Entry {
                code: "U2006",
                domain: Domain::Types,
                meaning: "Duplicate argument",
            },
            Code::U2007 => &Entry {
                code: "U2007",
                domain: Domain::Types,
                meaning: "Non-exhaustive `when` over variants",
            },
            Code::U2008 => &Entry {
                code: "U2008",
                domain: Domain::Types,
                meaning: "Unreachable arm or misplaced `else`",
            },
            Code::U2009 => &Entry {
                code: "U2009",
                domain: Domain::Types,
                meaning: "Missing or unknown field in constructor",
            },
            Code::U2010 => &Entry {
                code: "U2010",
                domain: Domain::Types,
                meaning: "Cannot infer lambda parameter type",
            },
            Code::U2011 => &Entry {
                code: "U2011",
                domain: Domain::Types,
                meaning: "Statement value is not `Unit`",
            },
            Code::U2012 => &Entry {
                code: "U2012",
                domain: Domain::Types,
                meaning: "`result` outside `ensures` or `expect`",
            },
            Code::U2013 => &Entry {
                code: "U2013",
                domain: Domain::Types,
                meaning: "`it` or `error` outside its allowed position",
            },
            Code::U2014 => &Entry {
                code: "U2014",
                domain: Domain::Types,
                meaning: "Expression outside the refinement language",
            },
            Code::U2015 => &Entry {
                code: "U2015",
                domain: Domain::Types,
                meaning: "Invalid predicate body (impure, failing, recursive or outside RL for `reflect`)",
            },
            Code::U2016 => &Entry {
                code: "U2016",
                domain: Domain::Types,
                meaning: "Opaque predicate called outside `checked`",
            },
            Code::U2017 => &Entry {
                code: "U2017",
                domain: Domain::Types,
                meaning: "Use of a binding after it was passed by `take`",
            },
            Code::U2018 => &Entry {
                code: "U2018",
                domain: Domain::Types,
                meaning: "An `edit` argument together with another use of its binding in the same call",
            },
            Code::U2019 => &Entry {
                code: "U2019",
                domain: Domain::Types,
                meaning: "Assignment to a binding that is not editable",
            },
            Code::U2020 => &Entry {
                code: "U2020",
                domain: Domain::Types,
                meaning: "A parameter convention (`edit`, `take`) not matched at the call site, or declared where none is allowed",
            },
            Code::U2021 => &Entry {
                code: "U2021",
                domain: Domain::Types,
                meaning: "A function with `edit` or `take` parameters used as a value",
            },
            Code::U2022 => &Entry {
                code: "U2022",
                domain: Domain::Types,
                meaning: "Integer literal does not fit its type",
            },
            Code::U2023 => &Entry {
                code: "U2023",
                domain: Domain::Types,
                meaning: "A binding declared outside a `together` branch assigned, or passed as `edit` or `take`, inside it",
            },
            Code::U2024 => &Entry {
                code: "U2024",
                domain: Domain::Types,
                meaning: "A `together` body with a statement other than a binding statement, or fewer than two branches",
            },
            Code::U2025 => &Entry {
                code: "U2025",
                domain: Domain::Types,
                meaning: "A type argument does not satisfy the constraint of its parameter",
            },
            Code::U2026 => &Entry {
                code: "U2026",
                domain: Domain::Types,
                meaning: "An operation on a type parameter that its constraint does not allow",
            },
            Code::U2027 => &Entry {
                code: "U2027",
                domain: Domain::Types,
                meaning: "An unknown constraint name",
            },
            Code::U2028 => &Entry {
                code: "U2028",
                domain: Domain::Types,
                meaning: "Values of different units meet, or a measured and a unitless value",
            },
            Code::U2029 => &Entry {
                code: "U2029",
                domain: Domain::Types,
                meaning: "An invalid unit declaration or unit expression",
            },
            Code::U2030 => &Entry {
                code: "U2030",
                domain: Domain::Types,
                meaning: "A unit conversion without an expected unit, or not exact",
            },
            Code::U2031 => &Entry {
                code: "U2031",
                domain: Domain::Types,
                meaning: "A float literal rounds to an infinity, or is not zero and rounds to zero",
            },
            Code::U2032 => &Entry {
                code: "U2032",
                domain: Domain::Types,
                meaning: "A float literal other than zero, or a float range, as a pattern",
            },
            Code::U2033 => &Entry {
                code: "U2033",
                domain: Domain::Types,
                meaning: "A comparison of an `F64` with a float literal zero",
            },
            Code::U2034 => &Entry {
                code: "U2034",
                domain: Domain::Types,
                meaning: "`stop` or `skip` outside a loop body, or across a task boundary",
            },
            Code::U2035 => &Entry {
                code: "U2035",
                domain: Domain::Types,
                meaning: "`first of` with fewer than two branches, or a line that is not an expression; `start` outside the block of a `tasks`, inside a task within it, or with a `with` clause",
            },
            Code::U2037 => &Entry {
                code: "U2037",
                domain: Domain::Types,
                meaning: "A decimal scale out of place",
            },
            Code::U2038 => &Entry {
                code: "U2038",
                domain: Domain::Types,
                meaning: "A tensor shape, index, permutation or axis out of place",
            },
            Code::U2039 => &Entry {
                code: "U2039",
                domain: Domain::Types,
                meaning: "A `map`, `zip_map` or `update` in a function with `simd required` whose lambda cannot be vectorized for a listed target",
            },
            Code::U2040 => &Entry {
                code: "U2040",
                domain: Domain::Types,
                meaning: "A statement after `stop` or `skip` in the same block, which never runs",
            },
            Code::U3001 => &Entry {
                code: "U3001",
                domain: Domain::Authority,
                meaning: "Capability not in scope",
            },
            Code::U3002 => &Entry {
                code: "U3002",
                domain: Domain::Authority,
                meaning: "Operation not permitted by capability type",
            },
            Code::U3003 => &Entry {
                code: "U3003",
                domain: Domain::Authority,
                meaning: "Effects exceed declared `uses`",
            },
            Code::U3004 => &Entry {
                code: "U3004",
                domain: Domain::Authority,
                meaning: "Capability used in a pure function",
            },
            Code::U3005 => &Entry {
                code: "U3005",
                domain: Domain::Authority,
                meaning: "Capability reference used as a value",
            },
            Code::U3006 => &Entry {
                code: "U3006",
                domain: Domain::Authority,
                meaning: "Effectful or failing lambda escapes",
            },
            Code::U3007 => &Entry {
                code: "U3007",
                domain: Domain::Authority,
                meaning: "Fallible expression without handling",
            },
            Code::U3008 => &Entry {
                code: "U3008",
                domain: Domain::Authority,
                meaning: "`if error pass` with a different error kind",
            },
            Code::U3009 => &Entry {
                code: "U3009",
                domain: Domain::Authority,
                meaning: "Failure variant not in the declared error kind",
            },
            Code::U3010 => &Entry {
                code: "U3010",
                domain: Domain::Authority,
                meaning: "Handler does not match its capability",
            },
            Code::U3011 => &Entry {
                code: "U3011",
                domain: Domain::Authority,
                meaning: "Handler missing an operation",
            },
            Code::U3012 => &Entry {
                code: "U3012",
                domain: Domain::Authority,
                meaning: "Host capability in `main` not granted by `una run`",
            },
            Code::U3013 => &Entry {
                code: "U3013",
                domain: Domain::Authority,
                meaning: "Effectful or fallible call nested inside an expression",
            },
            Code::U3014 => &Entry {
                code: "U3014",
                domain: Domain::Authority,
                meaning: "Host capability used in an example",
            },
            Code::U3015 => &Entry {
                code: "U3015",
                domain: Domain::Authority,
                meaning: "`std.testing` used outside a handler or example",
            },
            Code::U3016 => &Entry {
                code: "U3016",
                domain: Domain::Authority,
                meaning: "Second-class type used as a type argument",
            },
            Code::U3017 => &Entry {
                code: "U3017",
                domain: Domain::Authority,
                meaning: "`of` clause that names no parameter",
            },
            Code::U3018 => &Entry {
                code: "U3018",
                domain: Domain::Authority,
                meaning: "`of` clause that names a parameter not of function type",
            },
            Code::U3019 => &Entry {
                code: "U3019",
                domain: Domain::Authority,
                meaning: "`of` clause that names a function type with its own `uses` or `fails`",
            },
            Code::U3020 => &Entry {
                code: "U3020",
                domain: Domain::Authority,
                meaning: "Call into a boundary module whose capability is not granted with `with`",
            },
            Code::U3021 => &Entry {
                code: "U3021",
                domain: Domain::Authority,
                meaning: "Call that grants a boundary module more than its ceiling",
            },
            Code::U3022 => &Entry {
                code: "U3022",
                domain: Domain::Authority,
                meaning: "Labeled operation passed to code that reads it lower",
            },
            Code::U3023 => &Entry {
                code: "U3023",
                domain: Domain::Authority,
                meaning: "Header that grants more than its lock in `una.toml`",
            },
            Code::U3024 => &Entry {
                code: "U3024",
                domain: Domain::Authority,
                meaning: "Call into a boundary module from which a release is reachable",
            },
            Code::U3025 => &Entry {
                code: "U3025",
                domain: Domain::Authority,
                meaning: "`if error` clause on an expression that cannot fail",
            },
            Code::U3026 => &Entry {
                code: "U3026",
                domain: Domain::Authority,
                meaning: "`attempt` of a call that cannot fail",
            },
            Code::U3027 => &Entry {
                code: "U3027",
                domain: Domain::Authority,
                meaning: "Code that may run forever where it is not allowed to",
            },
            Code::U3028 => &Entry {
                code: "U3028",
                domain: Domain::Authority,
                meaning: "A function that says `diverges`, used as a value",
            },
            Code::U3029 => &Entry {
                code: "U3029",
                domain: Domain::Authority,
                meaning: "A failure that must be handled where it is not",
            },
            Code::U3032 => &Entry {
                code: "U3032",
                domain: Domain::Authority,
                meaning: "A `simd required` clause outside a `systems` package of edition `0.5`",
            },
            Code::U3033 => &Entry {
                code: "U3033",
                domain: Domain::Authority,
                meaning: "An operation of `HttpServer` in the effect set of a `together` branch, a `together for` body, a `first of` branch, or `handle` or `on_stop` of a `serve` module",
            },
            Code::U4001 => &Entry {
                code: "U4001",
                domain: Domain::Security,
                meaning: "Flow to a lower label",
            },
            Code::U4002 => &Entry {
                code: "U4002",
                domain: Domain::Security,
                meaning: "Implicit flow through control, failure or call under raised pc",
            },
            Code::U4003 => &Entry {
                code: "U4003",
                domain: Domain::Security,
                meaning: "Sink label exceeded",
            },
            Code::U4004 => &Entry {
                code: "U4004",
                domain: Domain::Security,
                meaning: "`policy` outside a trusted module",
            },
            Code::U4005 => &Entry {
                code: "U4005",
                domain: Domain::Security,
                meaning: "Invalid policy (public `via`, or type mismatch)",
            },
            Code::U4007 => &Entry {
                code: "U4007",
                domain: Domain::Security,
                meaning: "Unknown label category",
            },
            Code::U4008 => &Entry {
                code: "U4008",
                domain: Domain::Security,
                meaning: "A text literal with interpolation in a pattern",
            },
            Code::U5001 => &Entry {
                code: "U5001",
                domain: Domain::Verification,
                meaning: "Obligation refuted",
            },
            Code::U5002 => &Entry {
                code: "U5002",
                domain: Domain::Verification,
                meaning: "Obligation unknown",
            },
            Code::U5003 => &Entry {
                code: "U5003",
                domain: Domain::Verification,
                meaning: "Solver missing or version mismatch",
            },
            Code::U6001 => &Entry {
                code: "U6001",
                domain: Domain::Runtime,
                meaning: "`main` failed",
            },
            Code::U6003 => &Entry {
                code: "U6003",
                domain: Domain::Runtime,
                meaning: "Call depth or memory limit exceeded",
            },
            Code::U6004 => &Entry {
                code: "U6004",
                domain: Domain::Runtime,
                meaning: "A header value contains a control character",
            },
            Code::U6005 => &Entry {
                code: "U6005",
                domain: Domain::Runtime,
                meaning: "A host value is not valid UTF-8",
            },
            Code::U6006 => &Entry {
                code: "U6006",
                domain: Domain::Runtime,
                meaning: "The floating-point environment of the platform is not the default one",
            },
            Code::U6007 => &Entry {
                code: "U6007",
                domain: Domain::Runtime,
                meaning: "The host stopped the run before it finished",
            },
            Code::U6009 => &Entry {
                code: "U6009",
                domain: Domain::Runtime,
                meaning: "The host gave no random bytes for keyed hashing",
            },
            Code::U7001 => &Entry {
                code: "U7001",
                domain: Domain::Project,
                meaning: "Non-canonical formatting",
            },
            Code::U7002 => &Entry {
                code: "U7002",
                domain: Domain::Project,
                meaning: "Invalid `una.toml`",
            },
            Code::U7003 => &Entry {
                code: "U7003",
                domain: Domain::Project,
                meaning: "Escape budget exceeded",
            },
            Code::U7004 => &Entry {
                code: "U7004",
                domain: Domain::Project,
                meaning: "Missing or invalid `verification.lock`",
            },
            Code::U7005 => &Entry {
                code: "U7005",
                domain: Domain::Project,
                meaning: "Trusted module not declared in both header and `una.toml`",
            },
            Code::U7007 => &Entry {
                code: "U7007",
                domain: Domain::Project,
                meaning: "A form of a preview edition that a release changed or removed",
            },
            Code::U7008 => &Entry {
                code: "U7008",
                domain: Domain::Project,
                meaning: "Invalid `ita.una` or `itaty.una`",
            },
            Code::U7009 => &Entry {
                code: "U7009",
                domain: Domain::Project,
                meaning: "Missing, invalid or inconsistent `ita.lock`",
            },
            Code::U7010 => &Entry {
                code: "U7010",
                domain: Domain::Project,
                meaning: "A package, archive, statement, root, checkpoint or log proof that fails verification",
            },
            Code::U7011 => &Entry {
                code: "U7011",
                domain: Domain::Project,
                meaning: "A locked package that is neither in the vendor directory nor in Itaoca",
            },
            Code::U7012 => &Entry {
                code: "U7012",
                domain: Domain::Project,
                meaning: "A dependency the root may not use as written",
            },
            Code::U7013 => &Entry {
                code: "U7013",
                domain: Domain::Project,
                meaning: "No resolution: no release meets a need, or a bound of resolution is exceeded",
            },
            Code::U7014 => &Entry {
                code: "U7014",
                domain: Domain::Project,
                meaning: "A locked release yanked, under an advisory or signed after a stated compromise, or a pending recovery, a key change or a transfer of its owner",
            },
            Code::U7015 => &Entry {
                code: "U7015",
                domain: Domain::Project,
                meaning: "The executable cannot be laid out: a branch beyond its target's range, or a section larger than its format allows",
            },
            Code::U7016 => &Entry {
                code: "U7016",
                domain: Domain::Project,
                meaning: "A message of the editor that `una lsp` refuses: past a bound of its transport, not framed or not JSON",
            },
            Code::U8001 => &Entry {
                code: "U8001",
                domain: Domain::Internal,
                meaning: "Internal compiler error (always a bug)",
            },
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Code {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl FromStr for Code {
    type Err = UnknownCode;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Code::ALL
            .into_iter()
            .find(|code| code.as_str() == text)
            .ok_or_else(|| UnknownCode(text.to_owned()))
    }
}

/// The error of parsing text that is not a registered code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownCode(String);

impl UnknownCode {
    /// The text that is not a registered code.
    pub fn text(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnknownCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown diagnostic code {:?}", self.0)
    }
}

impl std::error::Error for UnknownCode {}

/// One row of the registry table. The explain texts live in `una-explain`, which the runtime
/// never links (#413).
struct Entry {
    code: &'static str,
    domain: Domain,
    meaning: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_variants() {
        for code in Code::ALL {
            assert_eq!(format!("{code:?}"), code.as_str());
        }
    }

    #[test]
    fn parses_every_registered_code() {
        for code in Code::ALL {
            assert_eq!(code.as_str().parse::<Code>(), Ok(code));
        }
    }

    #[test]
    fn rejects_text_that_is_not_a_registered_code() {
        // The codes of the earlier names are written in two pieces, so that the audit for
        // old code shapes (ADR 0025) finds none in the repository.
        for text in [
            "U9999",
            "U4006",
            "U6002",
            "u0001",
            concat!("T", "0001"),
            concat!("V", "0001"),
            "",
            " U0001",
            "U0001 ",
            "U00001",
        ] {
            assert_eq!(text.parse::<Code>(), Err(UnknownCode(text.to_owned())));
        }
    }

    #[test]
    fn unknown_code_message_names_the_text() {
        let error = "U9999".parse::<Code>().unwrap_err();
        assert_eq!(error.to_string(), "unknown diagnostic code \"U9999\"");
        assert_eq!(error.text(), "U9999");
    }
}
