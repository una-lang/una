//! JSON in the interpreter (`spec/23`, ADR 0073): the parser and the writer are `una_core::json`,
//! shared with the native runtime; this converts its trees to and from values, iteratively, so
//! no value's depth recurses on the native stack (ADR 0060).

use std::rc::Rc;

use num_bigint::BigInt;
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
    Object(
        std::vec::IntoIter<(String, Tree)>,
        Entries<'p>,
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
                    stack.push(Building::Object(entries.into_iter(), Entries::new(), None));
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
                            out.insert(Key::Text(Rc::clone(&key)), (Value::Text(key), value));
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
                        Some(Building::Object(_, out, _)) => {
                            Some(json(OBJECT, vec![Value::Map(Rc::new(out))]))
                        }
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
    /// A `Number` holds an `Int` of more than `MAX_INT_DIGITS` digits, which is never written
    /// (`spec/11 §7`, #923).
    IntTooLong,
}

/// The tree of a `Json` value, asking `within` every [`json::CHECK_EVERY`] values whether the
/// run is within its memory budget.
pub(crate) fn tree(value: &Value<'_>, within: &mut dyn FnMut() -> bool) -> Result<Tree, NoTree> {
    let mut stack: Vec<Walking<'_, '_>> = Vec::new();
    let mut pending = Some(value);
    let mut ticks = 0;
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
                (NUMBER, Some(Value::Int(n))) => {
                    Some(Tree::Int(una_core::int_text(n).ok_or(NoTree::IntTooLong)?))
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
