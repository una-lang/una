//! JSON in the interpreter (`spec/23`, ADR 0073): the parser and the writer are `una_core::json`,
//! shared with the native runtime; this converts its trees to and from values, iteratively, so
//! no value's depth recurses on the native stack (ADR 0060).

use std::rc::Rc;

use una_core::bigint::BigInt;
use una_core::json::{self, Tree};

use crate::machine::prelude_ctor;
use crate::value::{Entries, Key, Text, Value};

/// The variants of `Json`, in declaration order (`spec/23 §1`).
const NULL: usize = 0;
const BOOLEAN: usize = 1;
const NUMBER: usize = 2;
const DECIMAL: usize = 3;
const STR: usize = 4;
const ARRAY: usize = 5;
const OBJECT: usize = 6;
const VARIANTS: [&str; 7] = [
    "Null", "Boolean", "Number", "Decimal", "Str", "Array", "Object",
];

fn json<'p>(index: usize, fields: Vec<Value<'p>>) -> Value<'p> {
    let name = VARIANTS.get(index).copied().unwrap_or("Null");
    Value::data(prelude_ctor("Json", name, index), fields)
}

/// `Decimal(text)`, a JSON number with a fraction or an exponent (`spec/23 §1`).
pub(crate) fn decimal<'p>(text: &str) -> Value<'p> {
    json(DECIMAL, vec![Value::text(text)])
}

/// `Number(n)`, a JSON number with neither a fraction nor an exponent (`spec/23 §1`).
pub(crate) fn number<'p>(n: BigInt) -> Value<'p> {
    json(NUMBER, vec![Value::Int(n)])
}

/// The failure of `parse_json` for `error` (`spec/23 §3` rule 5).
pub(crate) fn error<'p>(error: json::Error) -> Value<'p> {
    let (name, index, at) = match error {
        json::Error::Syntax(at) => ("Syntax", 0, at),
        json::Error::TooDeep(at) => ("TooDeep", 1, at),
        json::Error::DuplicateKey(at) => ("DuplicateKey", 2, at),
        json::Error::NumberTooLong(at) => ("NumberTooLong", 3, at),
    };
    Value::data(
        prelude_ctor("JsonError", name, index),
        vec![Value::Int(BigInt::from(at))],
    )
}

/// An array or an object whose children are being converted.
enum Building<'p> {
    Array(std::vec::IntoIter<Tree>, Vec<Value<'p>>),
    /// An object's entries so far, in the order the parser sorted them, built into its map in
    /// bulk once the object ends (#886).
    Object(
        std::vec::IntoIter<(String, Tree)>,
        Vec<(Key, (Value<'p>, Value<'p>))>,
        Option<Rc<Text>>,
    ),
}

