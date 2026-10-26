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

/// What `std.host` exports to an import in `edition`.
pub(crate) fn std_host_names(edition: Edition) -> &'static [&'static str] {
    if edition >= Edition::V0_2 {
        &STD_HOST_NAMES_0_2
    } else {
        &STD_HOST_NAMES
    }
}

/// The public functions of `std.testing`.
pub(crate) const STD_TESTING_NAMES: [&str; 1] = ["fake_hash"];

/// The public functions of `std.testing` in edition 0.4: also `elapse` (ADR 0116 rule 19).
pub(crate) const STD_TESTING_NAMES_0_4: [&str; 2] = ["fake_hash", "elapse"];

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

/// The standard modules a package of `edition` may import (`spec/10 §11` rule 10, ADR 0119
/// rule 0).
pub(crate) fn importable(edition: Edition) -> &'static [&'static str] {
    if edition >= Edition::V0_4 {
        &[STD_HOST, STD_TESTING, STD_TIME, STD_DRIVE, STD_COLLECTIONS]
    } else {
        &[STD_HOST, STD_TESTING]
    }
}

/// The path of the standard module of the collections, edition 0.4 (ADR 0119 rule 0).
pub(crate) const STD_COLLECTIONS: &str = una_core::COLLECTIONS_MODULE;

/// What `std.collections` exports to an import in edition 0.4: its types, its kind `Ranked`
/// and the functions that make the types implemented so far (ADR 0119 rule 0). The other
/// functions join the list with the types they make.
pub(crate) const STD_COLLECTIONS_NAMES: [&str; 11] = [
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

/// The constructors of dense values that edition 0.4 adds to the prelude (`spec/29 §17`, ADR
/// 0120 rule 32). The methods of dense values are resolved by their receiver, through
/// functions of the prelude whose names start with [`DENSE_HIDDEN`], which no name in a program
/// reaches.
pub(crate) const DENSE_0_4: [&str; 5] = [
    "array",
    "array_filled",
    "matrix",
    "matrix_filled",
    "matrix_diagonal",
];

/// The prefix of the prelude functions that implement the methods and operators of dense
/// values (`spec/29 §17`): only the checker's resolution by receiver reaches them, never a name
/// written in a program.
pub(crate) const DENSE_HIDDEN: &str = "dense_";

/// The functions of `Decimal` that edition 0.4 adds to the prelude (`spec/29 §16`, ADR 0120
/// rules 16 and 17).
pub(crate) const DECIMAL_0_4: [&str; 3] = ["parse_decimal", "as_decimal", "json_decimal"];

/// The kind `Rounding` and its variants, which edition 0.4 adds to the prelude (`spec/29 §16`,
/// ADR 0120 rule 14).
pub(crate) const ROUNDING_0_4: [&str; 8] = [
    "Rounding",
    "HalfEven",
    "HalfAwayFromZero",
    "HalfTowardZero",
    "TowardZero",
    "AwayFromZero",
    "TowardPositive",
    "TowardNegative",
];

/// The variant that edition 0.4 adds to `HttpError`: an operation that a host stop
/// interrupts (ADR 0116 rule 12). Earlier editions keep the kind's eight variants, since a new
/// variant changes which `when` arms are exhaustive.
pub(crate) const HTTP_0_4: [&str; 1] = ["Interrupted"];

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
    fn the_prelude_matches_spec_14() {
        let (lower, upper) = specified();
        let ours: BTreeSet<String> = LOWER.iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(ours, lower);
        let ours: BTreeSet<String> = UPPER.iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(ours, upper);
    }
}
