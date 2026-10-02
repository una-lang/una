//! Runtime values (`spec/11 §2`).

use std::cmp::Ordering;
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
}

/// A constructor and its fields.
#[derive(Clone, Debug)]
pub(crate) struct Data<'p> {
    /// The constructor.
    pub(crate) ctor: Ctor,
    /// The fields, in order.
    pub(crate) fields: Vec<Value<'p>>,
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
pub(crate) fn equal(a: &Value<'_>, b: &Value<'_>) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => x == y,
        (Value::Unit, Value::Unit) => true,
        (Value::Hashed(x), Value::Hashed(y)) => x == y,
        (Value::Data(x), Value::Data(y)) => {
            x.ctor.kind == y.ctor.kind
                && x.ctor.index == y.ctor.index
                && x.fields.len() == y.fields.len()
                && x.fields.iter().zip(&y.fields).all(|(a, b)| equal(a, b))
        }
        (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| equal(a, b))
        }
        _ => false,
    }
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