/// The `Json` value of `tree`, asking `within` every [`json::CHECK_EVERY`] values whether the
/// run is within its memory budget: `None` when it is not.
pub(crate) fn value<'p>(tree: Tree, within: &mut dyn FnMut() -> bool) -> Option<Value<'p>> {
    let mut stack: Vec<Building<'p>> = Vec::new();
    let mut pending = Some(tree);
    let mut ticks = 0;
    loop {
        let mut done = None;
        if let Some(mut tree) = pending.take() {
            ticks += 1;
            if ticks >= json::CHECK_EVERY {
                ticks = 0;
                if !within() {
                    return None;
                }
            }
            done = match &mut tree {
                Tree::Null => Some(json(NULL, Vec::new())),
                Tree::Bool(b) => Some(json(BOOLEAN, vec![Value::Bool(*b)])),
                Tree::Int(digits) => Some(json(
                    NUMBER,
                    vec![Value::Int(digits.parse().unwrap_or_default())],
                )),
                Tree::Decimal(text) => Some(json(DECIMAL, vec![Value::text(text)])),
                Tree::Str(text) => Some(json(STR, vec![Value::text(text)])),
                Tree::Array(items) => {
                    let items = std::mem::take(items);
                    let count = items.len();
                    stack.push(Building::Array(
                        items.into_iter(),
                        Vec::with_capacity(count),
                    ));
                    None
                }
                Tree::Object(entries) => {
                    let entries = std::mem::take(entries);
                    let count = entries.len();
                    stack.push(Building::Object(
                        entries.into_iter(),
                        Vec::with_capacity(count),
                        None,
                    ));
                    None
                }
            };
        }
        // A finished value goes to the array or object being built; a finished container is
        // in turn a finished value.
        loop {
            if let Some(value) = done.take() {
                match stack.last_mut() {
                    None => return Some(value),
                    Some(Building::Array(_, out)) => out.push(value),
                    Some(Building::Object(_, out, key)) => {
                        if let Some(key) = key.take() {
                            out.push((Key::Text(Rc::clone(&key)), (Value::Text(key), value)));
                        }
                    }
                }
            }
            let next = match stack.last_mut() {
                None => None,
                Some(Building::Array(items, _)) => items.next().map(|tree| (None, tree)),
                Some(Building::Object(entries, _, _)) => {
                    entries.next().map(|(key, tree)| (Some(key), tree))
                }
            };
            match next {
                Some((key, tree)) => {
                    if let (Some(key), Some(Building::Object(_, _, slot))) = (key, stack.last_mut())
                    {
                        *slot = Some(Rc::new(Text::new(key)));
                    }
                    pending = Some(tree);
                    break;
                }
                None => {
                    done = match stack.pop() {
                        Some(Building::Array(_, out)) => {
                            Some(json(ARRAY, vec![Value::List(Rc::new(out))]))
                        }
                        // The parser gave the entries sorted and without duplicate keys, so the
                        // map is built in bulk, in time linear in its size (spec/23 §6 rule 1).
                        Some(Building::Object(_, out, _)) => Some(json(
                            OBJECT,
                            vec![Value::Map(Rc::new(Entries::from_iter(out)))],
                        )),
                        None => None,
                    };
                    if done.is_none() {
                        // Unreachable: the root returns above.
                        return Some(json(NULL, Vec::new()));
                    }
                }
            }
        }
    }
}

/// The children of a `Json` value whose tree is being built.
enum Walking<'v, 'p> {
    Array(std::slice::Iter<'v, Value<'p>>, Vec<Tree>),
    Object(
        std::collections::btree_map::Values<'v, Key, (Value<'p>, Value<'p>)>,
        Vec<(String, Tree)>,
        Option<String>,
    ),
}

/// Why [`tree`] made no tree.
pub(crate) enum NoTree {
    /// The value is not a `Json`.
    NotJson,
    /// The run is over its memory budget.
    OverBudget,
}

