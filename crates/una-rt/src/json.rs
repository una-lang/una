//! JSON in native code (`spec/23`, ADR 0073): the parser and the writer are `una_core::json`,
//! shared with the interpreter; this converts its trees to and from heap words, iteratively, so
//! no value's depth recurses on the native stack (ADR 0060).

use una_core::bigint::BigInt;
use una_core::json::{self, Halt, Tree};

use crate::host;
use crate::runtime;
use crate::values::{
    data, field, int_word, list, list_items, map_from_sorted, map_words, push, release, tag,
    text_value, text_word,
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
    /// An object's entries so far, keys and values owned, in the order the parser sorted them,
    /// built into its map in bulk once the object ends (#886).
    Object(
        std::vec::IntoIter<(String, Tree)>,
        Vec<(i64, i64)>,
        Option<i64>,
    ),
}

/// The `Json` word of `tree`, owned by the caller, asking `within` every
/// [`json::CHECK_EVERY`] values whether the run is within its memory budget: `None` when it is
/// not, with what was built so far released.
fn word(tree: Tree, within: &mut dyn FnMut() -> bool) -> Option<i64> {
    let mut stack: Vec<Building> = Vec::new();
    let mut pending = Some(tree);
    let mut ticks = 0;
    loop {
        let mut done = None;
        if let Some(mut tree) = pending.take() {
            ticks += 1;
            if ticks >= json::CHECK_EVERY {
                ticks = 0;
                if !within() {
                    for building in stack {
                        match building {
                            Building::Array(_, out) => release(out),
                            Building::Object(_, entries, key) => {
                                for (key, value) in entries {
                                    release(key);
                                    release(value);
                                }
                                if let Some(key) = key {
                                    release(key);
                                }
                            }
                        }
                    }
                    return None;
                }
            }
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
        loop {
            if let Some(value) = done.take() {
                match stack.last_mut() {
                    None => return Some(value),
                    Some(Building::Array(_, out)) => {
                        push(*out, value);
                        release(value);
                    }
                    Some(Building::Object(_, out, key)) => match key.take() {
                        // The entry takes over both words.
                        Some(key) => out.push((key, value)),
                        None => release(value),
                    },
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
                    // Sorted and unique, as the parser gives them: built in bulk, in time linear
                    // in the object's size (spec/23 §6 rule 1).
                    Some(Building::Object(_, out, _)) => {
                        done = Some(wrap(OBJECT, map_from_sorted(out)));
                    }
                    None => return Some((NULL << 1) | 1),
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

/// The tree of a `Json` word, which stays borrowed, asking `within` every
/// [`json::CHECK_EVERY`] values whether the run is within its memory budget: `None` when it is
/// not, or when a large `Int` does not fit it, the run then aborting at `site`.
fn tree(word: i64, within: &mut dyn FnMut() -> bool, site: i64) -> Option<Tree> {
    let mut stack: Vec<Walking> = Vec::new();
    let mut pending = Some(word);
    let mut ticks = 0;
    // The bytes of text copied into the tree since the last check, of numbers, strings and keys:
    // every 64 KiB of them is a check too, so many mid-size numbers and strings cannot pass the hard ceiling between two
    // checks of values (`spec/11 §7`, `spec/23 §3` rule 6, #1223).
    let mut copied = 0usize;
    loop {
        let mut done = None;
        if let Some(word) = pending.take() {
            ticks += 1;
            if ticks >= json::CHECK_EVERY {
                ticks = 0;
                if !within() {
                    return None;
                }
            }
            done = match tag(word) {
                BOOLEAN => Some(Tree::Bool(field(word, 0) != 0)),
                // A large `Int` is written once the run has room for its text (`spec/11 §7`,
                // #1223): otherwise the call aborts with U6003, as over the budget.
                NUMBER => match crate::render::int_written(field(word, 0)) {
                    Some(digits) => Some(Tree::Int(digits)),
                    None => {
                        runtime::write_over_budget(site);
                        return None;
                    }
                },
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
                if let Tree::Int(text) | Tree::Decimal(text) | Tree::Str(text) = &tree {
                    copied = copied.saturating_add(text.len());
                    if copied >= una_core::INT_TEXT_RESERVED_FROM {
                        copied = 0;
                        if !within() {
                            return None;
                        }
                    }
                }
                match stack.last_mut() {
                    None => return Some(tree),
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
                    // A key's text is copied too, and counted with the others.
                    if let Some(key) = &key {
                        copied = copied.saturating_add(key.len());
                        if copied >= una_core::INT_TEXT_RESERVED_FROM {
                            copied = 0;
                            if !within() {
                                return None;
                            }
                        }
                    }
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
                    None => return Some(Tree::Null),
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
        json::Error::NumberTooLong(at) => (3, at),
    };
    wrap(variant, int_word(&BigInt::from(at)))
}

/// `parse_json(t)` at `site`: the `Json` of `t`, or a `JsonError` failure, which
/// `una_rt_host_failed` then reports (`spec/23 §3`). Over the memory budget it aborts at
/// `site` with U6003, as a loop iteration does, and [`json_stopped`] says so.
pub fn parse_json(t: i64, site: i64) -> i64 {
    let mut within = || runtime::budget(usize::try_from(site).unwrap_or(usize::MAX)) == 0;
    let built = match json::parse_within(&text_value(t), &mut within) {
        Ok(tree) => word(tree, &mut within),
        Err(Halt::Invalid(failure)) => return host::failed(error(failure)),
        Err(Halt::OverBudget) => None,
    };
    built.unwrap_or_else(stopped)
}

/// `to_json(j)` at `site`: the canonical text of `j` (`spec/23 §4`), with the memory budget
/// checked as [`parse_json`] checks it.
pub fn to_json(j: i64, site: i64) -> i64 {
    let mut within = || runtime::budget(usize::try_from(site).unwrap_or(usize::MAX)) == 0;
    let text = tree(j, &mut within, site).and_then(|tree| json::write_within(&tree, &mut within));
    text.map_or_else(stopped, text_word)
}

thread_local! {
    /// Whether the last `parse_json` or `to_json` of this thread stopped over the budget.
    static STOPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Exchanges the thread's [`STOPPED`] with a parked task's (`sched`).
pub(crate) fn swap_stopped(stopped: &mut bool) {
    *stopped = STOPPED.with(|cell| cell.replace(*stopped));
}

/// The value of a JSON call that stopped over the budget: `Null`, which the task's stop
/// discards, with [`json_stopped`] set.
fn stopped() -> i64 {
    STOPPED.with(|cell| cell.set(true));
    (NULL << 1) | 1
}

/// 1 when the last `parse_json` or `to_json` of this thread stopped over the memory budget,
/// and the task must stop, as after [`runtime::budget`]; the flag is then cleared.
pub fn json_stopped() -> i64 {
    i64::from(STOPPED.with(|cell| cell.replace(false)))
}

/// `quote_json(t)`: `t` as a JSON string literal (`spec/23 §4` rule 2).
pub fn quote_json(t: i64) -> i64 {
    text_word(json::quote(&text_value(t)))
}

/// `is_json_number(t)`: 1 when `t` is one JSON number, else 0 (`spec/23 §1`).
pub fn is_json_number(t: i64) -> i64 {
    i64::from(json::is_json_number(&text_value(t)))
}
