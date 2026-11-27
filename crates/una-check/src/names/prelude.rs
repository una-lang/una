//! The names of the prelude and of `std.testing` (`spec/14`), for name resolution.

use una_project::Edition;

/// Upper-case names of the prelude: types, variants, host capabilities and their error kinds
/// (`spec/14 §1, §3`).
pub(crate) const UPPER: [&str; 24] = [
    "Int",
    "Nat",
    "Bool",
    "Text",
    "Unit",
    "List",
    "Option",
    "Result",
    "Hashed",
    "Some",
    "None",
    "Ok",
    "Err",
    "Console",
    "Clock",
    "Env",
    "Files",
    "Keys",
    "FileError",
    "NotFound",
    "Denied",
    "Io",
    "KeyError",
    "UnknownKey",
];

/// Functions of the prelude (`spec/14 §1`).
pub(crate) const LOWER: [&str; 20] = [
    "abs",
    "min",
    "max",
    "is_digits",
    "starts_with",
    "trim",
    "to_int",
    "is_empty",
    "first",
    "at",
    "append",
    "contains",
    "map",
    "filter",
    "first_where",
    "fold",
    "sum",
    "is_some",
    "or_else",
    "is_ok",
];

/// The first segments of the module paths reserved for the standard library (ADR 0038 D7,
/// `spec/10 §11` rule 8).
pub(crate) const RESERVED_SEGMENTS: [&str; 2] = ["std", "prelude"];

/// The path of the standard module of test helpers, outside the prelude (`spec/14 §4`).
pub(crate) const STD_TESTING: &str = "std.testing";

/// The path of the standard module that declares the host capabilities (`spec/14 §3`).
pub(crate) const STD_HOST: &str = "std.host";

/// What `std.host` exports to an import in edition 0.1: the host capabilities and their
/// failure kinds (`spec/10 §11` rule 10). The prelude puts the same declarations in scope.
const STD_HOST_NAMES: [&str; 7] = [
    "Console",
    "Clock",
    "Env",
    "Files",
    "Keys",
    "FileError",
    "KeyError",
];

/// What `std.host` exports in edition 0.2: also `Http` and its kinds (`spec/24 §1`, #307).
const STD_HOST_NAMES_0_2: [&str; 11] = [
    "Console",
    "Clock",
    "Env",
    "Files",
    "Keys",
    "FileError",
    "KeyError",
    "Http",
    "Header",
    "Response",
    "HttpError",
];

/// What `std.host` exports in edition 0.4: also `HttpServer`, its kinds and `ReplyRight` (ADR
/// 0123 rules 0 and 1, `spec/31 §1`), which the prelude does not put in scope.
const STD_HOST_NAMES_0_4: [&str; 20] = [
    "Console",
    "Clock",
    "Env",
    "Files",
    "Keys",
    "FileError",
    "KeyError",
    "Http",
    "Header",
    "Response",
    "HttpError",
    "HttpServer",
    "Method",
    "Param",
    "Request",
    "Exchange",
    "Status",
    "Reply",
    "ServeError",
    "ReplyRight",
];

/// The names of `std.host` that only an import brings into scope, in edition 0.4: those of the
/// server (ADR 0123 rule 0). An import of one is never redundant.
pub(crate) const SERVER_NAMES: [&str; 9] = [
    "HttpServer",
    "Method",
    "Param",
    "Request",
    "Exchange",
    "Status",
    "Reply",
    "ServeError",
    "ReplyRight",
];

/// The variants of a sum kind of the server that `std.host` declares (ADR 0123 rule 1), which an
/// import of the kind brings into scope.
pub(crate) fn server_variants(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "Method" => Some(&["Get", "Head", "Post", "Put", "Patch", "Delete", "Options"]),
        "ServeError" => Some(&["Interrupted", "TimedOut", "Closed", "NotHeld", "BadReply"]),
        _ => None,
    }
}

/// What `std.host` exports to an import in `edition`.
pub(crate) fn std_host_names(edition: Edition) -> &'static [&'static str] {
    if edition >= Edition::V0_4 {
        &STD_HOST_NAMES_0_4
    } else if edition >= Edition::V0_2 {
        &STD_HOST_NAMES_0_2
    } else {
        &STD_HOST_NAMES
    }
}

/// The public functions of `std.testing`.
pub(crate) const STD_TESTING_NAMES: [&str; 1] = ["fake_hash"];

/// The public names of `std.testing` in edition 0.4: also `elapse` (ADR 0116 rule 19), and
/// `test_request`, `test_exchange` and `TestRequestError` (ADR 0123 rule 21).
pub(crate) const STD_TESTING_NAMES_0_4: [&str; 5] = [
    "fake_hash",
    "elapse",
    "test_request",
    "test_exchange",
    "TestRequestError",
];

