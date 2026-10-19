//! Dense values in the interpreter (`spec/29 §17`, ADR 0120 rules 26 to 47): construction,
//! views, element-wise functions, reductions and products, by the reference chains of
//! `una_core::dense`, so the interpreter gives the bits of every other implementation.
//!
//! Every buffer is checked against the memory budget before it is allocated, in checked
//! arithmetic, and allocated fallibly: a refused one is U6003, never an abort of the process
//! (ADR 0120, Security). A view is O(1) and shares its parent's buffer; an update is in place
//! when the value and its buffer are held once, and copies the view's own elements first
//! otherwise (rule 31). Every shape a kernel ensures is checked before it returns (rule 53).

use std::rc::Rc;

use num_bigint::BigInt;
use una_core::dense as chains;
use una_core::float;
use una_syntax::Span;

use super::{Abort, Flow, Machine, Stop, fault, none, precondition, some};
use crate::value::{Dense, DenseKind, Elements, Value};

/// Whether the prelude function `name` is a constructor or a method of dense values.
pub(super) fn is_dense(name: &str) -> bool {
    matches!(
        name,
        "array" | "array_filled" | "matrix" | "matrix_filled" | "matrix_diagonal"
    ) || name.starts_with("dense_")
}

/// The methods of dense values this engine runs, the name after `dense_array_` or
/// `dense_matrix_`: a prelude function of another name is a fault, never another method's
/// computation, and a test checks that every prelude function is one of these.
pub(super) const METHODS: [&str; 34] = [
    "get",
    "at",
    "set",
    "slice",
    "row",
    "column",
    "block",
    "transpose",
    "compact",
    "to_list",
    "map",
    "zip_map",
    "update",
    "fold",
    "multiply_elements",
    "divide_elements",
    "sum",
    "dot",
    "norm2",
    "all_finite",
    "sum_exact",
    "dot_exact",
    "row_sums",
    "column_sums",
    "multiply",
    "multiply_array",
    "add_product",
    "add_scaled",
    "scale",
    // Constructors, by their own names.
    "array",
    "array_filled",
    "matrix",
    "matrix_filled",
    "matrix_diagonal",
];

/// The bytes a buffer holds per element: a raw `F64`, or a value.
fn element_bytes(f64s: bool) -> usize {
    if f64s {
        std::mem::size_of::<f64>()
    } else {
        std::mem::size_of::<Value<'_>>()
    }
}

/// The fault of a dense method's arguments of the wrong types, which checking excludes.
fn bad<'p>(name: &str) -> Stop<'p> {
    fault(format!("`{name}` applied to values of the wrong types"))
}

/// A dimension argument, an `Int` that the callee's precondition keeps at least 0, as a
/// `usize`. A dimension beyond what this machine can index is refused by the memory budget,
/// U6003, whatever the element count: one rule for every shape, so a dimension the program
/// reads back is always the exact one it gave.
fn dimension<'p>(value: Option<&Value<'p>>, name: &str, over: Stop<'p>) -> Result<usize, Stop<'p>> {
    match value {
        Some(Value::Int(n)) if n.sign() == num_bigint::Sign::Minus => Err(bad(name)),
        Some(Value::Int(n)) => match usize::try_from(n) {
            Ok(n) if isize::try_from(n).is_ok() => Ok(n),
            _ => Err(over),
        },
        _ => Err(bad(name)),
    }
}

/// An index argument as a `usize`, `None` when it is negative or beyond any length.
fn index(value: Option<&Value<'_>>) -> Option<usize> {
    match value {
        Some(Value::Int(n)) => usize::try_from(n).ok(),
        _ => None,
    }
}

/// The dense value of an argument.
fn dense<'a, 'p>(value: Option<&'a Value<'p>>, name: &str) -> Result<&'a Rc<Dense<'p>>, Stop<'p>> {
    match value {
        Some(Value::Dense(dense)) => Ok(dense),
        _ => Err(bad(name)),
    }
}

/// An `F64` argument.
fn scalar<'p>(value: Option<&Value<'p>>, name: &str) -> Result<f64, Stop<'p>> {
    match value {
        Some(Value::F64(x)) => Ok(*x),
        _ => Err(bad(name)),
    }
}

