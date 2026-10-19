//! The built-in methods of the standard library reference (#690): `.len`, the methods of maps,
//! the conversions and the wrapping, saturating and bit operations of the fixed-width integers.
//! They have no declaration in the standard modules, so their signature lines and doc comments
//! are written here, from `spec/14 §2`, `spec/18 §4`, `§5` and `spec/21 §2`. The names come from
//! the checker (`una_check::builtin_methods`), and tests keep the two in step: every method the
//! checker resolves has an entry and every entry is one it resolves, and each entry's signature
//! is probed against `una check` in every edition, with a positive and a negative call.
//!
//! The listing is `docs/std/builtins.json`, beside `docs/std/listing.json`, with the same item
//! shape under the unstable schema `una.std-builtins/1` (`docs/std/README.md`). Operators and
//! interpolation are not methods and are not listed (`spec/14 §2`).

use serde::Serialize;
use una_core::IntType;
use una_project::Edition;

use crate::std_listing::{Item, Variant, cited};

/// The schema of the built-in listing: unstable, not part of the specification.
pub const STD_BUILTINS_SCHEMA: &str = "una.std-builtins/1";

/// One built-in method on one receiver type.
#[derive(Clone, Debug)]
pub struct Builtin {
    /// The method's name, as written after the dot.
    pub name: &'static str,
    /// The receiver's type, with `T`, `K` and `V` for type arguments and `F` for any
    /// fixed-width integer type.
    pub receiver: &'static str,
    /// The receiver's name in the signature line.
    pub var: &'static str,
    /// Whether the call updates the receiver in place, written `edit m.set(...)`.
    pub edit: bool,
    /// Whether it is written without parentheses, as `.len` is.
    pub property: bool,
    /// The labeled arguments after the receiver, as `(label, type)`.
    pub params: Vec<(&'static str, &'static str)>,
    /// The result's type.
    pub result: String,
    /// The first edition that has it.
    pub since: Edition,
}

impl Builtin {
    /// The signature line, as `m.get(key: K) -> Option[V]`, and the receiver's type.
    pub fn declaration(&self) -> String {
        let call = if self.property {
            format!("{}.{}", self.var, self.name)
        } else {
            let args: Vec<String> = self
                .params
                .iter()
                .map(|(label, ty)| format!("{label}: {ty}"))
                .collect();
            format!("{}.{}({})", self.var, self.name, args.join(", "))
        };
        let edit = if self.edit { "edit " } else { "" };
        let receiver = if self.receiver == "F" {
            format!("F, one of {}", fixed_types().join(", "))
        } else {
            self.receiver.to_owned()
        };
        format!(
            "{edit}{call} -> {}\n  where {}: {receiver}",
            self.result, self.var
        )
    }
}

/// The fixed-width integer types, in the order of `spec/18 §1`.
fn fixed_types() -> Vec<&'static str> {
    IntType::ALL.iter().map(|t| t.as_str()).collect()
}