/// The names of `std.testing` that name the server's kinds (ADR 0123 rule 21): an import of
/// one makes the package one that uses the server, as an import of [`SERVER_NAMES`] does.
pub(crate) const STD_TESTING_SERVER_NAMES: [&str; 3] =
    ["test_request", "test_exchange", "TestRequestError"];

/// What `std.testing` exports to an import in `edition`.
pub(crate) fn std_testing_names(edition: Edition) -> &'static [&'static str] {
    if edition >= Edition::V0_4 {
        &STD_TESTING_NAMES_0_4
    } else {
        &STD_TESTING_NAMES
    }
}

/// The path of the standard module of time units, edition 0.4 (ADR 0116 rule 0).
pub(crate) const STD_TIME: &str = "std.time";

/// What `std.time` exports: its measures, the failure kinds of `within` (ADR 0117 rule 5), and
/// the built-in constants `millisecond`, `second` and `minute`, of type `Int of Millisecond`
/// (ADR 0116 rule 0 and its amendment of 2026-10-19).
pub(crate) const STD_TIME_NAMES: [&str; 8] = [
    "Second",
    "Millisecond",
    "Minute",
    "Timed",
    "Deadline",
    "millisecond",
    "second",
    "minute",
];

/// The built-in constants of `std.time` and their values in milliseconds.
pub(crate) const STD_TIME_CONSTANTS: [(&str, i64); 3] =
    [("millisecond", 1), ("second", 1000), ("minute", 60_000)];

/// The path of the standard module of the host-driven step, edition 0.4 (ADR 0116 rule 2).
pub(crate) const STD_DRIVE: &str = "std.drive";

/// What `std.drive` exports: the kind of a tick (ADR 0116 rule 2).
pub(crate) const STD_DRIVE_NAMES: [&str; 1] = ["Tick"];

/// The path of the standard module of rate limits computed in code, edition 0.4 (ADR 0117
/// rule 9).
pub(crate) const STD_LIMIT: &str = "std.limit";

/// What `std.limit` exports: the capability `Limit`, its failure kind `Limited`, and the
/// handler `limiter`, which the runtime provides (ADR 0117 rule 9, `spec/27 §10`).
pub(crate) const STD_LIMIT_NAMES: [&str; 3] = ["Limit", "Limited", "limiter"];

/// The standard modules a package of `edition` may import (`spec/10 §11` rule 10, ADR 0119
/// rule 0, ADR 0117 rule 9): `std.collections` from edition 0.5 (ADR 0139 rule 6).
pub(crate) fn importable(edition: Edition) -> &'static [&'static str] {
    if edition >= Edition::V0_5 {
        &[
            STD_HOST,
            STD_TESTING,
            STD_TIME,
            STD_DRIVE,
            STD_COLLECTIONS,
            STD_LIMIT,
        ]
    } else if edition >= Edition::V0_4 {
        &[STD_HOST, STD_TESTING, STD_TIME, STD_DRIVE, STD_LIMIT]
    } else {
        &[STD_HOST, STD_TESTING]
    }
}

/// The path of the standard module of the collections, edition 0.5 (ADR 0119 rule 0, ADR 0139
/// rule 6).
pub(crate) const STD_COLLECTIONS: &str = una_core::COLLECTIONS_MODULE;

/// What `std.collections` exports to an import in edition 0.5: its types, its kind `Ranked`,
/// the functions that make the types implemented so far and the conversions into them (ADR 0119
/// rules 0 and 16). The other functions join the list with the types they make.
pub(crate) const STD_COLLECTIONS_NAMES: [&str; 21] = [
    "Stack",
    "Queue",
    "Set",
    "HashMap",
    "HashSet",
    "PriorityQueue",
    "Ranked",
    "BitSet",
    "empty_stack",
    "empty_queue",
    "empty_set",
    "empty_hash_map",
    "empty_hash_set",
    "lowest_first",
    "highest_first",
    "empty_bit_set",
    "to_stack",
    "to_queue",
    "to_set",
    "to_hash_set",
    "to_hash_map",
];

/// The text functions of the prelude in edition 0.2 (`spec/22 §2`).
pub(crate) const TEXT_0_2: [&str; 14] = [
    "split",
    "join",
    "ends_with",
    "contains_text",
    "find",
    "replace",
    "slice",
    "to_upper",
    "to_lower",
    "chars",
    "parse_int",
    "trim_start",
    "trim_end",
    "bytes",
];

/// The functions of `F64` that edition 0.3 adds to the prelude (`spec/29`, ADR 0114 rule 3.9).
pub(crate) const FLOAT_0_3: [&str; 6] = [
    "nan",
    "infinity",
    "f64_from_bits",
    "parse_f64",
    "as_f64",
    "json_number",
];