/// A contiguous row-major dense value of `shape` over `elements`.
fn compact<'p>(kind: DenseKind, shape: Vec<usize>, elements: Elements<'p>) -> Value<'p> {
    let strides = row_major(&shape);
    Value::Dense(Rc::new(Dense {
        kind,
        buffer: Rc::new(elements),
        offset: 0,
        shape,
        strides,
    }))
}

/// The strides of a contiguous row-major value of `shape`.
fn row_major(shape: &[usize]) -> Vec<usize> {
    let mut strides = vec![1; shape.len()];
    let mut step = 1_usize;
    for (stride, length) in strides.iter_mut().zip(shape).rev() {
        *stride = step;
        step = step.saturating_mul(*length);
    }
    strides
}

impl<'p> Dense<'p> {
    /// The positions in the buffer of the elements, in row-major order, computed as they are
    /// read: no allocation.
    fn positions(&self) -> impl Iterator<Item = usize> + Clone + '_ {
        (0..self.count()).filter_map(move |k| self.position_at(k))
    }

    /// The `F64` elements in row-major order, read in place: `None` when the buffer does not
    /// hold raw `F64` values, which checking excludes for the float methods. An array of no
    /// elements may hold values.
    fn f64_iter(&self) -> Option<impl Iterator<Item = f64> + Clone + '_> {
        let items: &[f64] = match self.buffer.as_ref() {
            Elements::F64(items) => items,
            Elements::Values(items) if self.count() == 0 || items.is_empty() => &[],
            Elements::Values(_) => return None,
        };
        Some(
            self.positions()
                .map(move |p| items.get(p).copied().unwrap_or(f64::NAN)),
        )
    }
}

/// The elements of a result being built, in the representation its first element decides,
/// within a buffer reserved for all of them before the first is stored.
enum Builder<'p> {
    /// Nothing stored yet; the number to come.
    Empty(usize),
    /// Raw `F64` values.
    F64(Vec<f64>),
    /// Any other values.
    Values(Vec<Value<'p>>),
}

