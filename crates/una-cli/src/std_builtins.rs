//! The built-in methods of the standard library reference (#690): `.len`, the methods of maps,
//! of `Stack`, of `Queue`, of sets, of hash maps and hash sets, of priority queues and of bit
//! sets with their `.size`, the conversions and the wrapping, saturating and bit operations of
//! the fixed-width integers. They have no declaration in the standard modules, so their
//! signature lines and doc comments are written here, from `spec/14 §2`, `spec/18 §4`, `§5`,
//! `spec/21 §2`, `spec/28 §5`, `§6`, `§7`, `§8` and `§9`. The names come from
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
/// methods of maps, of `Stack` and of `Queue`, those of sets, of hash maps, of hash sets, of
/// priority queues and of bit sets, the conversions, then the fixed-width operations.
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
        // Edition 0.5 (ADR 0119 rule 18).
        Builtin {
            edit: true,
            since: Edition::V0_5,
            ..entry(
                "update",
                "Map[K, V]",
                "m",
                &[("key", "K"), ("initial", "V"), ("change", "fn(V) -> V")],
                "Unit",
            )
        },
    ];
    // Edition 0.5: the methods of `Stack` and `Queue` (`spec/28 §5`), in the order of their
    // tables.
    let seq =
        |name, receiver, var, edit, params: &[(&'static str, &'static str)], result| Builtin {
            edit,
            since: Edition::V0_5,
            ..entry(name, receiver, var, params, result)
        };
    all.extend([
        seq("push", "Stack[T]", "s", true, &[("item", "T")], "Unit"),
        seq("pop", "Stack[T]", "s", true, &[], "Option[T]"),
        seq("peek", "Stack[T]", "s", false, &[], "Option[T]"),
        seq("items", "Stack[T]", "s", false, &[], "List[T]"),
        seq("push", "Queue[T]", "q", true, &[("item", "T")], "Unit"),
        seq("pop", "Queue[T]", "q", true, &[], "Option[T]"),
        seq("peek", "Queue[T]", "q", false, &[], "Option[T]"),
        seq(
            "push_front",
            "Queue[T]",
            "q",
            true,
            &[("item", "T")],
            "Unit",
        ),
        seq("pop_back", "Queue[T]", "q", true, &[], "Option[T]"),
        seq("peek_back", "Queue[T]", "q", false, &[], "Option[T]"),
        seq("items", "Queue[T]", "q", false, &[], "List[T]"),
    ]);
    // Edition 0.5: the methods of `Set` (`spec/28 §6`, ADR 0119 rule 7).
    let set = |name, params: &[(&'static str, &'static str)], result: &str| Builtin {
        since: Edition::V0_5,
        ..entry(name, "Set[T]", "s", params, result)
    };
    let set_update = |name| Builtin {
        edit: true,
        ..set(name, &[("item", "T")], "Unit")
    };
    all.extend([
        len("Set[T]", "s", Edition::V0_5),
        set("has", &[("item", "T")], "Bool"),
        set_update("add"),
        set_update("remove"),
        set("with", &[("item", "T")], "Set[T]"),
        set("without", &[("item", "T")], "Set[T]"),
        set("first", &[], "Option[T]"),
        set("last", &[], "Option[T]"),
        set("between", &[("low", "T"), ("high", "T")], "List[T]"),
        set("items", &[], "List[T]"),
        set("union", &[("other", "Set[T]")], "Set[T]"),
        set("intersection", &[("other", "Set[T]")], "Set[T]"),
        set("difference", &[("other", "Set[T]")], "Set[T]"),
        set("is_subset", &[("of", "Set[T]")], "Bool"),
    ]);
    // Edition 0.5: the methods of `HashMap` and `HashSet` (`spec/28 §7`, ADR 0119 rule 12).
    let hash_map = |name, params: &[(&'static str, &'static str)], result: &str| Builtin {
        since: Edition::V0_5,
        ..entry(name, "HashMap[K, V]", "m", params, result)
    };
    let hash_set = |name, params: &[(&'static str, &'static str)], result: &str| Builtin {
        since: Edition::V0_5,
        ..entry(name, "HashSet[T]", "s", params, result)
    };
    all.extend([
        len("HashMap[K, V]", "m", Edition::V0_5),
        hash_map("has", &[("key", "K")], "Bool"),
        hash_map("get", &[("key", "K")], "Option[V]"),
        Builtin {
            edit: true,
            ..hash_map("set", &[("key", "K"), ("value", "V")], "Unit")
        },
        Builtin {
            edit: true,
            ..hash_map("remove", &[("key", "K")], "Unit")
        },
        hash_map("with", &[("key", "K"), ("value", "V")], "HashMap[K, V]"),
        hash_map("without", &[("key", "K")], "HashMap[K, V]"),
        Builtin {
            edit: true,
            ..hash_map(
                "update",
                &[("key", "K"), ("initial", "V"), ("change", "fn(V) -> V")],
                "Unit",
            )
        },
        hash_map("keys", &[], "List[K]"),
        hash_map("values", &[], "List[V]"),
        hash_map("entries", &[], "List[Entry[K, V]]"),
        hash_map("same_entries", &[("as", "HashMap[K, V]")], "Bool"),
        len("HashSet[T]", "s", Edition::V0_5),
        hash_set("has", &[("item", "T")], "Bool"),
        Builtin {
            edit: true,
            ..hash_set("add", &[("item", "T")], "Unit")
        },
        Builtin {
            edit: true,
            ..hash_set("remove", &[("item", "T")], "Unit")
        },
        hash_set("with", &[("item", "T")], "HashSet[T]"),
        hash_set("without", &[("item", "T")], "HashSet[T]"),
        hash_set("items", &[], "List[T]"),
        hash_set("union", &[("other", "HashSet[T]")], "HashSet[T]"),
        hash_set("intersection", &[("other", "HashSet[T]")], "HashSet[T]"),
        hash_set("difference", &[("other", "HashSet[T]")], "HashSet[T]"),
        hash_set("is_subset", &[("of", "HashSet[T]")], "Bool"),
        hash_set("same_items", &[("as", "HashSet[T]")], "Bool"),
    ]);
    // Edition 0.5: the methods of `PriorityQueue` (`spec/28 §8`, ADR 0119 rule 13), in the
    // order of its table.
    let priority = |name, edit, params: &[(&'static str, &'static str)], result| Builtin {
        edit,
        since: Edition::V0_5,
        ..entry(name, "PriorityQueue[T, P]", "q", params, result)
    };
    all.extend([
        priority("push", true, &[("item", "T"), ("priority", "P")], "Unit"),
        priority("pop", true, &[], "Option[T]"),
        priority("pop_ranked", true, &[], "Option[Ranked[T, P]]"),
        priority("peek", false, &[], "Option[T]"),
        priority("peek_priority", false, &[], "Option[P]"),
        priority("items", false, &[], "List[T]"),
    ]);
    // Edition 0.5: `.size` and the methods of `BitSet` (`spec/28 §9`, ADR 0119 rule 14).
    let bits = |name, params: &[(&'static str, &'static str)], result: &str| Builtin {
        since: Edition::V0_5,
        ..entry(name, "BitSet", "b", params, result)
    };
    let bits_update = |name| Builtin {
        edit: true,
        ..bits(name, &[("item", "Int")], "Unit")
    };
    all.extend([
        len("BitSet", "b", Edition::V0_5),
        Builtin {
            property: true,
            ..bits("size", &[], "Int")
        },
        bits("has", &[("item", "Int")], "Bool"),
        bits_update("add"),
        bits_update("remove"),
        bits("with", &[("item", "Int")], "BitSet"),
        bits("without", &[("item", "Int")], "BitSet"),
        bits("union", &[("other", "BitSet")], "BitSet"),
        bits("intersection", &[("other", "BitSet")], "BitSet"),
        bits("difference", &[("other", "BitSet")], "BitSet"),
        bits("is_subset", &[("of", "BitSet")], "Bool"),
        bits("first", &[], "Option[Int]"),
        bits("last", &[], "Option[Int]"),
        bits("items", &[], "List[Int]"),
    ]);
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
            "never negative (`spec/21 §2`). From edition 0.5 on, a `Stack`, a `Queue`, a `Set`, a",
            "`HashMap`, a `HashSet` and a `PriorityQueue` count their items, and a `BitSet` its",
            "members, in O(1) (`spec/28 §5`, `§6`, `§7`, `§8`, `§9`).",
        ],
        "has" => &[
            "Whether `m` has `key`. Like `m.len`, it may appear in `requires`, `ensures`, `where`",
            "and predicates (`spec/21 §2`, `spec/21 §4`). On a `Set`, from edition 0.5 on, whether",
            "`s` holds `item`, in O(log n) (`spec/28 §6`). On a `HashMap` and a `HashSet`, the same",
            "in O(|k|) expected, outside contracts in edition 0.5 (`spec/28 §7`). On a `BitSet`,",
            "whether `item` is a member, `false` outside its range, in O(1) (`spec/28 §9`).",
        ],
        "get" => &[
            "`Some` exactly when `m.has(key: k)`. A missing key gives `None`, not a failure",
            "(`spec/21 §2`). On a `HashMap`, in O(|k|) expected (`spec/28 §7`).",
        ],
        "set" => &[
            "Updates `m` in place, written `edit m.set(...)`: afterwards `m.has(key: k)` and",
            "`m.get(key: k) == Some(v)`, and `m.len` grows by one when the key was not there",
            "(`spec/21 §2`). On a `HashMap`, a key already there keeps its place in the listing,",
            "in O(|k|) expected, amortized (`spec/28 §7`).",
        ],
        "remove" => &[
            "Updates `m` in place, written `edit m.remove(...)`: afterwards `not m.has(key: k)`,",
            "and `m.len` shrinks by one when the key was there (`spec/21 §2`). On a `Set`, written",
            "`edit s.remove(...)`, afterwards `not s.has(item: x)`, in O(log n) (`spec/28 §6`). On a",
            "`HashMap` and a `HashSet`, the same in O(|k|) expected, amortized (`spec/28 §7`). On a",
            "`BitSet`, in O(1), and an item outside its range changes nothing (`spec/28 §9`).",
        ],
        "with" => &[
            "A map with the contract of `set`, for the result (`spec/21 §2`). On a `Set`, a set",
            "with the contract of `add`, for the result (`spec/28 §6`); on a `HashMap` and a",
            "`HashSet`, the same for the hash table (`spec/28 §7`); on a `BitSet`, the same for the",
            "bit set (`spec/28 §9`).",
        ],
        "without" => &[
            "A map with the contract of `remove`, for the result (`spec/21 §2`). On a `Set`, a set",
            "with the contract of `remove`, for the result (`spec/28 §6`); on a `HashMap` and a",
            "`HashSet`, the same for the hash table (`spec/28 §7`); on a `BitSet`, the same for the",
            "bit set (`spec/28 §9`).",
        ],
        "keys" => &[
            "The keys of `m`, in key order, as many as `m.len` (`spec/21 §2`). On a `HashMap`, in",
            "the order the keys were first added, in O(n) (`spec/28 §7`).",
        ],
        "values" => &[
            "The values of `m`, in the order of their keys, as many as `m.len` (`spec/21 §2`). On",
            "a `HashMap`, in the order the keys were first added, in O(n) (`spec/28 §7`).",
        ],
        "size" => &[
            "The number of bits of a `BitSet`, fixed when `empty_bit_set` makes it: its members",
            "are integers from 0 to `size - 1`. O(1) (`spec/28 §9`).",
        ],
        "entries" => &[
            "The entries of `m` as `Entry` values, in key order, as many as `m.len`",
            "(`spec/21 §2`). On a `HashMap`, in the order the keys were first added, in O(n)",
            "(`spec/28 §7`).",
        ],
        "same_entries" => &[
            "Whether `m` and `as` have the same keys, each with an equal value, whatever the order",
            "they were added in; `V` must satisfy `Eq`. A hash map has no `==`. O(n · |k|)",
            "expected (`spec/28 §7`).",
        ],
        "same_items" => &[
            "Whether `s` and `as` hold the same items, whatever the order they were added in. A",
            "hash set has no `==`. O(n · |k|) expected (`spec/28 §7`).",
        ],
        "update" => &[
            "Updates `m` in place, written `edit m.update(...)`: sets `key` to `change(v)`, where",
            "`v` is its value, or `initial` when `m` has no `key`. `change` is called exactly once,",
            "and when it fails, `m` keeps its value from before the call (`spec/21 §2`). On a",
            "`HashMap`, the same in O(|k|) expected, amortized (`spec/28 §7`).",
        ],
        "push" => &[
            "Updates the receiver in place, written `edit s.push(item: x)`: a `Stack` takes",
            "`item` on top and a `Queue` at its back, and its length grows by one, in O(1)",
            "amortized time (`spec/28 §5`). A `PriorityQueue`, written",
            "`edit q.push(item: x, priority: p)`, takes `item` at `priority`, in O(log n)",
            "comparisons, amortized (`spec/28 §8`).",
        ],
        "pop" => &[
            "Updates the receiver in place, written `const x = edit s.pop()`: it removes the top of",
            "a `Stack` or the front of a `Queue` and gives it as `Some`, or gives `None` and",
            "leaves an empty one as it is, in O(1) amortized time (`spec/28 §5`). A",
            "`PriorityQueue` gives its item of the first priority, the one pushed first among",
            "equal priorities, in O(log n) comparisons, amortized (`spec/28 §8`).",
        ],
        "peek" => &[
            "The top of a `Stack` or the front of a `Queue`, the item `pop` would remove, as",
            "`Some`, and `None` exactly when it is empty, in O(1) time (`spec/28 §5`). On a",
            "`PriorityQueue`, the item `pop` would give, in O(1) time (`spec/28 §8`).",
        ],
        "pop_ranked" => &[
            "Updates a `PriorityQueue` in place, written `const r = edit q.pop_ranked()`: as `pop`,",
            "with the priority, as `Some` of a `Ranked`, in O(log n) comparisons, amortized",
            "(`spec/28 §8`).",
        ],
        "peek_priority" => &[
            "The priority of the item that `pop` would give from a `PriorityQueue`, as `Some`,",
            "and `None` exactly when it is empty, in O(1) time (`spec/28 §8`).",
        ],
        "items" => &[
            "The items as a list: a `Stack` top first and a `Queue` front first, as many as its",
            "`len`, in O(n) time (`spec/28 §5`); a `Set`'s in ascending order, as many as its",
            "`len`, in O(n) time (`spec/28 §6`); a `HashSet`'s in the order its items were first",
            "added, in O(n) time (`spec/28 §7`); a `PriorityQueue`'s in the order of the pops, in",
            "O(n log n) comparisons (`spec/28 §8`); a `BitSet`'s members in ascending order, in",
            "O(size / 64 + len) time (`spec/28 §9`).",
        ],
        "push_front" => &[
            "Updates a `Queue` in place, written `edit q.push_front(item: x)`: `item` goes to the",
            "front, so `peek` gives it, in O(1) amortized time (`spec/28 §5`).",
        ],
        "pop_back" => &[
            "Updates a `Queue` in place, written `const x = edit q.pop_back()`: it removes the back",
            "and gives it as `Some`, or gives `None` when the queue is empty, in O(1) amortized",
            "time (`spec/28 §5`).",
        ],
        "peek_back" => &[
            "The back of a `Queue`, the item `pop_back` would remove, as `Some`, and `None`",
            "exactly when it is empty, in O(1) time (`spec/28 §5`).",
        ],
        "add" => &[
            "Updates `s` in place, written `edit s.add(...)`: afterwards `s.has(item: x)`, and",
            "`s.len` grows by one when the item was not there. O(log n) (`spec/28 §6`); on a",
            "`HashSet`, O(|k|) expected, amortized (`spec/28 §7`). On a `BitSet`, O(1), and it",
            "requires `0 <= item and item < b.size` (`spec/28 §9`).",
        ],
        "first" => &[
            "The least item of `s`, or `None` when it is empty. O(log n) (`spec/28 §6`), and",
            "O(size / 64) on a `BitSet` (`spec/28 §9`).",
        ],
        "last" => &[
            "The greatest item of `s`, or `None` when it is empty. O(log n) (`spec/28 §6`), and",
            "O(size / 64) on a `BitSet` (`spec/28 §9`).",
        ],
        "between" => &[
            "The items of `s` from `low` to `high`, both included, in ascending order; none when",
            "`low` is above `high`. O(log n + k) for the k items listed (`spec/28 §6`).",
        ],
        "union" => &[
            "The items of `s` and those of `other`. O(n + m), or less when `other` is much smaller",
            "(`spec/28 §6`). On a `HashSet`, the receiver's items, then the other's it lacks, in",
            "the other's order (`spec/28 §7`). On a `BitSet`, `other` has its size, in",
            "O(size / 64) (`spec/28 §9`).",
        ],
        "intersection" => &[
            "The items of `s` that `other` holds. O(n + m), or less when one of the sets is much",
            "smaller (`spec/28 §6`). On a `HashSet`, in the receiver's order (`spec/28 §7`). On a",
            "`BitSet`, `other` has its size, in O(size / 64) (`spec/28 §9`).",
        ],
        "difference" => &[
            "The items of `s` that `other` does not hold. O(n + m), or less when `other` is much",
            "smaller (`spec/28 §6`). On a `HashSet`, in the receiver's order (`spec/28 §7`). On a",
            "`BitSet`, `other` has its size, in O(size / 64) (`spec/28 §9`).",
        ],
        "is_subset" => &[
            "Whether `of` holds every item of `s`. O(n + m), or less when `s` is much smaller",
            "(`spec/28 §6`). On a `HashSet`, O(n · |k|) expected (`spec/28 §7`). On a `BitSet`,",
            "`of` has its size, in O(size / 64) (`spec/28 §9`).",
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