/// The constructors of dense values that edition 0.5 adds to the prelude (`spec/29 §17`, ADR
/// 0120 rule 32, ADR 0139 rule 6). The methods of dense values are resolved by their receiver, through
/// functions of the prelude whose names start with [`DENSE_HIDDEN`], which no name in a program
/// reaches.
pub(crate) const DENSE_0_5: [&str; 7] = [
    "array",
    "array_filled",
    "matrix",
    "matrix_filled",
    "matrix_diagonal",
    "tensor",
    "tensor_filled",
];

/// The prefix of the prelude functions that implement the methods and operators of dense
/// values (`spec/29 §17`): only the checker's resolution by receiver reaches them, never a name
/// written in a program.
pub(crate) const DENSE_HIDDEN: &str = "dense_";

/// The functions of `Decimal` that edition 0.5 adds to the prelude (`spec/29 §16`, ADR 0120
/// rules 16 and 17).
pub(crate) const DECIMAL_0_5: [&str; 3] = ["parse_decimal", "as_decimal", "json_decimal"];

/// The kind `Rounding` and its variants, which edition 0.5 adds to the prelude (`spec/29 §16`,
/// ADR 0120 rule 14).
pub(crate) const ROUNDING_0_5: [&str; 8] = [
    "Rounding",
    "HalfEven",
    "HalfAwayFromZero",
    "HalfTowardZero",
    "TowardZero",
    "AwayFromZero",
    "TowardPositive",
    "TowardNegative",
];
/// The functions of `F32` that edition 0.5 adds to the prelude (`spec/29`, ADR 0120 rule 5).
pub(crate) const FLOAT_0_5: [&str; 2] = ["f32_from_bits", "parse_f32"];

/// The function of `Complex` that edition 0.5 adds to the prelude (`spec/29 §19`, ADR 0120
/// rule 22).
pub(crate) const COMPLEX_0_5: [&str; 1] = ["complex"];

/// The upper names that edition 0.5 adds: `F32`, the finite kinds and `Complex` (ADR 0120
/// rules 5, 11 and 22).
pub(crate) const UPPER_0_5: [&str; 4] = ["F32", "Finite64", "Finite32", "Complex"];

/// Whether `name` is a built-in name of edition 0.5 that ADR 0139 rule 6 moves there from
/// edition 0.4: a name of the numeric foundation (ADR 0120) or `std.collections` (ADR 0119).
/// In edition 0.4 it is the ordinary error of an unknown name, which no release ever accepted
/// there (ADR 0139 rule 7), with the advice to raise the edition.
pub(crate) fn moved_to_0_5(name: &str) -> bool {
    DENSE_0_5.contains(&name)
        || DECIMAL_0_5.contains(&name)
        || ROUNDING_0_5.contains(&name)
        || FLOAT_0_5.contains(&name)
        || COMPLEX_0_5.contains(&name)
        || UPPER_0_5.contains(&name)
        || matches!(name, "Array" | "Matrix" | "Tensor" | "Lu" | "Decimal")
        || name == STD_COLLECTIONS
}

/// The edition from which `name` is an upper-case name built in ahead of every
/// declaration, so that no kind, variant, capability or unit of a module of that edition takes
/// it (U1002): the names that `names::Checker::declare` refuses, by the edition each rule
/// starts in. `None` for a name that no edition builds in. O(1).
pub(crate) fn built_in_upper(name: &str) -> Option<Edition> {
    if una_core::IntType::from_name(name).is_some()
        || matches!(
            name,
            "Int"
                | "Nat"
                | "Bool"
                | "Text"
                | "Unit"
                | "Hashed"
                | "List"
                | "Option"
                | "Result"
                | "Map"
                | "Entry"
                | "Json"
                | "JsonNumber"
                | "JsonError"
                | "Http"
                | "Header"
                | "Response"
                | "HttpError"
                | "Eq"
                | "Ord"
                | "Integer"
        )
    {
        return Some(Edition::V0_2);
    }
    match name {
        "F64" => Some(Edition::V0_3),
        "Decimal" | "Array" | "Matrix" | "Tensor" | "Lu" | "F32" | "Complex" => Some(Edition::V0_5),
        _ => None,
    }
}

/// The advice of an unknown name or module that [`moved_to_0_5`] holds, in edition 0.4.
pub(crate) fn moved_to_0_5_advice(name: &str) -> String {
    format!("`{name}` exists from edition 0.5 (ADR 0139): write `edition 0.5` in ita.una to use it")
}

