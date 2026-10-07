//! Runtime values (`spec/11 §2`).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::rc::Rc;

use num_bigint::BigInt;
use una_core::{Ctor, Expr, Name};

use crate::machine::{Caps, Env};

/// A value of the Una Abstract Machine; `'p` is the program's lifetime.
#[derive(Clone, Debug)]
pub(crate) enum Value<'p> {
    /// An arbitrary-precision integer (ADR 0004).
    Int(BigInt),
    /// A boolean.
    Bool(bool),
    /// A text.
    Text(Rc<str>),
    /// `unit`.
    Unit,
    /// A constructor applied to its fields.
    Data(Rc<Data<'p>>),
    /// A list.
    List(Rc<Vec<Value<'p>>>),
    /// A closure `⟨λ, ρ⟩`.
    Closure(Rc<Closure<'p>>),
    /// A keyed hash.
    Hashed(Rc<[u8]>),
    /// A tuple, internal to the desugaring of the memory model (`spec/17 §4`).
    Tuple(Rc<Vec<Value<'p>>>),
    /// A map, in the order of its keys (`spec/21 §5`): each entry keeps its key as a value,
    /// for listing and rendering.
    Map(Rc<Entries<'p>>),
}

/// The entries of a map, by the order of their keys.
pub(crate) type Entries<'p> = BTreeMap<Key, (Value<'p>, Value<'p>)>;

/// A map's key, in the order of `spec/03 §1` and `spec/20 §5`: an integer, a text by its
/// scalar values, or a variant without fields by its declaration. A map holds keys of one
/// type, so the order across these never applies.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Int(BigInt),
    Text(Rc<str>),
    Variant(usize),
}

impl Key {
    /// The key of a value of an `Ord` type, `None` for any other value.
    pub(crate) fn of(value: &Value<'_>) -> Option<Key> {
        match value {
            Value::Int(n) => Some(Key::Int(n.clone())),
            Value::Text(t) => Some(Key::Text(t.clone())),
            Value::Data(data) if data.fields.is_empty() => Some(Key::Variant(data.ctor.index)),
            _ => None,
        }
    }
}

/// A constructor and its fields.
#[derive(Clone, Debug)]
pub(crate) struct Data<'p> {
    /// The constructor.
    pub(crate) ctor: Ctor,
    /// The fields, in order.
    pub(crate) fields: Vec<Value<'p>>,
}

// A value can be deeper than the call depth bounds, since a loop builds it without calls
// (`spec/17 §4`), so dropping and comparing values never recurse on the native stack, which
// would overflow (ADR 0032).
impl Drop for Data<'_> {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.fields);
        while let Some(value) = pending.pop() {
            match value {
                Value::Data(data) => {
                    if let Ok(mut data) = Rc::try_unwrap(data) {
                        pending.append(&mut data.fields);
                    }
                }
                Value::List(items) | Value::Tuple(items) => {
                    if let Ok(items) = Rc::try_unwrap(items) {
                        pending.extend(items);
                    }
                }
                Value::Map(entries) => {
                    if let Ok(entries) = Rc::try_unwrap(entries) {
                        for (_, (key, value)) in entries {
                            pending.push(key);
                            pending.push(value);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// A lambda with the environment it closes over.
#[derive(Clone, Debug)]
pub(crate) struct Closure<'p> {
    /// The parameters.
    pub(crate) params: Vec<&'p Name>,
    /// The body.
    pub(crate) body: &'p Expr,
    /// The environment at the lambda.
    pub(crate) env: Env<'p>,
    /// The capabilities in scope at the lambda.
    pub(crate) caps: Caps<'p>,
}

impl<'p> Value<'p> {
    /// A constructor value.
    pub(crate) fn data(ctor: Ctor, fields: Vec<Value<'p>>) -> Value<'p> {
        Value::Data(Rc::new(Data { ctor, fields }))
    }

    /// A text value.
    pub(crate) fn text(text: &str) -> Value<'p> {
        Value::Text(Rc::from(text))
    }

    /// An integer value.
    pub(crate) fn int(value: i64) -> Value<'p> {
        Value::Int(BigInt::from(value))
    }
}

/// Structural equality: `==` of `spec/03 §1`. Functions are never compared (the checker
/// rejects it), and compare unequal.
/// It compares with a worklist, so a deep value does not recurse on the native stack.
pub(crate) fn equal(a: &Value<'_>, b: &Value<'_>) -> bool {
    let mut pending = vec![(a, b)];
    while let Some(pair) = pending.pop() {
        let same = match pair {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Text(x), Value::Text(y)) => x == y,
            (Value::Unit, Value::Unit) => true,
            (Value::Hashed(x), Value::Hashed(y)) => x == y,
            (Value::Data(x), Value::Data(y)) => {
                let same = x.ctor.kind == y.ctor.kind
                    && x.ctor.index == y.ctor.index
                    && x.fields.len() == y.fields.len();
                pending.extend(x.fields.iter().zip(&y.fields));
                same
            }
            (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
                pending.extend(x.iter().zip(y.iter()));
                x.len() == y.len()
            }
            // The same keys, each with an equal value (`spec/21 §1` rule 5).
            (Value::Map(x), Value::Map(y)) => {
                let same = x.len() == y.len() && x.keys().eq(y.keys());
                pending.extend(x.values().zip(y.values()).map(|((_, a), (_, b))| (a, b)));
                same
            }
            _ => false,
        };
        if !same {
            return false;
        }
    }
    true
}

/// The order of `<`: integers, texts by scalar values, and variants without fields by
/// declaration (`spec/03 §1`).
pub(crate) fn compare(a: &Value<'_>, b: &Value<'_>) -> Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Data(x), Value::Data(y)) => x.ctor.index.cmp(&y.ctor.index),
        _ => Ordering::Equal,
    }
}
