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
    /// Reserved keyword not available in v0.1.
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
    /// Obligation refuted.
    U5001,
    /// Obligation unknown.
    U5002,
    /// Solver missing or version mismatch.
    U5003,
    /// `main` failed.
    U6001,
    /// Call depth exceeded.
    U6003,
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
    /// Internal compiler error (always a bug).
    U8001,
}

impl Code {
    /// Every registered code, in the order of `spec/13 §4`.
    pub const ALL: [Code; 68] = [
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
        Code::U4001,
        Code::U4002,
        Code::U4003,
        Code::U4004,
        Code::U4005,
        Code::U4007,
        Code::U5001,
        Code::U5002,
        Code::U5003,
        Code::U6001,
        Code::U6003,
        Code::U7001,
        Code::U7002,
        Code::U7003,
        Code::U7004,
        Code::U7005,
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

    /// The text that `una explain` prints for the code.
    pub fn explain_text(self) -> &'static str {
        self.entry().explain
    }

    fn entry(self) -> &'static Entry {
        match self {
            Code::U0001 => &Entry {
                code: "U0001",
                domain: Domain::Syntax,
                meaning: "Invalid character, byte order mark or lone carriage return",
                explain: include_str!("../explain/U0001.md"),
            },
            Code::U0002 => &Entry {
                code: "U0002",
                domain: Domain::Syntax,
                meaning: "Tab in indentation",
                explain: include_str!("../explain/U0002.md"),
            },
            Code::U0003 => &Entry {
                code: "U0003",
                domain: Domain::Syntax,
                meaning: "Inconsistent indentation",
                explain: include_str!("../explain/U0003.md"),
            },
            Code::U0004 => &Entry {
                code: "U0004",
                domain: Domain::Syntax,
                meaning: "Unterminated text literal",
                explain: include_str!("../explain/U0004.md"),
            },
            Code::U0005 => &Entry {
                code: "U0005",
                domain: Domain::Syntax,
                meaning: "Invalid escape sequence",
                explain: include_str!("../explain/U0005.md"),
            },
            Code::U0006 => &Entry {
                code: "U0006",
                domain: Domain::Syntax,
                meaning: "Text literal inside interpolation",
                explain: include_str!("../explain/U0006.md"),
            },
            Code::U0007 => &Entry {
                code: "U0007",
                domain: Domain::Syntax,
                meaning: "Malformed literal",
                explain: include_str!("../explain/U0007.md"),
            },
            Code::U0101 => &Entry {
                code: "U0101",
                domain: Domain::Syntax,
                meaning: "Unexpected token",
                explain: include_str!("../explain/U0101.md"),
            },
            Code::U0102 => &Entry {
                code: "U0102",
                domain: Domain::Syntax,
                meaning: "Clause out of order or repeated",
                explain: include_str!("../explain/U0102.md"),
            },
            Code::U0103 => &Entry {
                code: "U0103",
                domain: Domain::Syntax,
                meaning: "Reserved keyword not available in v0.1",
                explain: include_str!("../explain/U0103.md"),
            },
            Code::U1001 => &Entry {
                code: "U1001",
                domain: Domain::Names,
                meaning: "Unresolved name",
                explain: include_str!("../explain/U1001.md"),
            },
            Code::U1002 => &Entry {
                code: "U1002",
                domain: Domain::Names,
                meaning: "Duplicate definition",
                explain: include_str!("../explain/U1002.md"),
            },
            Code::U1003 => &Entry {
                code: "U1003",
                domain: Domain::Names,
                meaning: "Import cycle",
                explain: include_str!("../explain/U1003.md"),
            },
            Code::U1004 => &Entry {
                code: "U1004",
                domain: Domain::Names,
                meaning: "Use of a private declaration from another module",
                explain: include_str!("../explain/U1004.md"),
            },
            Code::U1005 => &Entry {
                code: "U1005",
                domain: Domain::Names,
                meaning: "Naming convention violation",
                explain: include_str!("../explain/U1005.md"),
            },
            Code::U1006 => &Entry {
                code: "U1006",
                domain: Domain::Names,
                meaning: "Undeclared requirement in `derived from`",
                explain: include_str!("../explain/U1006.md"),
            },
            Code::U1007 => &Entry {
                code: "U1007",
                domain: Domain::Names,
                meaning: "Generated requirement marked active",
                explain: include_str!("../explain/U1007.md"),
            },
            Code::U1008 => &Entry {
                code: "U1008",
                domain: Domain::Names,
                meaning: "Module path does not match file path",
                explain: include_str!("../explain/U1008.md"),
            },
            Code::U1009 => &Entry {
                code: "U1009",
                domain: Domain::Names,
                meaning: "Reserved module name",
                explain: include_str!("../explain/U1009.md"),
            },
            Code::U2001 => &Entry {
                code: "U2001",
                domain: Domain::Types,
                meaning: "Type mismatch",
                explain: include_str!("../explain/U2001.md"),
            },
            Code::U2002 => &Entry {
                code: "U2002",
                domain: Domain::Types,
                meaning: "Unknown or ill-formed type",
                explain: include_str!("../explain/U2002.md"),
            },
            Code::U2003 => &Entry {
                code: "U2003",
                domain: Domain::Types,
                meaning: "Wrong number of type arguments",
                explain: include_str!("../explain/U2003.md"),
            },
            Code::U2004 => &Entry {
                code: "U2004",
                domain: Domain::Types,
                meaning: "Missing argument label",
                explain: include_str!("../explain/U2004.md"),
            },
            Code::U2005 => &Entry {
                code: "U2005",
                domain: Domain::Types,
                meaning: "Unknown argument label",
                explain: include_str!("../explain/U2005.md"),
            },
            Code::U2006 => &Entry {
                code: "U2006",
                domain: Domain::Types,
                meaning: "Duplicate argument",
                explain: include_str!("../explain/U2006.md"),
            },
            Code::U2007 => &Entry {
                code: "U2007",
                domain: Domain::Types,
                meaning: "Non-exhaustive `when` over variants",
                explain: include_str!("../explain/U2007.md"),
            },
            Code::U2008 => &Entry {
                code: "U2008",
                domain: Domain::Types,
                meaning: "Unreachable arm or misplaced `else`",
                explain: include_str!("../explain/U2008.md"),
            },
            Code::U2009 => &Entry {
                code: "U2009",
                domain: Domain::Types,
                meaning: "Missing or unknown field in constructor",
                explain: include_str!("../explain/U2009.md"),
            },
            Code::U2010 => &Entry {
                code: "U2010",
                domain: Domain::Types,
                meaning: "Cannot infer lambda parameter type",
                explain: include_str!("../explain/U2010.md"),
            },
            Code::U2011 => &Entry {
                code: "U2011",
                domain: Domain::Types,
                meaning: "Statement value is not `Unit`",
                explain: include_str!("../explain/U2011.md"),
            },
            Code::U2012 => &Entry {
                code: "U2012",
                domain: Domain::Types,
                meaning: "`result` outside `ensures` or `expect`",
                explain: include_str!("../explain/U2012.md"),
            },
            Code::U2013 => &Entry {
                code: "U2013",
                domain: Domain::Types,
                meaning: "`it` or `error` outside its allowed position",
                explain: include_str!("../explain/U2013.md"),
            },
            Code::U2014 => &Entry {
                code: "U2014",
                domain: Domain::Types,
                meaning: "Expression outside the refinement language",
                explain: include_str!("../explain/U2014.md"),
            },
            Code::U2015 => &Entry {
                code: "U2015",
                domain: Domain::Types,
                meaning: "Invalid predicate body (impure, failing, recursive or outside RL for `reflect`)",
                explain: include_str!("../explain/U2015.md"),
            },
            Code::U2016 => &Entry {
                code: "U2016",
                domain: Domain::Types,
                meaning: "Opaque predicate called outside `checked`",
                explain: include_str!("../explain/U2016.md"),
            },
            Code::U3001 => &Entry {
                code: "U3001",
                domain: Domain::Authority,
                meaning: "Capability not in scope",
                explain: include_str!("../explain/U3001.md"),
            },
            Code::U3002 => &Entry {
                code: "U3002",
                domain: Domain::Authority,
                meaning: "Operation not permitted by capability type",
                explain: include_str!("../explain/U3002.md"),
            },
            Code::U3003 => &Entry {
                code: "U3003",
                domain: Domain::Authority,
                meaning: "Effects exceed declared `uses`",
                explain: include_str!("../explain/U3003.md"),
            },
            Code::U3004 => &Entry {
                code: "U3004",
                domain: Domain::Authority,
                meaning: "Capability used in a pure function",
                explain: include_str!("../explain/U3004.md"),
            },
            Code::U3005 => &Entry {
                code: "U3005",
                domain: Domain::Authority,
                meaning: "Capability reference used as a value",
                explain: include_str!("../explain/U3005.md"),
            },
            Code::U3006 => &Entry {
                code: "U3006",
                domain: Domain::Authority,
                meaning: "Effectful or failing lambda escapes",
                explain: include_str!("../explain/U3006.md"),
            },
            Code::U3007 => &Entry {
                code: "U3007",
                domain: Domain::Authority,
                meaning: "Fallible expression without handling",
                explain: include_str!("../explain/U3007.md"),
            },
            Code::U3008 => &Entry {
                code: "U3008",
                domain: Domain::Authority,
                meaning: "`if error pass` with a different error kind",
                explain: include_str!("../explain/U3008.md"),
            },
            Code::U3009 => &Entry {
                code: "U3009",
                domain: Domain::Authority,
                meaning: "Failure variant not in the declared error kind",
                explain: include_str!("../explain/U3009.md"),
            },
            Code::U3010 => &Entry {
                code: "U3010",
                domain: Domain::Authority,
                meaning: "Handler does not match its capability",
                explain: include_str!("../explain/U3010.md"),
            },
            Code::U3011 => &Entry {
                code: "U3011",
                domain: Domain::Authority,
                meaning: "Handler missing an operation",
                explain: include_str!("../explain/U3011.md"),
            },
            Code::U3012 => &Entry {
                code: "U3012",
                domain: Domain::Authority,
                meaning: "Host capability in `main` not granted by `una run`",
                explain: include_str!("../explain/U3012.md"),
            },
            Code::U3013 => &Entry {
                code: "U3013",
                domain: Domain::Authority,
                meaning: "Effectful or fallible call nested inside an expression",
                explain: include_str!("../explain/U3013.md"),
            },
            Code::U3014 => &Entry {
                code: "U3014",
                domain: Domain::Authority,
                meaning: "Host capability used in an example",
                explain: include_str!("../explain/U3014.md"),
            },
            Code::U3015 => &Entry {
                code: "U3015",
                domain: Domain::Authority,
                meaning: "`std.testing` used outside a handler or example",
                explain: include_str!("../explain/U3015.md"),
            },
            Code::U3016 => &Entry {
                code: "U3016",
                domain: Domain::Authority,
                meaning: "Second-class type used as a type argument",
                explain: include_str!("../explain/U3016.md"),
            },
            Code::U4001 => &Entry {
                code: "U4001",
                domain: Domain::Security,
                meaning: "Flow to a lower label",
                explain: include_str!("../explain/U4001.md"),
            },
            Code::U4002 => &Entry {
                code: "U4002",
                domain: Domain::Security,
                meaning: "Implicit flow through control, failure or call under raised pc",
                explain: include_str!("../explain/U4002.md"),
            },
            Code::U4003 => &Entry {
                code: "U4003",
                domain: Domain::Security,
                meaning: "Sink label exceeded",
                explain: include_str!("../explain/U4003.md"),
            },
            Code::U4004 => &Entry {
                code: "U4004",
                domain: Domain::Security,
                meaning: "`policy` outside a trusted module",
                explain: include_str!("../explain/U4004.md"),
            },
            Code::U4005 => &Entry {
                code: "U4005",
                domain: Domain::Security,
                meaning: "Invalid policy (public `via`, or type mismatch)",
                explain: include_str!("../explain/U4005.md"),
            },
            Code::U4007 => &Entry {
                code: "U4007",
                domain: Domain::Security,
                meaning: "Unknown label category",
                explain: include_str!("../explain/U4007.md"),
            },
            Code::U5001 => &Entry {
                code: "U5001",
                domain: Domain::Verification,
                meaning: "Obligation refuted",
                explain: include_str!("../explain/U5001.md"),
            },
            Code::U5002 => &Entry {
                code: "U5002",
                domain: Domain::Verification,
                meaning: "Obligation unknown",
                explain: include_str!("../explain/U5002.md"),
            },
            Code::U5003 => &Entry {
                code: "U5003",
                domain: Domain::Verification,
                meaning: "Solver missing or version mismatch",
                explain: include_str!("../explain/U5003.md"),
            },
            Code::U6001 => &Entry {
                code: "U6001",
                domain: Domain::Runtime,
                meaning: "`main` failed",
                explain: include_str!("../explain/U6001.md"),
            },
            Code::U6003 => &Entry {
                code: "U6003",
                domain: Domain::Runtime,
                meaning: "Call depth exceeded",
                explain: include_str!("../explain/U6003.md"),
            },
            Code::U7001 => &Entry {
                code: "U7001",
                domain: Domain::Project,
                meaning: "Non-canonical formatting",
                explain: include_str!("../explain/U7001.md"),
            },
            Code::U7002 => &Entry {
                code: "U7002",
                domain: Domain::Project,
                meaning: "Invalid `una.toml`",
                explain: include_str!("../explain/U7002.md"),
            },
            Code::U7003 => &Entry {
                code: "U7003",
                domain: Domain::Project,
                meaning: "Escape budget exceeded",
                explain: include_str!("../explain/U7003.md"),
            },
            Code::U7004 => &Entry {
                code: "U7004",
                domain: Domain::Project,
                meaning: "Missing or invalid `verification.lock`",
                explain: include_str!("../explain/U7004.md"),
            },
            Code::U7005 => &Entry {
                code: "U7005",
                domain: Domain::Project,
                meaning: "Trusted module not declared in both header and `una.toml`",
                explain: include_str!("../explain/U7005.md"),
            },
            Code::U8001 => &Entry {
                code: "U8001",
                domain: Domain::Internal,
                meaning: "Internal compiler error (always a bug)",
                explain: include_str!("../explain/U8001.md"),
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

/// One row of the registry table, with its explain text.
struct Entry {
    code: &'static str,
    domain: Domain,
    meaning: &'static str,
    explain: &'static str,
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
