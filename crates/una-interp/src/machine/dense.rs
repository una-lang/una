//! Dense values in the interpreter (`spec/29 §17`, ADR 0120 rules 26 to 54): construction,
//! views, element-wise functions, reductions, products, factorizations and solves, by the
//! blocked kernels of `una_core::dense::blocked` and their `fma` (ADR 0120 rule 53), through
//! the table of the run's limits (`una_core::dense::variants`, the baseline by default; rule
//! 56), whose chains are the references' of `una_core::dense`, so the interpreter gives the
//! bits of every other implementation. A kernel reads a contiguous operand in place, and a view of another
//! layout through a compact copy, reserved within the budget as the kernel's workspace.
//!
//! Every buffer is checked against the memory budget before it is allocated, in checked
//! arithmetic, and allocated fallibly: a refused one is U6003, never an abort of the process
//! (ADR 0120, Security). A view is O(1) and shares its parent's buffer; an update is in place
//! when the value and its buffer are held once, and copies the view's own elements first
//! otherwise (rule 31). Every shape a kernel ensures is checked before it returns (rule 53).

use std::borrow::Cow;
use std::rc::Rc;

use una_core::bigint::BigInt;
use una_core::dense::{self as chains, fma};
use una_core::float;
use una_syntax::Span;

use super::{Abort, Flow, Machine, Stop, fault, none, precondition, some};
use crate::value::{Dense, DenseKind, Elements, Value};

/// Whether the prelude function `name` is a constructor or a method of dense values.
pub(super) fn is_dense(name: &str) -> bool {
    matches!(
        name,
        "array"
            | "array_filled"
            | "matrix"
            | "matrix_filled"
            | "matrix_diagonal"
            | "tensor"
            | "tensor_filled"
    ) || name.starts_with("dense_")
}

/// The prefixes of the prelude functions of the methods of each dense type.
const PREFIXES: [&str; 4] = [
    "dense_array_",
    "dense_matrix_",
    "dense_tensor_",
    "dense_lu_",
];

/// The method of the prelude function `name` after its type's prefix, and the rank of a
/// tensor's function of one rank: `dense_tensor_get_3` is `get` of rank 3.
fn method_of(name: &str) -> Option<(&str, Option<usize>)> {
    let method = PREFIXES
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))?;
    if !name.starts_with("dense_tensor_") {
        return Some((method, None));
    }
    match method.rsplit_once('_') {
        Some((base, rank)) => match rank.parse::<usize>() {
            Ok(rank) if (1..=8).contains(&rank) => Some((base, Some(rank))),
            _ => Some((method, None)),
        },
        None => Some((method, None)),
    }
}

/// The methods of dense values this engine runs, the name after `dense_array_`,
/// `dense_matrix_`, `dense_tensor_` or `dense_lu_`, without a tensor's rank: a prelude function
/// of another name is a fault, never another method's computation, and a test checks that
/// every prelude function is one of these.
pub(super) const METHODS: &[&str] = &[
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
    "to_tensor",
    "to_array",
    "to_matrix",
    "make",
    "filled",
    "permute",
    "reshape",
    "dim",
    "sum_axis",
    "contract",
    "lu",
    "solve",
    "solve_array",
    "cholesky",
    "solve_lower",
    "solve_upper",
    "factors",
    "pivots",
    // Constructors, by their own names.
    "array",
    "array_filled",
    "matrix",
    "matrix_filled",
    "matrix_diagonal",
    "tensor",
    "tensor_filled",
];

/// The bytes a buffer holds per element: a raw `F64`, or a value.
fn element_bytes(f64s: bool) -> usize {
    if f64s {
        std::mem::size_of::<f64>()
    } else {
        std::mem::size_of::<Value<'_>>()
    }
}