/// Every built-in method on every receiver, in the order of the reference: `.len`, the
/// methods of maps, the conversions, then the fixed-width operations.
pub fn builtins() -> Vec<Builtin> {
    let entry = |name: &'static str,
                 receiver: &'static str,
                 var: &'static str,
                 params: &[(&'static str, &'static str)],
                 result: &str| {
        Builtin {
            name,
            receiver,
            var,
            edit: false,
            property: false,
            params: params.to_vec(),
            result: result.to_owned(),
            since: Edition::V0_2,
        }
    };
    let len = |receiver, var, since| Builtin {
        property: true,
        since,
        ..entry("len", receiver, var, &[], "Int")
    };
    let mut all = vec![
        len("Text", "t", Edition::V0_1),
        len("List[T]", "xs", Edition::V0_1),
        len("Map[K, V]", "m", Edition::V0_2),
        entry("has", "Map[K, V]", "m", &[("key", "K")], "Bool"),
        entry("get", "Map[K, V]", "m", &[("key", "K")], "Option[V]"),
        Builtin {
            edit: true,
            ..entry(
                "set",
                "Map[K, V]",
                "m",
                &[("key", "K"), ("value", "V")],
                "Unit",
            )
        },
        Builtin {
            edit: true,
            ..entry("remove", "Map[K, V]", "m", &[("key", "K")], "Unit")
        },
        entry(
            "with",
            "Map[K, V]",
            "m",
            &[("key", "K"), ("value", "V")],
            "Map[K, V]",
        ),
        entry("without", "Map[K, V]", "m", &[("key", "K")], "Map[K, V]"),
        entry("keys", "Map[K, V]", "m", &[], "List[K]"),
        entry("values", "Map[K, V]", "m", &[], "List[V]"),
        entry("entries", "Map[K, V]", "m", &[], "List[Entry[K, V]]"),
        // Edition 0.4 (ADR 0119 rule 18).
        Builtin {
            edit: true,
            since: Edition::V0_4,
            ..entry(
                "update",
                "Map[K, V]",
                "m",
                &[("key", "K"), ("initial", "V"), ("change", "fn(V) -> V")],
                "Unit",
            )
        },
    ];
    for target in IntType::ALL {
        let name: &'static str = conversion_name(target);
        all.push(entry(name, "Int", "x", &[], target.as_str()));
        all.push(entry(name, "F", "x", &[], target.as_str()));
    }
    all.push(entry("to_int", "Int", "x", &[], "Int"));
    all.push(entry("to_int", "F", "x", &[], "Int"));
    for name in ["wrapping_add", "wrapping_sub", "wrapping_mul"] {
        all.push(entry(name, "F", "a", &[("by", "F")], "F"));
    }
    for name in ["saturating_add", "saturating_sub", "saturating_mul"] {
        all.push(entry(name, "F", "a", &[("by", "F")], "F"));
    }
    for name in ["bit_and", "bit_or", "bit_xor"] {
        all.push(entry(name, "F", "a", &[("mask", "F")], "F"));
    }
    all.push(entry("bit_not", "F", "a", &[], "F"));
    all.push(entry("shift_right", "F", "a", &[("by", "Int")], "F"));
    all.push(entry(
        "wrapping_shift_left",
        "F",
        "a",
        &[("by", "Int")],
        "F",
    ));
    all
}

/// `to_i8` and the other conversions, by target.
fn conversion_name(target: IntType) -> &'static str {
    match target {
        IntType::I8 => "to_i8",
        IntType::I16 => "to_i16",
        IntType::I32 => "to_i32",
        IntType::I64 => "to_i64",
        IntType::U8 => "to_u8",
        IntType::U16 => "to_u16",
        IntType::U32 => "to_u32",
        IntType::U64 => "to_u64",
    }
}

