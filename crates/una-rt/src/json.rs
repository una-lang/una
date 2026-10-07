//! JSON in native code (`spec/23`, ADR 0073): the parser and the writer are `una_core::json`,
//! shared with the interpreter; this converts its trees to and from heap words, iteratively, so
//! no value's depth recurses on the native stack (ADR 0060).

use num_bigint::BigInt;
use una_core::json::{self, Tree};

use crate::host;
use crate::values::{
    data, field, int_value, int_word, list, list_items, map_empty, map_with_owned, map_words, push,
    release, tag, text_value, text_word,
};

/// The variants of `Json`, in declaration order (`spec/23 §1`).
const NULL: i64 = 0;
const BOOLEAN: i64 = 1;
const NUMBER: i64 = 2;
const DECIMAL: i64 = 3;
const STR: i64 = 4;
const ARRAY: i64 = 5;
const OBJECT: i64 = 6;

/// A variant of `Json` with one field, which it takes over: the caller gives up `value`.
fn wrap(variant: i64, value: i64) -> i64 {
    let word = data(variant);
    push(word, value);
    release(value);
    word
}

/// An array or an object whose children are being converted.
enum Building {
    Array(std::vec::IntoIter<Tree>, i64),
    Object(std::vec::IntoIter<(String, Tree)>, i64, Option<i64>),
}

/// The `Json` word of `tree`, owned by the caller.
fn word(tree: Tree) -> i64 {
    let mut stack: Vec<Building> = Vec::new();
    let mut pending = Some(tree);
    loop {
        let mut done = None;
        if let Some(mut tree) = pending.take() {
            done = match &mut tree {
                Tree::Null => Some((NULL << 1) | 1),
                Tree::Bool(b) => Some(wrap(BOOLEAN, i64::from(*b))),
                Tree::Int(digits) => {
                    let n: BigInt = digits.parse().unwrap_or_default();
                    Some(wrap(NUMBER, int_word(&n)))
                }
                Tree::Decimal(text) => Some(wrap(DECIMAL, text_word(std::mem::take(text)))),
                Tree::Str(text) => Some(wrap(STR, text_word(std::mem::take(text)))),
                Tree::Array(items) => {
                    let items = std::mem::take(items);
                    stack.push(Building::Array(items.into_iter(), list()));
                    None
                }
                Tree::Object(entries) => {
                    let entries = std::mem::take(entries);
                    stack.push(Building::Object(entries.into_iter(), map_empty(), None));
                    None
                }
            };
        }
        loop {
            if let Some(value) = done.take() {
                match stack.last_mut() {
                    None => return value,
                    Some(Building::Array(_, out)) => {
                        push(*out, value);
                        release(value);
                    }
                    Some(Building::Object(_, out, key)) => {
                        if let Some(key) = key.take() {
                            *out = map_with_owned(*out, key, value);
                            release(key);
                        }
                        release(value);
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
                        *slot = Some(text_word(key));
                    }
                    pending = Some(tree);
                    break;
                }
                None => match stack.pop() {
                    Some(Building::Array(_, out)) => done = Some(wrap(ARRAY, out)),
                    Some(Building::Object(_, out, _)) => done = Some(wrap(OBJECT, out)),
                    None => return (NULL << 1) | 1,
                },
            }
        }
    }
}

/// The children of a `Json` word whose tree is being built.
enum Walking {
    Array(std::vec::IntoIter<i64>, Vec<Tree>),
    Object(
        std::vec::IntoIter<(i64, i64)>,
        Vec<(String, Tree)>,
        Option<String>,
    ),
}

/// The tree of a `Json` word, which stays borrowed.
fn tree(word: i64) -> Tree {
    let mut stack: Vec<Walking> = Vec::new();
    let mut pending = Some(word);
    loop {
        let mut done = None;
        if let Some(word) = pending.take() {
            done = match tag(word) {
                BOOLEAN => Some(Tree::Bool(field(word, 0) != 0)),
                NUMBER => Some(Tree::Int(int_value(field(word, 0)).to_string())),
                DECIMAL => Some(Tree::Decimal(text_value(field(word, 0)))),
                STR => Some(Tree::Str(text_value(field(word, 0)))),
                ARRAY => {
                    let items = list_items(field(word, 0));
                    let count = items.len();
                    stack.push(Walking::Array(items.into_iter(), Vec::with_capacity(count)));
                    None
                }
                OBJECT => {
                    let entries = map_words(field(word, 0));
                    let count = entries.len();
                    stack.push(Walking::Object(
                        entries.into_iter(),
                        Vec::with_capacity(count),
                        None,
                    ));
                    None
                }
                _ => Some(Tree::Null),
            };
        }
        loop {
            if let Some(tree) = done.take() {
                match stack.last_mut() {
                    None => return tree,
                    Some(Walking::Array(_, out)) => out.push(tree),
                    Some(Walking::Object(_, out, key)) => {
                        out.push((key.take().unwrap_or_default(), tree));
                    }
                }
            }
            let next = match stack.last_mut() {
                None => None,
                Some(Walking::Array(items, _)) => items.next().map(|item| (None, item)),
                Some(Walking::Object(entries, _, _)) => entries
                    .next()
                    .map(|(key, item)| (Some(text_value(key)), item)),
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
                None => match stack.pop() {
                    Some(Walking::Array(_, out)) => done = Some(Tree::Array(out)),
                    Some(Walking::Object(_, out, _)) => done = Some(Tree::Object(out)),
                    None => return Tree::Null,
                },
            }
        }
    }
}

/// The failure word of `parse_json` for `error` (`spec/23 §3` rule 5).
fn error(error: json::Error) -> i64 {
    let (variant, at) = match error {
        json::Error::Syntax(at) => (0, at),
        json::Error::TooDeep(at) => (1, at),
        json::Error::DuplicateKey(at) => (2, at),
    };
    wrap(variant, int_word(&BigInt::from(at)))
}

/// `parse_json(t)`: the `Json` of `t`, or a `JsonError` failure, which `una_rt_host_failed`
/// then reports (`spec/23 §3`).
pub fn parse_json(t: i64) -> i64 {
    match json::parse(&text_value(t)) {
        Ok(tree) => word(tree),
        Err(failure) => host::failed(error(failure)),
    }
}

/// `to_json(j)`: the canonical text of `j` (`spec/23 §4`).
pub fn to_json(j: i64) -> i64 {
    text_word(json::write(&tree(j)))
}

/// `quote_json(t)`: `t` as a JSON string literal (`spec/23 §4` rule 2).
pub fn quote_json(t: i64) -> i64 {
    text_word(json::quote(&text_value(t)))
}

/// `is_json_number(t)`: 1 when `t` is one JSON number, else 0 (`spec/23 §1`).
pub fn is_json_number(t: i64) -> i64 {
    i64::from(json::is_json_number(&text_value(t)))
}