impl<'p> Machine<'p, '_> {
    /// Whether a buffer of `elements` elements of `bytes` each fits the memory budget, with
    /// what the run holds already: checked before the allocation, in checked arithmetic, so a
    /// size whose computation overflows fails the check (ADR 0120, Security).
    fn reserve(&self, elements: usize, bytes: usize, span: Span) -> Result<(), Stop<'p>> {
        let budget = self.max_memory.saturating_mul(1024 * 1024);
        let held = (self.held)().saturating_sub(self.baseline);
        let over = Stop::Abort(Abort::Memory {
            span,
            limit: self.max_memory,
        });
        match elements
            .checked_mul(bytes)
            .and_then(|size| size.checked_add(held))
        {
            Some(total) if total <= budget => Ok(()),
            _ => Err(over),
        }
    }

    /// A vector of `n` elements, reserved within the budget and fallibly.
    fn buffer<T>(&self, n: usize, f64s: bool, span: Span) -> Result<Vec<T>, Stop<'p>> {
        self.reserve(n, element_bytes(f64s), span)?;
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| {
            Stop::Abort(Abort::Memory {
                span,
                limit: self.max_memory,
            })
        })?;
        Ok(out)
    }

    /// A buffer of the elements of `items`, raw when every one is an `F64`: reserved before
    /// any is copied.
    fn buffer_from(&self, items: &[Value<'p>], span: Span) -> Result<Elements<'p>, Stop<'p>> {
        let floats = !items.is_empty() && items.iter().all(|v| matches!(v, Value::F64(_)));
        if floats {
            let mut out = self.buffer(items.len(), true, span)?;
            out.extend(items.iter().filter_map(|v| match v {
                Value::F64(x) => Some(float::canonical(*x)),
                _ => None,
            }));
            Ok(Elements::F64(out))
        } else {
            let mut out = self.buffer(items.len(), false, span)?;
            out.extend(items.iter().cloned());
            Ok(Elements::Values(out))
        }
    }

    /// A compact copy of the elements of `value`, in its representation, reserved before any
    /// is copied: a copy on write, `compact`.
    fn copy(&self, value: &Dense<'p>, span: Span) -> Result<Elements<'p>, Stop<'p>> {
        let n = value.count();
        match value.buffer.as_ref() {
            Elements::F64(items) => {
                let mut out = self.buffer(n, true, span)?;
                out.extend(value.positions().filter_map(|p| items.get(p).copied()));
                Ok(Elements::F64(out))
            }
            Elements::Values(items) => {
                let mut out = self.buffer(n, false, span)?;
                out.extend(value.positions().filter_map(|p| items.get(p).cloned()));
                Ok(Elements::Values(out))
            }
        }
    }

    /// Stores `value` as the next element of `out`, reserving the whole buffer at the first.
    fn build(&self, out: &mut Builder<'p>, value: Value<'p>, span: Span) -> Result<(), Stop<'p>> {
        if let Builder::Empty(n) = *out {
            *out = if matches!(value, Value::F64(_)) {
                Builder::F64(self.buffer(n, true, span)?)
            } else {
                Builder::Values(self.buffer(n, false, span)?)
            };
        }
        match (out, value) {
            (Builder::F64(items), Value::F64(x)) => items.push(float::canonical(x)),
            (Builder::Values(items), value) => items.push(value),
            _ => return Err(bad("an element-wise function")),
        }
        Ok(())
    }

    /// `n` copies of `value`.
    fn filled(&self, n: usize, value: &Value<'p>, span: Span) -> Result<Elements<'p>, Stop<'p>> {
        match value {
            Value::F64(x) => {
                let mut out = self.buffer(n, true, span)?;
                out.resize(n, float::canonical(*x));
                Ok(Elements::F64(out))
            }
            other => {
                let mut out = self.buffer(n, false, span)?;
                out.resize(n, other.clone());
                Ok(Elements::Values(out))
            }
        }
    }

    /// A dense value of `shape` from the elements `out` built.
    fn built(kind: DenseKind, shape: Vec<usize>, out: Builder<'p>) -> Value<'p> {
        let elements = match out {
            Builder::Empty(_) => Elements::Values(Vec::new()),
            Builder::F64(items) => Elements::F64(items),
            Builder::Values(items) => Elements::Values(items),
        };
        compact(kind, shape, elements)
    }

    /// A dense value of `shape` from row-major `F64` values, reserved before any is computed.
    fn made_f64(
        &self,
        kind: DenseKind,
        shape: Vec<usize>,
        n: usize,
        values: impl Iterator<Item = Result<f64, Stop<'p>>>,
        span: Span,
    ) -> Flow<'p> {
        let mut out = self.buffer(n, true, span)?;
        for value in values {
            out.push(float::canonical(value?));
        }
        Ok(compact(kind, shape, Elements::F64(out)))
    }

    /// The memory refusal at `span`, U6003.
    fn over(&self, span: Span) -> Stop<'p> {
        Stop::Abort(Abort::Memory {
            span,
            limit: self.max_memory,
        })
    }

    /// `value`'s storage held once, so it can be updated in place: the value itself when it
    /// and its buffer are held once, and a compact copy of its own elements otherwise, after
    /// the budget check (rule 31).
    fn owned(&self, value: Rc<Dense<'p>>, span: Span) -> Result<Dense<'p>, Stop<'p>> {
        let mut owned = Rc::try_unwrap(value).unwrap_or_else(|shared| (*shared).clone());
        if Rc::get_mut(&mut owned.buffer).is_none() {
            let elements = self.copy(&owned, span)?;
            owned.strides = row_major(&owned.shape);
            owned.offset = 0;
            owned.buffer = Rc::new(elements);
        }
        Ok(owned)
    }

    /// The dense operation `name` of the prelude (`spec/29 §17`), one that [`is_dense`]
    /// accepts.
    pub(super) fn dense_call(&mut self, name: &str, args: Vec<Value<'p>>, span: Span) -> Flow<'p> {
        match name {
            "array" => self.array(&args, span),
            "array_filled" => self.array_filled(&args, span),
            "matrix" => self.matrix(&args, span),
            "matrix_filled" => self.matrix_filled(&args, span),
            "matrix_diagonal" => self.matrix_diagonal(&args, span),
            _ => match name
                .strip_prefix("dense_array_")
                .or_else(|| name.strip_prefix("dense_matrix_"))
            {
                Some(method) => self.method(method, args, span),
                None => Err(fault(format!("no dense function `{name}`"))),
            },
        }
    }

    fn array(&self, args: &[Value<'p>], span: Span) -> Flow<'p> {
        let Some(Value::List(items)) = args.first() else {
            return Err(bad("array"));
        };
        let elements = self.buffer_from(items, span)?;
        Ok(compact(DenseKind::Array, vec![items.len()], elements))
    }

    fn array_filled(&self, args: &[Value<'p>], span: Span) -> Flow<'p> {
        let n = dimension(args.first(), "array_filled", self.over(span))?;
        let value = args.get(1).ok_or_else(|| bad("array_filled"))?;
        let elements = self.filled(n, value, span)?;
        Ok(compact(DenseKind::Array, vec![n], elements))
    }

    fn matrix(&self, args: &[Value<'p>], span: Span) -> Flow<'p> {
        let rows = dimension(args.first(), "matrix", self.over(span))?;
        let columns = dimension(args.get(1), "matrix", self.over(span))?;
        let Some(Value::List(items)) = args.get(2) else {
            return Err(bad("matrix"));
        };
        if rows.checked_mul(columns) != Some(items.len()) {
            return Err(precondition(
                "`matrix` with a number of elements other than `rows * columns`, which its precondition excludes",
                span,
            ));
        }
        let elements = self.buffer_from(items, span)?;
        Ok(compact(DenseKind::Matrix, vec![rows, columns], elements))
    }

    fn matrix_filled(&self, args: &[Value<'p>], span: Span) -> Flow<'p> {
        let rows = dimension(args.first(), "matrix_filled", self.over(span))?;
        let columns = dimension(args.get(1), "matrix_filled", self.over(span))?;
        let value = args.get(2).ok_or_else(|| bad("matrix_filled"))?;
        let n = rows.checked_mul(columns).ok_or_else(|| self.over(span))?;
        let elements = self.filled(n, value, span)?;
        Ok(compact(DenseKind::Matrix, vec![rows, columns], elements))
    }

    fn matrix_diagonal(&self, args: &[Value<'p>], span: Span) -> Flow<'p> {
        let values = dense(args.first(), "matrix_diagonal")?;
        let n = values.count();
        let size = n.checked_mul(n).ok_or_else(|| self.over(span))?;
        // `+0` of the element type off the diagonal.
        let zero = match values.element(0) {
            Some(Value::F64(_)) | None => Value::F64(0.0),
            Some(_) => Value::Int(BigInt::from(0)),
        };
        let mut elements = self.filled(size, &zero, span)?;
        for (i, item) in (0..n).filter_map(|k| values.element(k)).enumerate() {
            let position = i * n + i;
            match (&mut elements, item) {
                (Elements::F64(out), Value::F64(x)) => {
                    if let Some(slot) = out.get_mut(position) {
                        *slot = x;
                    }
                }
                (Elements::Values(out), item) => {
                    if let Some(slot) = out.get_mut(position) {
                        *slot = item;
                    }
                }
                _ => return Err(bad("matrix_diagonal")),
            }
        }
        Ok(compact(DenseKind::Matrix, vec![n, n], elements))
    }

    /// A method of a dense value, its receiver first in `args`.
    fn method(&mut self, method: &str, args: Vec<Value<'p>>, span: Span) -> Flow<'p> {
        if !METHODS.contains(&method) {
            return Err(fault(format!("no dense method `{method}`")));
        }
        let receiver = dense(args.first(), method)?.clone();
        let indices = |from: usize| -> Option<Vec<usize>> {
            (from..from + receiver.shape.len())
                .map(|i| index(args.get(i)))
                .collect()
        };
        match method {
            "get" => {
                let position = indices(1).and_then(|at| receiver.position(&at));
                match position.and_then(|p| receiver.buffer.get(p)) {
                    Some(value) => Ok(value),
                    // Only a false proof reaches here: an internal abort (rule 30, V5).
                    None => Err(precondition(
                        "`get` at an index outside the dense value, which its precondition excludes",
                        span,
                    )),
                }
            }
            "at" => {
                let position = indices(1).and_then(|at| receiver.position(&at));
                Ok(match position.and_then(|p| receiver.buffer.get(p)) {
                    Some(value) => some(value),
                    None => none(),
                })
            }
            "set" => {
                let at = indices(1);
                let item = args
                    .get(1 + receiver.shape.len())
                    .cloned()
                    .ok_or_else(|| bad(method))?;
                drop(args);
                let mut owned = self.owned(receiver, span)?;
                let position = at.and_then(|at| owned.position(&at)).ok_or_else(|| {
                    precondition(
                        "`set` at an index outside the dense value, which its precondition excludes",
                        span,
                    )
                })?;
                let buffer = Rc::get_mut(&mut owned.buffer).ok_or_else(|| bad(method))?;
                match (buffer, item) {
                    (Elements::F64(items), Value::F64(x)) => {
                        if let Some(slot) = items.get_mut(position) {
                            *slot = float::canonical(x);
                        }
                    }
                    (Elements::Values(items), item) => {
                        if let Some(slot) = items.get_mut(position) {
                            *slot = item;
                        }
                    }
                    _ => return Err(bad(method)),
                }
                Ok(updated(owned))
            }
            "slice" => {
                let (from, until) = (index(args.get(1)), index(args.get(2)));
                let length = receiver.shape.first().copied().unwrap_or(0);
                let stride = receiver.strides.first().copied().unwrap_or(1);
                match (from, until) {
                    (Some(from), Some(until)) if from <= until && until <= length => {
                        let mut view = (*receiver).clone();
                        view.offset = view.offset.saturating_add(from.saturating_mul(stride));
                        view.shape = vec![until - from];
                        Ok(Value::Dense(Rc::new(view)))
                    }
                    _ => Err(precondition(
                        "`slice` outside the array, which its precondition excludes",
                        span,
                    )),
                }
            }
            "row" | "column" => {
                let axis = usize::from(method == "column");
                let other = 1 - axis;
                let i = index(args.get(1))
                    .filter(|i| receiver.shape.get(axis).is_some_and(|n| i < n))
                    .ok_or_else(|| {
                        precondition(
                            "a row or column outside the matrix, which its precondition excludes",
                            span,
                        )
                    })?;
                let stride = receiver.strides.get(axis).copied().unwrap_or(0);
                let mut view = (*receiver).clone();
                view.kind = DenseKind::Array;
                view.offset = view.offset.saturating_add(i.saturating_mul(stride));
                view.shape = vec![receiver.shape.get(other).copied().unwrap_or(0)];
                view.strides = vec![receiver.strides.get(other).copied().unwrap_or(0)];
                Ok(Value::Dense(Rc::new(view)))
            }
            "block" => {
                let bounds: Option<Vec<usize>> = (1..5).map(|i| index(args.get(i))).collect();
                let (rows, columns) = (
                    receiver.shape.first().copied().unwrap_or(0),
                    receiver.shape.get(1).copied().unwrap_or(0),
                );
                match bounds.as_deref() {
                    Some(&[i, j, k, l]) if i <= j && j <= rows && k <= l && l <= columns => {
                        let mut view = (*receiver).clone();
                        let at = receiver.position(&[i, k]).unwrap_or(receiver.offset);
                        // An empty block starts anywhere; its offset is never read.
                        view.offset = if i < rows && k < columns {
                            at
                        } else {
                            receiver.offset
                        };
                        view.shape = vec![j - i, l - k];
                        Ok(Value::Dense(Rc::new(view)))
                    }
                    _ => Err(precondition(
                        "`block` outside the matrix, which its precondition excludes",
                        span,
                    )),
                }
            }
            "transpose" => {
                let mut view = (*receiver).clone();
                view.shape.reverse();
                view.strides.reverse();
                Ok(Value::Dense(Rc::new(view)))
            }
            "compact" => {
                let elements = self.copy(&receiver, span)?;
                Ok(compact(receiver.kind, receiver.shape.clone(), elements))
            }
            "to_list" => {
                let mut out = self.buffer(receiver.count(), false, span)?;
                for k in 0..receiver.count() {
                    out.push(receiver.element(k).ok_or_else(|| bad(method))?);
                }
                Ok(Value::List(Rc::new(out)))
            }
            "map" => {
                let each = args.get(1).cloned().ok_or_else(|| bad(method))?;
                let mut out = Builder::Empty(receiver.count());
                for k in 0..receiver.count() {
                    let item = receiver.element(k).ok_or_else(|| bad(method))?;
                    let value = self.apply(&each, vec![item], span)?;
                    self.build(&mut out, value, span)?;
                }
                Ok(Self::built(receiver.kind, receiver.shape.clone(), out))
            }
            "zip_map" => {
                let other = dense(args.get(1), method)?.clone();
                let each = args.get(2).cloned().ok_or_else(|| bad(method))?;
                same_shape(&receiver, &other, span)?;
                let mut out = Builder::Empty(receiver.count());
                for k in 0..receiver.count() {
                    let a = receiver.element(k).ok_or_else(|| bad(method))?;
                    let b = other.element(k).ok_or_else(|| bad(method))?;
                    let value = self.apply(&each, vec![a, b], span)?;
                    self.build(&mut out, value, span)?;
                }
                Ok(Self::built(receiver.kind, receiver.shape.clone(), out))
            }
            "update" => {
                let each = args.get(1).cloned().ok_or_else(|| bad(method))?;
                drop(args);
                let mut owned = self.owned(receiver, span)?;
                for k in 0..owned.count() {
                    let at = owned.indices(k).ok_or_else(|| bad(method))?;
                    let position = owned.position(&at).ok_or_else(|| bad(method))?;
                    let item = owned.buffer.get(position).ok_or_else(|| bad(method))?;
                    let new = self.apply(&each, vec![item], span)?;
                    let buffer = Rc::get_mut(&mut owned.buffer).ok_or_else(|| bad(method))?;
                    match (buffer, new) {
                        (Elements::F64(items), Value::F64(x)) => {
                            if let Some(slot) = items.get_mut(position) {
                                *slot = float::canonical(x);
                            }
                        }
                        (Elements::Values(items), new) => {
                            if let Some(slot) = items.get_mut(position) {
                                *slot = new;
                            }
                        }
                        _ => return Err(bad(method)),
                    }
                }
                Ok(updated(owned))
            }
            "fold" => {
                let mut acc = args.get(1).cloned().ok_or_else(|| bad(method))?;
                let step = args.get(2).cloned().ok_or_else(|| bad(method))?;
                for k in 0..receiver.count() {
                    let item = receiver.element(k).ok_or_else(|| bad(method))?;
                    acc = self.apply(&step, vec![acc, item], span)?;
                }
                Ok(acc)
            }
            "multiply_elements" | "divide_elements" => {
                let other = dense(args.get(1), method)?;
                same_shape(&receiver, other, span)?;
                let (a, b) = (
                    receiver.f64_iter().ok_or_else(|| bad(method))?,
                    other.f64_iter().ok_or_else(|| bad(method))?,
                );
                let multiply = method == "multiply_elements";
                let values = a.zip(b).map(|(x, y)| {
                    Ok(if multiply {
                        float::mul(x, y)
                    } else {
                        float::div(x, y)
                    })
                });
                self.made_f64(
                    receiver.kind,
                    receiver.shape.clone(),
                    receiver.count(),
                    values,
                    span,
                )
            }
            "sum" => {
                let items = receiver.f64_iter().ok_or_else(|| bad(method))?;
                Ok(Value::F64(chains::sum_f64(items)))
            }
            "dot" => {
                let other = dense(args.get(1), method)?;
                same_shape(&receiver, other, span)?;
                let (a, b) = (
                    receiver.f64_iter().ok_or_else(|| bad(method))?,
                    other.f64_iter().ok_or_else(|| bad(method))?,
                );
                Ok(Value::F64(chains::dot_f64(a.zip(b))))
            }
            "norm2" => {
                let items = receiver.f64_iter().ok_or_else(|| bad(method))?;
                Ok(Value::F64(chains::norm2_f64(items)))
            }
            "all_finite" => {
                let items = receiver.f64_iter().ok_or_else(|| bad(method))?;
                // The whole scan, then the answer (rule 54).
                let finite = items.fold(true, |all, x| all & x.is_finite());
                Ok(Value::Bool(finite))
            }
            "sum_exact" => {
                let mut total = BigInt::from(0);
                for k in 0..receiver.count() {
                    match receiver.element(k) {
                        Some(Value::Int(n)) => total += n,
                        _ => return Err(bad(method)),
                    }
                }
                Ok(Value::Int(total))
            }
            "dot_exact" => {
                let other = dense(args.get(1), method)?;
                same_shape(&receiver, other, span)?;
                let mut total = BigInt::from(0);
                for k in 0..receiver.count() {
                    match (receiver.element(k), other.element(k)) {
                        (Some(Value::Int(a)), Some(Value::Int(b))) => total += a * b,
                        _ => return Err(bad(method)),
                    }
                }
                Ok(Value::Int(total))
            }
            "row_sums" | "column_sums" => {
                let (rows, columns) = matrix_shape(&receiver);
                let along_rows = method == "row_sums";
                let (outer, inner) = if along_rows {
                    (rows, columns)
                } else {
                    (columns, rows)
                };
                let line = |i: usize| -> Result<f64, Stop<'p>> {
                    if receiver.f64_iter().is_none() {
                        return Err(bad(method));
                    }
                    let items = (0..inner).map(|j| {
                        let at = if along_rows { [i, j] } else { [j, i] };
                        f64_at(&receiver, &at).unwrap_or(f64::NAN)
                    });
                    Ok(chains::sum_f64(items))
                };
                let value = self.made_f64(
                    DenseKind::Array,
                    vec![outer],
                    outer,
                    (0..outer).map(line),
                    span,
                )?;
                ensure_len(&value, outer, method)?;
                Ok(value)
            }
            "multiply" => {
                let other = dense(args.get(1), method)?;
                let (m, k) = matrix_shape(&receiver);
                let (k2, n) = matrix_shape(other);
                if k != k2 {
                    return Err(precondition(
                        "`multiply` of matrices whose shapes do not match, which its precondition excludes",
                        span,
                    ));
                }
                if receiver.f64_iter().is_none() || other.f64_iter().is_none() {
                    return Err(bad(method));
                }
                let size = m.checked_mul(n).ok_or_else(|| self.over(span))?;
                let element = |ij: usize| -> Result<f64, Stop<'p>> {
                    let (i, j) = (ij / n.max(1), ij % n.max(1));
                    let pairs = (0..k).map(|t| {
                        (
                            f64_at(&receiver, &[i, t]).unwrap_or(f64::NAN),
                            f64_at(other, &[t, j]).unwrap_or(f64::NAN),
                        )
                    });
                    Ok(chains::dot_f64(pairs))
                };
                let value = self.made_f64(
                    DenseKind::Matrix,
                    vec![m, n],
                    size,
                    (0..size).map(element),
                    span,
                )?;
                ensure_shape(&value, &[m, n], method)?;
                Ok(value)
            }
            "multiply_array" => {
                let other = dense(args.get(1), method)?;
                let (m, k) = matrix_shape(&receiver);
                if other.shape.first().copied() != Some(k) {
                    return Err(precondition(
                        "`multiply_array` of a matrix and an array whose shapes do not match, which its precondition excludes",
                        span,
                    ));
                }
                if receiver.f64_iter().is_none() {
                    return Err(bad(method));
                }
                let element = |i: usize| -> Result<f64, Stop<'p>> {
                    let v = other.f64_iter().ok_or_else(|| bad(method))?;
                    let pairs = v
                        .enumerate()
                        .map(|(t, y)| (f64_at(&receiver, &[i, t]).unwrap_or(f64::NAN), y));
                    Ok(chains::dot_f64(pairs))
                };
                let value =
                    self.made_f64(DenseKind::Array, vec![m], m, (0..m).map(element), span)?;
                ensure_len(&value, m, method)?;
                Ok(value)
            }
            "add_product" => {
                let a = dense(args.get(1), method)?.clone();
                let b = dense(args.get(2), method)?.clone();
                drop(args);
                let (m, n) = matrix_shape(&receiver);
                let (m2, k) = matrix_shape(&a);
                let (k2, n2) = matrix_shape(&b);
                if m != m2 || k != k2 || n != n2 {
                    return Err(precondition(
                        "`add_product` of matrices whose shapes do not match, which its precondition excludes",
                        span,
                    ));
                }
                if a.f64_iter().is_none() || b.f64_iter().is_none() {
                    return Err(bad(method));
                }
                let mut owned = self.owned(receiver, span)?;
                for i in 0..m {
                    for j in 0..n {
                        let start = f64_at(&owned, &[i, j]).ok_or_else(|| bad(method))?;
                        let pairs = (0..k).map(|t| {
                            (
                                f64_at(&a, &[i, t]).unwrap_or(f64::NAN),
                                f64_at(&b, &[t, j]).unwrap_or(f64::NAN),
                            )
                        });
                        let value = chains::dot_from(start, pairs);
                        store(&mut owned, &[i, j], value, method)?;
                    }
                }
                Ok(updated(owned))
            }
            "add_scaled" => {
                let x = dense(args.get(1), method)?.clone();
                let k = scalar(args.get(2), method)?;
                drop(args);
                same_shape(&receiver, &x, span)?;
                let xs = x.f64_iter().ok_or_else(|| bad(method))?;
                let mut owned = self.owned(receiver, span)?;
                for (i, xi) in xs.enumerate() {
                    let yi = f64_at(&owned, &[i]).ok_or_else(|| bad(method))?;
                    store(&mut owned, &[i], float::fma(k, xi, yi), method)?;
                }
                Ok(updated(owned))
            }
            "scale" => {
                let k = scalar(args.get(1), method)?;
                drop(args);
                let mut owned = self.owned(receiver, span)?;
                for position in 0..owned.count() {
                    let at = owned.indices(position).ok_or_else(|| bad(method))?;
                    let x = f64_at(&owned, &at).ok_or_else(|| bad(method))?;
                    store(&mut owned, &at, float::mul(x, k), method)?;
                }
                Ok(updated(owned))
            }
            _ => Err(fault(format!("no dense method `{method}`"))),
        }
    }
}

/// The `F64` element of `value` at `indices`, read in place.
fn f64_at(value: &Dense<'_>, indices: &[usize]) -> Option<f64> {
    match (value.buffer.as_ref(), value.position(indices)) {
        (Elements::F64(items), Some(p)) => items.get(p).copied(),
        _ => None,
    }
}

/// The new value of a method that takes its receiver by `edit`, and its result, `unit`
/// (`spec/17 §4`).
fn updated(owned: Dense<'_>) -> Value<'_> {
    Value::Tuple(Rc::new(vec![Value::Dense(Rc::new(owned)), Value::Unit]))
}

/// Stores the `F64` `value` at `indices` of a value whose buffer it holds once.
fn store<'p>(
    owned: &mut Dense<'p>,
    indices: &[usize],
    value: f64,
    method: &str,
) -> Result<(), Stop<'p>> {
    let position = owned.position(indices).ok_or_else(|| bad(method))?;
    match Rc::get_mut(&mut owned.buffer) {
        Some(Elements::F64(items)) => {
            if let Some(slot) = items.get_mut(position) {
                *slot = float::canonical(value);
            }
            Ok(())
        }
        _ => Err(bad(method)),
    }
}

/// The rows and columns of a matrix.
fn matrix_shape(value: &Dense<'_>) -> (usize, usize) {
    (
        value.shape.first().copied().unwrap_or(0),
        value.shape.get(1).copied().unwrap_or(0),
    )
}

/// The precondition "same shape" of rule 34, at `span`.
fn same_shape<'p>(a: &Dense<'p>, b: &Dense<'p>, span: Span) -> Result<(), Stop<'p>> {
    if a.shape == b.shape {
        Ok(())
    } else {
        Err(precondition(
            "an element-wise operation of dense values of different shapes, which its precondition excludes",
            span,
        ))
    }
}

/// A kernel's `ensures` on its result's length, checked before it returns (rule 53): a
/// violation is an internal abort, so a kernel bug never gives the verifier a false fact.
fn ensure_len<'p>(value: &Value<'p>, len: usize, method: &str) -> Result<(), Stop<'p>> {
    ensure_shape(value, &[len], method)
}

/// [`ensure_len`] for any shape.
fn ensure_shape<'p>(value: &Value<'p>, shape: &[usize], method: &str) -> Result<(), Stop<'p>> {
    match value {
        Value::Dense(dense) if dense.shape == shape => Ok(()),
        _ => Err(fault(format!(
            "internal error: `{method}` gave a result of another shape than it ensures"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dense_function_of_the_prelude_is_one_this_engine_runs() {
        // A prelude function whose name this engine does not handle would fault at run time;
        // one it handled as another method would compute the wrong thing. Every `dense_` name
        // and constructor of the prelude is in `METHODS`, and nothing else is.
        let prelude = include_str!("../../../una-check/src/std/prelude_0_4.una");
        let mut seen = Vec::new();
        for line in prelude.lines() {
            let Some(rest) = line.strip_prefix("pub ") else {
                continue;
            };
            let Some(end) = rest.find(['(', '[']) else {
                continue;
            };
            let Some(name) = rest.get(..end) else {
                continue;
            };
            assert!(is_dense(name), "`{name}` is a dense function");
            let method = name
                .strip_prefix("dense_array_")
                .or_else(|| name.strip_prefix("dense_matrix_"))
                .unwrap_or(name);
            assert!(METHODS.contains(&method), "`{name}` is handled");
            seen.push(method);
        }
        for method in METHODS {
            assert!(seen.contains(&method), "`{method}` is a prelude function");
        }
    }
}