/// The doc comment of each built-in, by name: it paraphrases and cites the specification.
pub fn builtin_docs(name: &str) -> &'static [&'static str] {
    const NARROW: &[&str] = &[
        "Converts `x` to that type. A widening conversion, where every value of the source type",
        "fits the target, always succeeds; a narrowing one carries an `overflow` obligation that",
        "the value fits, or goes through `checked`, which counts against the escape budget",
        "(`spec/18 §4`).",
    ];
    match name {
        "len" => &[
            "Built in on `Text` and `List` and, from edition 0.2 on, on `Map` (`spec/14 §2`,",
            "`spec/21 §2`). On a `Text` it counts Unicode scalar values (`spec/14 §1`); a map's is",
            "never negative (`spec/21 §2`).",
        ],
        "has" => &[
            "Whether `m` has `key`. Like `m.len`, it may appear in `requires`, `ensures`, `where`",
            "and predicates (`spec/21 §2`, `spec/21 §4`).",
        ],
        "get" => &[
            "`Some` exactly when `m.has(key: k)`. A missing key gives `None`, not a failure",
            "(`spec/21 §2`).",
        ],
        "set" => &[
            "Updates `m` in place, written `edit m.set(...)`: afterwards `m.has(key: k)` and",
            "`m.get(key: k) == Some(v)`, and `m.len` grows by one when the key was not there",
            "(`spec/21 §2`).",
        ],
        "remove" => &[
            "Updates `m` in place, written `edit m.remove(...)`: afterwards `not m.has(key: k)`,",
            "and `m.len` shrinks by one when the key was there (`spec/21 §2`).",
        ],
        "with" => &["A map with the contract of `set`, for the result (`spec/21 §2`)."],
        "without" => &["A map with the contract of `remove`, for the result (`spec/21 §2`)."],
        "keys" => &["The keys of `m`, in key order, as many as `m.len` (`spec/21 §2`)."],
        "values" => {
            &["The values of `m`, in the order of their keys, as many as `m.len` (`spec/21 §2`)."]
        }
        "entries" => &[
            "The entries of `m` as `Entry` values, in key order, as many as `m.len`",
            "(`spec/21 §2`).",
        ],
        "update" => &[
            "Updates `m` in place, written `edit m.update(...)`: sets `key` to `change(v)`, where",
            "`v` is its value, or `initial` when `m` has no `key`. `change` is called exactly once,",
            "and when it fails, `m` keeps its value from before the call (`spec/21 §2`).",
        ],
        "to_int" => &[
            "Converts `x` to `Int`, which every value of a fixed-width type fits, so it always",
            "succeeds (`spec/18 §4`). On a `Text`, `to_int` is the prelude function (`spec/14 §1`).",
        ],
        "wrapping_add" | "wrapping_sub" | "wrapping_mul" => &[
            "Total: the exact result reduced into the type's range modulo `2^w`, where `w` is",
            "its width (`spec/18 §5`).",
        ],
        "saturating_add" | "saturating_sub" | "saturating_mul" => {
            &["Total: the exact result clamped to the type's range (`spec/18 §5`)."]
        }
        "bit_and" => &[
            "Total; on an unsigned type the result is at most `a` and at most `mask`",
            "(`spec/18 §5`).",
        ],
        "bit_or" => &[
            "Total; on an unsigned type the result is at least `a` and at least `mask`",
            "(`spec/18 §5`).",
        ],
        "bit_xor" | "bit_not" => {
            &["Total; its result is only known to be in the type's range (`spec/18 §5`)."]
        }
        "shift_right" => &[
            "Requires `0 <= by` and `by` below the width; on an unsigned type the result is at",
            "most `a`. On a signed type it divides by `2^by`, rounding toward negative infinity",
            "(`spec/18 §5`).",
        ],
        "wrapping_shift_left" => &[
            "Requires `0 <= by` and `by` below the width: `a * 2^by` reduced into the type's",
            "range modulo `2^w`. There is no checked left shift (`spec/18 §5`).",
        ],
        n if n.starts_with("to_") => NARROW,
        _ => &[],
    }
}

/// The listing, in its key order.
#[derive(Debug, Serialize)]
struct Listing {
    schema: &'static str,
    note: &'static str,
    items: Vec<Item>,
}

/// The built-ins as listing items, one per name, with one variant per run of editions in
/// which the set of receivers is the same.
pub fn std_builtin_items() -> Vec<Item> {
    let all = builtins();
    let mut names: Vec<&'static str> = Vec::new();
    for builtin in &all {
        if !names.contains(&builtin.name) {
            names.push(builtin.name);
        }
    }
    names
        .into_iter()
        .map(|name| {
            let docs: Vec<String> = builtin_docs(name).iter().map(|l| (*l).to_owned()).collect();
            let mut variants: Vec<Variant> = Vec::new();
            for edition in Edition::ALL {
                let lines: Vec<String> = all
                    .iter()
                    .filter(|b| b.name == name && b.since <= edition)
                    .map(Builtin::declaration)
                    .collect();
                if lines.is_empty() {
                    continue;
                }
                let declaration = lines.join("\n");
                match variants.last_mut() {
                    Some(last) if last.declaration == declaration => {
                        last.editions.push(edition.name());
                    }
                    _ => variants.push(Variant {
                        editions: vec![edition.name()],
                        declaration,
                        docs: docs.clone(),
                    }),
                }
            }
            Item {
                name: name.to_owned(),
                module: "builtin",
                kind: "method",
                spec: cited(&variants),
                variants,
            }
        })
        .collect()
}

/// The built-in listing as `docs/std/builtins.json` holds it: pretty JSON, newline-terminated.
pub fn std_builtins() -> String {
    let listing = Listing {
        schema: STD_BUILTINS_SCHEMA,
        note: "Unstable, not part of the specification: see docs/std/README.md. Operators and interpolation are in spec/14 §2.",
        items: std_builtin_items(),
    };
    match serde_json::to_string_pretty(&listing) {
        Ok(text) => format!("{text}\n"),
        // INVARIANT: the listing holds only strings and lists, which always serialize.
        Err(error) => format!("{{\"error\": \"{error}\"}}\n"),
    }
}