/// The heap bytes that a clone of `value` allocates of its own: the limbs of a nonzero integer,
/// at least the allocator's 16-byte block. Every other value is a word or shares its heap
/// through a reference count, so its clone allocates nothing (ADR 0120, Security).
fn clone_heap(value: &Value<'_>) -> usize {
    match value {
        Value::Int(n) => {
            let limbs = usize::try_from(n.bits().div_ceil(64)).unwrap_or(usize::MAX);
            if limbs == 0 {
                0
            } else {
                limbs.saturating_mul(8).max(16)
            }
        }
        _ => 0,
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
        Some(Value::Int(n)) if n.sign() == una_core::bigint::Sign::Minus => Err(bad(name)),
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

/// The `F64` elements of `value` as one row-major run of its own storage, when it is laid out
/// so: a compact value, or a row or a slice of one. O(rank).
fn as_slice<'v>(value: &'v Dense<'_>) -> Option<&'v [f64]> {
    let Elements::F64(items) = value.buffer.as_ref() else {
        return None;
    };
    if value.strides != row_major(&value.shape) {
        return None;
    }
    items.get(value.offset..value.offset.checked_add(value.count())?)
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

/// An LU factorization: the order n, the factors in the first `n * n` places, and the pivots.
type Factored = (usize, Vec<f64>, Vec<usize>);

impl<'p> Machine<'p, '_> {
    /// Whether a buffer of `elements` elements of `bytes` each fits the memory budget, with
    /// what the run holds already: checked before the allocation, in checked arithmetic, so a
    /// size whose computation overflows fails the check (ADR 0120, Security). A bit set's words
    /// are checked by it too (`bitsets.rs`).
    pub(super) fn reserve(
        &self,
        elements: usize,
        bytes: usize,
        span: Span,
    ) -> Result<(), Stop<'p>> {
        self.reserve_with(elements, bytes, 0, span)
    }

    /// [`Machine::reserve`] with `extra` bytes more: the heap that the elements' clones
    /// allocate of their own ([`clone_heap`]).
    fn reserve_with(
        &self,
        elements: usize,
        bytes: usize,
        extra: usize,
        span: Span,
    ) -> Result<(), Stop<'p>> {
        let budget = self.max_memory.saturating_mul(1024 * 1024);
        let held = (self.held)().saturating_sub(self.baseline);
        let over = Stop::Abort(Abort::Memory {
            span,
            limit: self.max_memory,
        });
        match elements
            .checked_mul(bytes)
            .and_then(|size| size.checked_add(extra))
            .and_then(|size| size.checked_add(held))
        {
            Some(total) if total <= budget => Ok(()),
            _ => Err(over),
        }
    }

    /// The heap bytes that cloning `values` allocates of their own, saturating.
    fn clones_heap<'v>(values: impl Iterator<Item = &'v Value<'p>>) -> usize
    where
        'p: 'v,
    {
        values.fold(0usize, |sum, value| sum.saturating_add(clone_heap(value)))
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
            self.reserve_with(
                items.len(),
                element_bytes(false),
                Self::clones_heap(items.iter()),
                span,
            )?;
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
                let heap = Self::clones_heap(value.positions().filter_map(|p| items.get(p)));
                self.reserve_with(n, element_bytes(false), heap, span)?;
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
                let heap = n.saturating_mul(clone_heap(other));
                self.reserve_with(n, element_bytes(false), heap, span)?;
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
            // The checker calls the function of each rank; these take the shape as a list.
            "tensor" | "tensor_filled" => {
                let lengths = match args.first() {
                    Some(Value::List(items)) if (1..=8).contains(&items.len()) => {
                        items.iter().cloned().collect::<Vec<_>>()
                    }
                    _ => return Err(bad(name)),
                };
                let rank = lengths.len();
                let mut flat = lengths;
                flat.extend(args.into_iter().skip(1));
                self.tensor_new(rank, &flat, name == "tensor_filled", span)
            }
            _ => match method_of(name) {
                Some(("make", Some(rank))) => self.tensor_new(rank, &args, false, span),
                Some(("filled", Some(rank))) => self.tensor_new(rank, &args, true, span),
                Some((method, rank)) => self.method(method, rank, args, span),
                None => Err(fault(format!("no dense function `{name}`"))),
            },
        }
    }

    /// A tensor of the `rank` lengths that start `args`, then the list of its elements, or the
    /// value of every element when `filled` (ADR 0120 rule 32).
    fn tensor_new(&self, rank: usize, args: &[Value<'p>], filled: bool, span: Span) -> Flow<'p> {
        let name = if filled { "tensor_filled" } else { "tensor" };
        let mut shape = Vec::with_capacity(rank);
        for k in 0..rank {
            shape.push(dimension(args.get(k), name, self.over(span))?);
        }
        let n = shape
            .iter()
            .try_fold(1_usize, |n, length| n.checked_mul(*length))
            .ok_or_else(|| self.over(span))?;
        let elements = if filled {
            let value = args.get(rank).ok_or_else(|| bad(name))?;
            self.filled(n, value, span)?
        } else {
            let Some(Value::List(items)) = args.get(rank) else {
                return Err(bad(name));
            };
            if items.len() != n {
                return Err(precondition(
                    "`tensor` with a number of elements other than the product of its lengths, which its precondition excludes",
                    span,
                ));
            }
            self.buffer_from(items, span)?
        };
        Ok(compact(DenseKind::Tensor, shape, elements))
    }

    /// The `F64` elements of `value` in row-major order, in a vector reserved within the budget
    /// with room for `extra` more: the workspace of a factorization or a solve (ADR 0120,
    /// Security).
    fn f64_copy(
        &self,
        value: &Dense<'p>,
        extra: usize,
        name: &str,
        span: Span,
    ) -> Result<Vec<f64>, Stop<'p>> {
        let items = value.f64_iter().ok_or_else(|| bad(name))?;
        let n = value
            .count()
            .checked_add(extra)
            .ok_or_else(|| self.over(span))?;
        let mut out = self.buffer(n, true, span)?;
        out.extend(items);
        Ok(out)
    }

    /// The `F64` elements of `value` in row-major order as one slice: its own storage when it is
    /// contiguous ([`as_slice`]), and otherwise a compact copy, reserved within the budget, the
    /// workspace of a kernel. O(1) or O(size).
    fn slice_of<'v>(
        &self,
        value: &'v Dense<'p>,
        name: &str,
        span: Span,
    ) -> Result<Cow<'v, [f64]>, Stop<'p>> {
        match as_slice(value) {
            Some(items) => Ok(Cow::Borrowed(items)),
            None => Ok(Cow::Owned(self.f64_copy(value, 0, name, span)?)),
        }
    }

    /// The size of `value`, a square matrix, or a precondition failure.
    fn square(value: &Dense<'p>, name: &str, span: Span) -> Result<usize, Stop<'p>> {
        let (rows, columns) = matrix_shape(value);
        if rows == columns {
            Ok(rows)
        } else {
            Err(precondition(
                &format!(
                    "`{name}` of a matrix that is not square, which its precondition excludes"
                ),
                span,
            ))
        }
    }

    /// The LU factorization of the square matrix `value` (ADR 0120 rule 48): the factors and
    /// the pivots, or `None` when it is singular, after it ran to its end. The factors are in
    /// the first `n * n` places of a vector with room for `extra` more.
    fn factorize(
        &self,
        value: &Dense<'p>,
        extra: usize,
        span: Span,
    ) -> Result<Option<Factored>, Stop<'p>> {
        let n = Self::square(value, "lu", span)?;
        let mut factors = self.f64_copy(value, extra, "lu", span)?;
        let mut pivots: Vec<usize> = self.buffer(n, true, span)?;
        pivots.resize(n, 0);
        let nonsingular = (self.kernels.lu)(&mut factors, n, &mut pivots);
        Ok(nonsingular.then_some((n, factors, pivots)))
    }

    /// `rhs`, a matrix of `n` rows, solved with the factors and pivots of an LU factorization
    /// (ADR 0120 rule 48): a matrix of `rhs`'s shape, checked before it is returned.
    fn lu_solved(
        &self,
        factors: &[f64],
        pivots: &[usize],
        n: usize,
        rhs: &Dense<'p>,
        kind: DenseKind,
        span: Span,
    ) -> Flow<'p> {
        let shape = rhs.shape.clone();
        let r = if kind == DenseKind::Array {
            1
        } else {
            shape.get(1).copied().unwrap_or(0)
        };
        if shape.first().copied() != Some(n) {
            return Err(precondition(
                "`solve` with a right-hand side of another number of rows, which its precondition excludes",
                span,
            ));
        }
        let mut x = self.f64_copy(rhs, 0, "solve", span)?;
        (self.kernels.lu_solve)(factors, pivots, n, &mut x, r);
        let value = compact(kind, shape.clone(), Elements::F64(x));
        ensure_shape(&value, &shape, "solve")?;
        Ok(value)
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

    /// A method of a dense value, its receiver first in `args`; `rank` is the rank of a
    /// tensor's function of one rank, the length of the list literal it was called with.
    fn method(
        &mut self,
        method: &str,
        rank: Option<usize>,
        args: Vec<Value<'p>>,
        span: Span,
    ) -> Flow<'p> {
        if !METHODS.contains(&method) {
            return Err(fault(format!("no dense method `{method}`")));
        }
        // The run's table of kernels (ADR 0120 rule 56), a shared reference to a static.
        let kernels = self.kernels;
        let receiver = dense(args.first(), method)?.clone();
        // An index or a permutation of another length than the tensor's rank: `at` gives
        // `None`, and the others' preconditions exclude it.
        if let Some(rank) = rank
            && matches!(method, "get" | "at" | "set" | "permute")
            && rank != receiver.shape.len()
        {
            return if method == "at" {
                Ok(none())
            } else {
                Err(precondition(
                    "a list literal of another length than the tensor's rank, which the precondition excludes",
                    span,
                ))
            };
        }
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
                // The integers it clones allocate of their own, so their heap is reserved too.
                let heap = match receiver.buffer.as_ref() {
                    Elements::Values(items) => {
                        Self::clones_heap(receiver.positions().filter_map(|p| items.get(p)))
                    }
                    Elements::F64(_) => 0,
                };
                self.reserve_with(receiver.count(), element_bytes(false), heap, span)?;
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
                Ok(Value::F64(match as_slice(&receiver) {
                    Some(x) => (kernels.sum)(x),
                    None => chains::sum_f64(items),
                }))
            }
            "dot" => {
                let other = dense(args.get(1), method)?;
                same_shape(&receiver, other, span)?;
                let (a, b) = (
                    receiver.f64_iter().ok_or_else(|| bad(method))?,
                    other.f64_iter().ok_or_else(|| bad(method))?,
                );
                Ok(Value::F64(match (as_slice(&receiver), as_slice(other)) {
                    (Some(x), Some(y)) => kernels.dot(x, y),
                    _ => kernels.dot_of_pairs(0.0, a.zip(b)),
                }))
            }
            "norm2" => {
                let items = receiver.f64_iter().ok_or_else(|| bad(method))?;
                Ok(Value::F64(kernels.norm2_of(items)))
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
                let a = self.slice_of(&receiver, method, span)?;
                let b = self.slice_of(other, method, span)?;
                let mut out: Vec<f64> = self.buffer(size, true, span)?;
                out.resize(size, 0.0);
                (kernels.add_product)(&mut out, &a, &b, m, k, n);
                let value = compact(DenseKind::Matrix, vec![m, n], Elements::F64(out));
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
                if receiver.f64_iter().is_none() || other.f64_iter().is_none() {
                    return Err(bad(method));
                }
                let a = self.slice_of(&receiver, method, span)?;
                let v = self.slice_of(other, method, span)?;
                let mut out: Vec<f64> = self.buffer(m, true, span)?;
                out.resize(m, 0.0);
                (kernels.multiply_array)(&a, m, k, &v, &mut out);
                let value = compact(DenseKind::Array, vec![m], Elements::F64(out));
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
                let count = owned.count();
                if count == 0 {
                    return Ok(updated(owned));
                }
                if as_slice(&owned).is_none() {
                    // A view held once, of another layout: compacted first, so the kernel
                    // updates one run of storage in place.
                    let elements = self.copy(&owned, span)?;
                    owned.strides = row_major(&owned.shape);
                    owned.offset = 0;
                    owned.buffer = Rc::new(elements);
                }
                let (x, y) = (
                    self.slice_of(&a, method, span)?,
                    self.slice_of(&b, method, span)?,
                );
                let start = owned.offset;
                match Rc::get_mut(&mut owned.buffer) {
                    Some(Elements::F64(items)) => {
                        let c = items
                            .get_mut(start..start + count)
                            .ok_or_else(|| bad(method))?;
                        (kernels.add_product)(c, &x, &y, m, k, n);
                    }
                    _ => return Err(bad(method)),
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
                    store(&mut owned, &[i], fma::baseline(k, xi, yi), method)?;
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
            // The O(1) conversions between an array or a matrix and a tensor (rule 39): views
            // of another type over the same storage.
            "to_tensor" | "to_array" | "to_matrix" => {
                let (kind, rank) = match method {
                    "to_tensor" => (DenseKind::Tensor, receiver.shape.len()),
                    "to_array" => (DenseKind::Array, 1),
                    _ => (DenseKind::Matrix, 2),
                };
                if receiver.shape.len() != rank {
                    return Err(precondition(
                        "a tensor of another rank than the conversion takes, which its precondition excludes",
                        span,
                    ));
                }
                let mut view = (*receiver).clone();
                view.kind = kind;
                Ok(Value::Dense(Rc::new(view)))
            }
            "dim" => {
                let axis = index(args.get(1))
                    .filter(|axis| *axis < receiver.shape.len())
                    .ok_or_else(|| {
                        precondition(
                            "`dim` of an axis the tensor does not have, which its precondition excludes",
                            span,
                        )
                    })?;
                let length = receiver.shape.get(axis).copied().unwrap_or(0);
                Ok(Value::Int(BigInt::from(length)))
            }
            // A view with its axes and their strides in the order of the permutation (rule
            // 29): O(rank).
            "permute" => {
                let rank = receiver.shape.len();
                let axes: Option<Vec<usize>> = (1..=rank).map(|i| index(args.get(i))).collect();
                let axes = axes.ok_or_else(|| bad(method))?;
                let mut seen = [false; 8];
                for axis in &axes {
                    match seen.get_mut(*axis) {
                        Some(slot) if !*slot && *axis < rank => *slot = true,
                        _ => return Err(bad(method)),
                    }
                }
                let mut view = (*receiver).clone();
                view.shape = axes
                    .iter()
                    .map(|axis| receiver.shape.get(*axis).copied().unwrap_or(0))
                    .collect();
                view.strides = axes
                    .iter()
                    .map(|axis| receiver.strides.get(*axis).copied().unwrap_or(0))
                    .collect();
                Ok(Value::Dense(Rc::new(view)))
            }
            // O(1) on a contiguous tensor; otherwise a compacting copy, after the budget check
            // (rule 39).
            "reshape" => {
                let rank = rank.ok_or_else(|| bad(method))?;
                let mut shape = Vec::with_capacity(rank);
                for k in 1..=rank {
                    shape.push(dimension(args.get(k), method, self.over(span))?);
                }
                let n = shape
                    .iter()
                    .try_fold(1_usize, |n, length| n.checked_mul(*length))
                    .ok_or_else(|| self.over(span))?;
                if n != receiver.count() {
                    return Err(precondition(
                        "`reshape` to another number of elements, which its precondition excludes",
                        span,
                    ));
                }
                if receiver.strides == row_major(&receiver.shape) {
                    let mut view = (*receiver).clone();
                    view.kind = DenseKind::Tensor;
                    view.strides = row_major(&shape);
                    view.shape = shape;
                    Ok(Value::Dense(Rc::new(view)))
                } else {
                    let elements = self.copy(&receiver, span)?;
                    Ok(compact(DenseKind::Tensor, shape, elements))
                }
            }
            // Each element of the result is a sum along the axis, by rule 45.
            "sum_axis" => {
                let rank = receiver.shape.len();
                let axis = index(args.get(1))
                    .filter(|axis| rank >= 2 && *axis < rank)
                    .ok_or_else(|| {
                        precondition(
                            "`sum` along an axis the tensor does not have, which its precondition excludes",
                            span,
                        )
                    })?;
                if receiver.f64_iter().is_none() {
                    return Err(bad(method));
                }
                let length = receiver.shape.get(axis).copied().unwrap_or(0);
                let shape: Vec<usize> = receiver
                    .shape
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k != axis)
                    .map(|(_, length)| *length)
                    .collect();
                let n: usize = shape.iter().product();
                let element = |p: usize| -> Result<f64, Stop<'p>> {
                    let mut at = [0_usize; 8];
                    let mut rest = p;
                    for (k, length) in shape.iter().enumerate().rev() {
                        let slot = if k < axis { k } else { k + 1 };
                        if let Some(index) = at.get_mut(slot) {
                            *index = rest % length.max(&1);
                        }
                        rest /= length.max(&1);
                    }
                    let items = (0..length).map(|i| {
                        if let Some(index) = at.get_mut(axis) {
                            *index = i;
                        }
                        f64_at(&receiver, at.get(..rank).unwrap_or(&[])).unwrap_or(f64::NAN)
                    });
                    Ok(chains::sum_f64(items))
                };
                let value = self.made_f64(
                    DenseKind::Tensor,
                    shape.clone(),
                    n,
                    (0..n).map(element),
                    span,
                )?;
                ensure_shape(&value, &shape, "sum")?;
                Ok(value)
            }
            // Each element of the result is an inner product over the pair of axes, by rule 46,
            // so it has the bits of the matching element of `multiply` (rule 39).
            "contract" => {
                let other = dense(args.get(1), method)?.clone();
                let (axis, other_axis) = (index(args.get(2)), index(args.get(3)));
                let (rt, ru) = (receiver.shape.len(), other.shape.len());
                let (axis, other_axis) = match (axis, other_axis) {
                    (Some(k), Some(l))
                        if k < rt
                            && l < ru
                            && receiver.shape.get(k) == other.shape.get(l)
                            && (3..=10).contains(&(rt + ru)) =>
                    {
                        (k, l)
                    }
                    _ => {
                        return Err(precondition(
                            "`contract` over axes whose lengths do not match, which its precondition excludes",
                            span,
                        ));
                    }
                };
                if receiver.f64_iter().is_none() || other.f64_iter().is_none() {
                    return Err(bad(method));
                }
                let terms = receiver.shape.get(axis).copied().unwrap_or(0);
                let mut shape: Vec<usize> = Vec::with_capacity(rt + ru - 2);
                shape.extend(
                    receiver
                        .shape
                        .iter()
                        .enumerate()
                        .filter(|(k, _)| *k != axis)
                        .map(|(_, length)| *length),
                );
                shape.extend(
                    other
                        .shape
                        .iter()
                        .enumerate()
                        .filter(|(k, _)| *k != other_axis)
                        .map(|(_, length)| *length),
                );
                let n = shape
                    .iter()
                    .try_fold(1_usize, |n, length| n.checked_mul(*length))
                    .ok_or_else(|| self.over(span))?;
                let element = |p: usize| -> Result<f64, Stop<'p>> {
                    // The result's indices: the remaining axes of t, then those of u.
                    let mut out = [0_usize; 16];
                    let mut rest = p;
                    for (k, length) in shape.iter().enumerate().rev() {
                        if let Some(index) = out.get_mut(k) {
                            *index = rest % length.max(&1);
                        }
                        rest /= length.max(&1);
                    }
                    let (mut ti, mut ui) = ([0_usize; 8], [0_usize; 8]);
                    let mut next = 0;
                    for (k, slot) in ti.iter_mut().enumerate().take(rt) {
                        if k != axis {
                            *slot = out.get(next).copied().unwrap_or(0);
                            next += 1;
                        }
                    }
                    for (k, slot) in ui.iter_mut().enumerate().take(ru) {
                        if k != other_axis {
                            *slot = out.get(next).copied().unwrap_or(0);
                            next += 1;
                        }
                    }
                    let pairs = (0..terms).map(|i| {
                        if let Some(index) = ti.get_mut(axis) {
                            *index = i;
                        }
                        if let Some(index) = ui.get_mut(other_axis) {
                            *index = i;
                        }
                        (
                            f64_at(&receiver, ti.get(..rt).unwrap_or(&[])).unwrap_or(f64::NAN),
                            f64_at(&other, ui.get(..ru).unwrap_or(&[])).unwrap_or(f64::NAN),
                        )
                    });
                    Ok(kernels.dot_of_pairs(0.0, pairs))
                };
                let value = self.made_f64(
                    DenseKind::Tensor,
                    shape.clone(),
                    n,
                    (0..n).map(element),
                    span,
                )?;
                ensure_shape(&value, &shape, "contract")?;
                Ok(value)
            }
            // The factorizations and solves (rules 48 to 50). Each runs to its end, whatever
            // the values (rule 54), in workspaces reserved within the budget first.
            "lu" => {
                drop(args);
                // Room for the pivots is reserved with the factors: they are the last row.
                let rows = matrix_shape(&receiver).0;
                let Some((n, mut factors, pivots)) = self.factorize(&receiver, rows, span)? else {
                    return Ok(none());
                };
                factors.extend(pivots.iter().map(|p| *p as f64));
                let value = compact(DenseKind::Lu, vec![n + 1, n], Elements::F64(factors));
                ensure_shape(&value, &[n + 1, n], method)?;
                Ok(some(value))
            }
            "solve" | "solve_array" if receiver.kind == DenseKind::Matrix => {
                let rhs = dense(args.get(1), method)?.clone();
                drop(args);
                let Some((n, factors, pivots)) = self.factorize(&receiver, 0, span)? else {
                    return Ok(none());
                };
                let kind = if method == "solve" {
                    DenseKind::Matrix
                } else {
                    DenseKind::Array
                };
                Ok(some(
                    self.lu_solved(&factors, &pivots, n, &rhs, kind, span)?,
                ))
            }
            "solve" => {
                let rhs = dense(args.get(1), method)?.clone();
                drop(args);
                let n = receiver.shape.get(1).copied().unwrap_or(0);
                let Elements::F64(items) = receiver.buffer.as_ref() else {
                    return Err(bad(method));
                };
                let start = receiver.offset;
                let factors = items.get(start..start + n * n).ok_or_else(|| bad(method))?;
                let stored = items
                    .get(start + n * n..start + n * n + n)
                    .ok_or_else(|| bad(method))?;
                let mut pivots: Vec<usize> = self.buffer(n, true, span)?;
                pivots.extend(stored.iter().map(|p| *p as usize));
                self.lu_solved(factors, &pivots, n, &rhs, DenseKind::Matrix, span)
            }
            "factors" => {
                let n = receiver.shape.get(1).copied().unwrap_or(0);
                let mut view = (*receiver).clone();
                view.kind = DenseKind::Matrix;
                view.shape = vec![n, n];
                Ok(Value::Dense(Rc::new(view)))
            }
            "pivots" => {
                let n = receiver.shape.get(1).copied().unwrap_or(0);
                // Each pivot but row 0 is an integer with a heap of its own ([`clone_heap`]).
                self.reserve_with(n, element_bytes(false), n.saturating_mul(16), span)?;
                let mut out = self.buffer(n, false, span)?;
                for k in 0..n {
                    let p = f64_at(&receiver, &[n, k]).ok_or_else(|| bad(method))?;
                    out.push(Value::Int(BigInt::from(p as usize)));
                }
                Ok(Value::List(Rc::new(out)))
            }
            "cholesky" => {
                drop(args);
                let n = Self::square(&receiver, method, span)?;
                let mut factor = self.f64_copy(&receiver, 0, method, span)?;
                if !(kernels.cholesky)(&mut factor, n) {
                    return Ok(none());
                }
                let value = compact(DenseKind::Matrix, vec![n, n], Elements::F64(factor));
                ensure_shape(&value, &[n, n], method)?;
                Ok(some(value))
            }
            "solve_lower" | "solve_upper" => {
                let rhs = dense(args.get(1), method)?.clone();
                let unit = match args.get(2) {
                    Some(Value::Bool(unit)) => *unit,
                    _ => return Err(bad(method)),
                };
                drop(args);
                let n = Self::square(&receiver, method, span)?;
                let (rows, r) = matrix_shape(&rhs);
                if rows != n {
                    return Err(precondition(
                        "a triangular solve with a right-hand side of another number of rows, which its precondition excludes",
                        span,
                    ));
                }
                let t = self.f64_copy(&receiver, 0, method, span)?;
                let mut x = self.f64_copy(&rhs, 0, method, span)?;
                let triangle = if method == "solve_lower" {
                    chains::Triangle::Lower
                } else {
                    chains::Triangle::Upper
                };
                (kernels.solve_triangular)(&t, n, &mut x, r, triangle, unit);
                let value = compact(DenseKind::Matrix, vec![n, r], Elements::F64(x));
                ensure_shape(&value, &[n, r], method)?;
                Ok(value)
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
            // The prelude of edition 0.4 holds `Decimal`'s functions too.
            if !is_dense(name) {
                continue;
            }
            let method = method_of(name).map_or(name, |(method, _)| method);
            assert!(METHODS.contains(&method), "`{name}` is handled");
            seen.push(method);
        }
        for method in METHODS {
            assert!(seen.contains(method), "`{method}` is a prelude function");
        }
    }
}