/// The variants that edition 0.4 adds to `HttpError`: an operation that a host stop
/// interrupts (ADR 0116 rule 12), and a request whose limit's queue is full (ADR 0117 rule 8).
/// Earlier editions keep the kind's eight variants, since a new variant changes which `when`
/// arms are exhaustive.
pub(crate) const HTTP_0_4: [&str; 2] = ["Interrupted", "RateLimited"];

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

    use std::collections::BTreeSet;

    use super::*;

    /// The names of `spec/14`: functions of its tables, types and variants of the prelude line,
    /// host capabilities and their error kinds.
    fn specified() -> (BTreeSet<String>, BTreeSet<String>) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/spec/14-standard-library.md");
        let spec = std::fs::read_to_string(path).unwrap();
        let prelude = spec.split("## 2.").next().unwrap();
        let host = spec
            .split("## 3.")
            .nth(1)
            .unwrap()
            .split("## 4.")
            .next()
            .unwrap();
        let mut lower = BTreeSet::new();
        let mut upper = BTreeSet::new();
        for line in prelude.lines() {
            if let Some(row) = line.strip_prefix("| `") {
                let name: String = row
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    lower.insert(name);
                }
            }
            if let Some(types) = line.strip_prefix("**Types:**") {
                for word in types.split(|c: char| !c.is_ascii_alphanumeric()) {
                    if word.starts_with(|c: char| c.is_ascii_uppercase()) && word.len() > 1 {
                        upper.insert(word.to_owned());
                    }
                }
            }
        }
        for line in host.lines() {
            if let Some(row) = line.strip_prefix("| `") {
                let name: String = row
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                if name.starts_with(|c: char| c.is_ascii_uppercase()) {
                    upper.insert(name);
                }
            }
            if let Some(kind) = line.strip_prefix("kind ") {
                for word in kind.split(|c: char| !c.is_ascii_alphanumeric()) {
                    if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                        upper.insert(word.to_owned());
                    }
                }
            }
        }
        // `T`, `E` and `U` are type parameters in the prelude line, not names.
        for parameter in ["T", "E", "U"] {
            upper.remove(parameter);
        }
        (lower, upper)
    }

    #[test]
    fn every_name_edition_0_5_adds_to_the_prelude_is_hidden_from_edition_0_4() {
        // ADR 0122 rule 49: a module of edition 0.4 under a root of 0.5 shares the root's
        // prelude, and `Env::lower` hides from it exactly the names `moved_to_0_5` holds, by a
        // name and by method syntax. Every declaration `prelude_0_4.una` adds is one of them,
        // or a function of dense methods that no name reaches (`DENSE_HIDDEN`).
        let source = include_str!("../std/prelude_0_4.una");
        let mut seen = 0;
        for line in source.lines() {
            let Some(rest) = line.strip_prefix("pub ") else {
                continue;
            };
            let rest = rest.strip_prefix("kind ").unwrap_or(rest);
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            assert!(!name.is_empty(), "{line}");
            seen += 1;
            assert!(
                name.starts_with(DENSE_HIDDEN) || moved_to_0_5(&name),
                "`{name}` of prelude_0_4.una is reachable from a module of edition 0.4"
            );
        }
        assert!(seen > 10, "{seen}");
    }

    #[test]
    fn the_names_of_the_server_are_the_declarations_of_std_host_in_0_4() {
        // ADR 0123 rule 1: every name of the server is declared by `host_0_4.una`, or is the
        // built-in `ReplyRight`; the variants an import brings are the kinds' own; edition 0.3
        // exports none of them, and the prelude's scope holds none (rule 0).
        let source = include_str!("../std/host_0_4.una");
        let declared: BTreeSet<&str> = source
            .lines()
            .filter_map(|line| {
                let rest = line
                    .strip_prefix("pub kind ")
                    .or_else(|| line.strip_prefix("pub capability "))?;
                rest.split(|c: char| !c.is_ascii_alphanumeric()).next()
            })
            .collect();
        let mut names: BTreeSet<&str> = SERVER_NAMES.iter().copied().collect();
        assert!(names.remove("ReplyRight"));
        assert_eq!(names, declared);
        for kind in ["Method", "ServeError"] {
            let line = source
                .lines()
                .find(|line| line.starts_with(&format!("pub kind {kind} = ")))
                .unwrap();
            let variants: Vec<&str> = line
                .split_once(" = ")
                .unwrap()
                .1
                .split('|')
                .map(str::trim)
                .collect();
            assert_eq!(server_variants(kind).unwrap(), variants.as_slice());
        }
        for name in SERVER_NAMES {
            assert!(std_host_names(Edition::V0_4).contains(&name), "{name}");
            assert!(!std_host_names(Edition::V0_3).contains(&name), "{name}");
            assert!(!UPPER.contains(&name), "{name}");
        }
    }

    #[test]
    fn the_prelude_matches_spec_14() {
        let (lower, upper) = specified();
        let ours: BTreeSet<String> = LOWER.iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(ours, lower);
        let ours: BTreeSet<String> = UPPER.iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(ours, upper);
    }
}
