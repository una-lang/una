//! Runtime values (`spec/11 §2`).

use std::cmp::Ordering;
use std::rc::Rc;

use num_bigint::BigInt;
use una_core::{Ctor, Expr, Name};

use crate::machine::{Caps, Env, Instance};

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

// A value can be deeper than the call depth bounds, since `fold` builds it without calls, so
// releasing a value never recurses on the native stack, which would overflow (S-30): records
// and function values release what only they hold by one worklist.
impl Drop for Data<'_> {
    fn drop(&mut self) {
        unwind(std::mem::take(&mut self.fields), Vec::new());
    }
}

// A function value holds the values it closes over and the handlers of its capabilities, whose
// arguments are values too.
impl Drop for Closure<'_> {
    fn drop(&mut self) {
        let bindings = std::mem::take(&mut self.env);
        let values = bindings.into_iter().map(|(_, value)| value).collect();
        unwind(values, vec![std::mem::take(&mut self.caps)]);
    }
}

/// Drops `pending` and the capabilities in `caps`, with everything only they hold, by a worklist
/// instead of recursion. A value or a capability held elsewhere too is only released.
fn unwind<'p>(mut pending: Vec<Value<'p>>, mut caps: Vec<Caps<'p>>) {
    loop {
        if let Some(value) = pending.pop() {
            match value {
                Value::Data(data) => {
                    if let Ok(mut data) = Rc::try_unwrap(data) {
                        pending.append(&mut data.fields);
                    }
                }
                Value::List(items) => {
                    if let Ok(items) = Rc::try_unwrap(items) {
                        pending.extend(items);
                    }
                }
                Value::Closure(closure) => {
                    if let Ok(mut closure) = Rc::try_unwrap(closure) {
                        let bindings = std::mem::take(&mut closure.env);
                        pending.extend(bindings.into_iter().map(|(_, value)| value));
                        caps.push(std::mem::take(&mut closure.caps));
                    }
                }
                Value::Int(_)
                | Value::Bool(_)
                | Value::Text(_)
                | Value::Unit
                | Value::Hashed(_) => {}
            }
        } else if let Some(held) = caps.pop() {
            if let Ok(held) = Rc::try_unwrap(held) {
                for (_, instance) in held {
                    if let Ok(Instance::Handled {
                        values,
                        caps: inner,
                        ..
                    }) = Rc::try_unwrap(instance)
                    {
                        pending.extend(values);
                        caps.push(inner);
                    }
                }
            }
        } else {
            return;
        }
    }
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
        (Value::List(x), Value::List(y)) => {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_deep_chain_of_records_and_function_values_drops_without_native_recursion() {
        // S-30: a chain of records, of records holding function values that close over the
        // previous link, and of function values alone, as `fold` builds them, each drop by the
        // worklist on a small stack.
        let dropped = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let mut sources = una_syntax::SourceDb::new();
                let file = sources.add("chain.una", b"");
                let body: &'static Expr = Box::leak(Box::new(Expr::Atom {
                    atom: una_core::Atom::Unit,
                    span: una_syntax::Span::new(file, 0, 0),
                }));
                let closing = |value: Value<'static>| {
                    Value::Closure(Rc::new(Closure {
                        params: Vec::new(),
                        body,
                        env: vec![("prev", value)],
                        caps: Rc::default(),
                    }))
                };
                let node = Ctor {
                    kind: una_core::Path {
                        module: "chain".to_owned(),
                        name: "Link".to_owned(),
                    },
                    name: "Next".to_owned(),
                    index: 1,
                };
                let mut records = Value::Unit;
                for _ in 0..100_000 {
                    records = Value::data(node.clone(), vec![records]);
                }
                drop(records);
                let mut through_records = Value::Unit;
                for _ in 0..100_000 {
                    let closure = closing(through_records);
                    through_records = Value::data(node.clone(), vec![closure]);
                }
                drop(through_records);
                let mut through_closures = Value::Unit;
                for _ in 0..100_000 {
                    through_closures = closing(through_closures);
                }
                drop(through_closures);
                true
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(dropped);
    }
}