/// The tree of a `Json` value, asking `within` every [`json::CHECK_EVERY`] values whether the
/// run is within its memory budget, and `room` before a large `Int` is written.
pub(crate) fn tree(
    value: &Value<'_>,
    within: &mut dyn FnMut() -> bool,
    room: crate::machine::Room<'_>,
) -> Result<Tree, NoTree> {
    let mut stack: Vec<Walking<'_, '_>> = Vec::new();
    let mut pending = Some(value);
    let mut ticks = 0;
    // The bytes of text copied into the tree since the last check: every 64 KiB of them is a
    // check too, so many mid-size numbers and strings cannot pass the hard ceiling between two
    // checks of values (`spec/11 §7`, `spec/23 §3` rule 6, #1223).
    let mut copied = 0usize;
    loop {
        let mut done = None;
        if let Some(value) = pending.take() {
            ticks += 1;
            if ticks >= json::CHECK_EVERY {
                ticks = 0;
                if !within() {
                    return Err(NoTree::OverBudget);
                }
            }
            let Value::Data(data) = value else {
                return Err(NoTree::NotJson);
            };
            let field = data.fields.first();
            done = match (data.ctor.index, field) {
                (NULL, _) => Some(Tree::Null),
                (BOOLEAN, Some(Value::Bool(b))) => Some(Tree::Bool(*b)),
                // A large `Int` is written once its text fits the budget (`spec/11 §7`).
                (NUMBER, Some(Value::Int(n))) => {
                    if una_core::int_text_room(n).is_some_and(|bytes| !room(bytes)) {
                        return Err(NoTree::OverBudget);
                    }
                    Some(Tree::Int(n.to_string()))
                }
                (DECIMAL, Some(Value::Text(t))) => Some(Tree::Decimal(t.to_string())),
                (STR, Some(Value::Text(t))) => Some(Tree::Str(t.to_string())),
                (ARRAY, Some(Value::List(items))) => {
                    stack.push(Walking::Array(
                        items.iter(),
                        Vec::with_capacity(items.len()),
                    ));
                    None
                }
                (OBJECT, Some(Value::Map(entries))) => {
                    stack.push(Walking::Object(
                        entries.values(),
                        Vec::with_capacity(entries.len()),
                        None,
                    ));
                    None
                }
                _ => return Err(NoTree::NotJson),
            };
        }
        loop {
            if let Some(tree) = done.take() {
                if let Tree::Int(text) | Tree::Decimal(text) | Tree::Str(text) = &tree {
                    copied = copied.saturating_add(text.len());
                    if copied >= una_core::INT_TEXT_RESERVED_FROM {
                        copied = 0;
                        if !within() {
                            return Err(NoTree::OverBudget);
                        }
                    }
                }
                match stack.last_mut() {
                    None => return Ok(tree),
                    Some(Walking::Array(_, out)) => out.push(tree),
                    Some(Walking::Object(_, out, key)) => {
                        out.push((key.take().unwrap_or_default(), tree));
                    }
                }
            }
            let next = match stack.last_mut() {
                None => None,
                Some(Walking::Array(items, _)) => items.next().map(|item| (None, item)),
                Some(Walking::Object(entries, _, _)) => entries.next().map(|(key, item)| {
                    let key = match key {
                        Value::Text(t) => t.to_string(),
                        _ => String::new(),
                    };
                    (Some(key), item)
                }),
            };
            match next {
                Some((key, item)) => {
                    if let (Some(key), Some(Walking::Object(_, _, slot))) = (key, stack.last_mut())
                    {
                        *slot = Some(key);
                    }
                    pending = Some(item);
                    break;
                }
                None => {
                    done = match stack.pop() {
                        Some(Walking::Array(_, out)) => Some(Tree::Array(out)),
                        Some(Walking::Object(_, out, _)) => Some(Tree::Object(out)),
                        None => return Err(NoTree::NotJson),
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::value::KEY_COMPARISONS;

    #[test]
    fn an_object_with_keys_in_order_is_built_with_comparisons_linear_in_its_size() {
        // #886, spec/23 §6 rule 1: the parser gives an object's keys sorted, and its map is
        // built in bulk, a fixed number of comparisons per key at every size; inserting them
        // one by one takes about log2 of the size per key, 16 at 2^16.
        for k in [1_usize << 12, 1 << 14, 1 << 16] {
            let entries = (0..k)
                .map(|i| (format!("key{i:08}"), Tree::Int(i.to_string())))
                .collect();
            let before = KEY_COMPARISONS.with(std::cell::Cell::get);
            let built = value(Tree::Object(entries), &mut || true).unwrap();
            let compared = KEY_COMPARISONS.with(std::cell::Cell::get) - before;
            assert!(compared <= 3 * k, "{k} keys: {compared} comparisons");
            let size = match &built {
                Value::Data(json) => match json.fields.first() {
                    Some(Value::Map(map)) => Some(map.len()),
                    _ => None,
                },
                _ => None,
            };
            assert_eq!(size, Some(k), "an Object of {k} entries");
        }
    }
}
