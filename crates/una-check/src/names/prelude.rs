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
