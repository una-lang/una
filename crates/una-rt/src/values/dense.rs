//! Dense values in native code (`spec/29 §17`, ADR 0120 rules 26 to 54; design note
//! `docs/plan/0120-s6-design.md`, part 1): construction, views, element access, reductions,
//! products, factorizations and solves, with the semantics, the text and the bits of the
//! interpreter's `machine/dense.rs`.
//!
//! - **One heap object per value** (D1): an [`Object::Dense`] header with its kind, offset,
//!   shape and strides over a store. Canonical `F64` elements live raw in an `Arc<Vec<f64>>`,
//!   counted atomically and shared by the views of one value and, across tasks, by the copies
//!   of a `together` block; every other element is a word of an [`Object::Words`] of this
//!   task's heap, which the header holds as its one cell, as a list holds its items.
//! - **Every allocation is reserved first** (D2): its bytes are computed in checked
//!   arithmetic and announced to [`crate::runtime::try_reserve`] before a fallible
//!   `try_reserve_exact`; a refusal is U6003 at the call's site, never an abort of the process.
//! - **In place when held once** (D3, rule 31): the `_owned` entry points take the caller's
//!   reference and write in place when the header and its store are held once, and copy the
//!   value's own elements into a fresh compact store first otherwise.
//! - **A precondition that fails is an internal abort** (D4): U8001 at the call's site, with
//!   the interpreter's message.
//!
//! Every function takes words and returns a word the caller owns; a view is O(1) and an access
//! is O(rank), with no allocation but the boxed `F64` it returns. Each function here reads the
//! heap through [`with`] and [`with_mut`], one object at a time, and never holds a borrow of
//! the heap while it calls another function of [`super`].

use std::borrow::Cow;
use std::sync::Arc;

use num_bigint::BigInt;
use una_core::dense::{self as chains, blocked, fma};
use una_core::float;

use super::{
    HEAP, NONE, Object, alloc, copied, count, count_limit, data, f64_word, fault, field,
    int_value, int_word, is_handle, push, release, retain, slot, some, unique, with,
};

/// The most axes a tensor has (ADR 0120 rule 27).
pub(crate) const MAX_RANK: usize = 8;

/// The bytes a dense value's header takes, which a copy at `together` charges for it whatever
/// its elements: an `F64` store is shared, never copied (D1, ADR 0113 amendment of
/// 2026-10-19).
pub(crate) const HEADER_BYTES: usize = size_of::<Dense>();

/// Which dense type a value is, as the interpreter's `DenseKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DenseKind {
    /// `Array[T]`: one axis.
    Array,
    /// `Matrix[T]`: rows, then columns.
    Matrix,
    /// `Tensor[T]`: 1 to 8 axes.
    Tensor,
    /// `Lu[T]` of size n: n + 1 rows of n raw `F64` values, the factors in the first n rows
    /// and the pivots in the last (ADR 0120 rule 38), the interpreter's layout.
    Lu,
}

/// The lengths of a value's axes, or its strides, or the indices of one element: 0 to 8
/// numbers, held inline, so no access allocates. The places past `rank` are always 0.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Axes {
    rank: usize,
    lengths: [usize; MAX_RANK],
}

impl PartialEq for Axes {
    fn eq(&self, other: &Axes) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Axes {}

impl Axes {
    /// The axes of `lengths`, or `None` past [`MAX_RANK`].
    fn of(lengths: &[usize]) -> Option<Axes> {
        if lengths.len() > MAX_RANK {
            return None;
        }
        let mut out = [0; MAX_RANK];
        for (slot, length) in out.iter_mut().zip(lengths) {
            *slot = *length;
        }
        Some(Axes {
            rank: lengths.len(),
            lengths: out,
        })
    }

    /// One axis of length `n`.
    fn one(n: usize) -> Axes {
        let mut lengths = [0; MAX_RANK];
        lengths[0] = n;
        Axes { rank: 1, lengths }
    }

    /// Two axes, `rows` then `columns`.
    fn two(rows: usize, columns: usize) -> Axes {
        let mut lengths = [0; MAX_RANK];
        lengths[0] = rows;
        lengths[1] = columns;
        Axes { rank: 2, lengths }
    }

    /// The numbers, in order.
    pub(crate) fn as_slice(&self) -> &[usize] {
        self.lengths.get(..self.rank).unwrap_or(&[])
    }

    /// The number of axes.
    pub(crate) fn rank(&self) -> usize {
        self.rank
    }

    /// The number at `axis`.
    fn get(&self, axis: usize) -> Option<usize> {
        self.as_slice().get(axis).copied()
    }

    /// The product of the lengths, saturating: a value's shape was checked when it was made,
    /// so its product fits.
    fn count(&self) -> usize {
        self.as_slice()
            .iter()
            .fold(1_usize, |n, length| n.saturating_mul(*length))
    }

    /// The product of the lengths, or `None` when it overflows.
    fn checked_count(&self) -> Option<usize> {
        self.as_slice()
            .iter()
            .try_fold(1_usize, |n, length| n.checked_mul(*length))
    }

    /// The strides of a contiguous row-major value of this shape, as the interpreter's
    /// `row_major`.
    fn row_major(&self) -> Axes {
        let mut strides = Axes {
            rank: self.rank,
            lengths: [0; MAX_RANK],
        };
        let mut step = 1_usize;
        for axis in (0..self.rank).rev() {
            if let (Some(stride), Some(length)) =
                (strides.lengths.get_mut(axis), self.lengths.get(axis))
            {
                *stride = step;
                step = step.saturating_mul(*length);
            }
        }
        strides
    }

    /// The numbers in reverse order: a transpose's shape and strides.
    fn reversed(&self) -> Axes {
        let mut out = *self;
        if let Some(used) = out.lengths.get_mut(..self.rank) {
            used.reverse();
        }
        out
    }

    /// The numbers without the one at `axis`.
    fn without(&self, axis: usize) -> Axes {
        let mut out = Axes {
            rank: 0,
            lengths: [0; MAX_RANK],
        };
        for (k, length) in self.as_slice().iter().enumerate() {
            if k != axis
                && let Some(slot) = out.lengths.get_mut(out.rank)
            {
                *slot = *length;
                out.rank += 1;
            }
        }
        out
    }
}

/// Where a dense value's elements are in its store: its kind, the position of its first
/// element, and the length and stride of each axis (ADR 0120 rules 28 and 29). Element
/// `(i_0, ...)` is at `offset + Σ i_k × strides_k`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Its type.
    pub(crate) kind: DenseKind,
    /// The position of the first element.
    pub(crate) offset: usize,
    /// The length of each axis.
    pub(crate) shape: Axes,
    /// The distance between consecutive indices of each axis, in the store.
    pub(crate) strides: Axes,
}

impl Layout {
    /// A contiguous row-major layout of `shape`.
    fn compact(kind: DenseKind, shape: Axes) -> Layout {
        Layout {
            kind,
            offset: 0,
            strides: shape.row_major(),
            shape,
        }
    }

    /// The number of elements.
    pub(crate) fn count(&self) -> usize {
        self.shape.count()
    }

    /// Whether the value is laid out row-major without gaps.
    fn contiguous(&self) -> bool {
        self.strides == self.shape.row_major()
    }

    /// The position in the store of the element at `indices`, one per axis, when each is in
    /// range. O(rank).
    fn position(&self, indices: &[usize]) -> Option<usize> {
        if indices.len() != self.shape.rank {
            return None;
        }
        let mut position = self.offset;
        for ((index, length), stride) in indices
            .iter()
            .zip(self.shape.as_slice())
            .zip(self.strides.as_slice())
        {
            if index >= length {
                return None;
            }
            position = position.checked_add(index.checked_mul(*stride)?)?;
        }
        Some(position)
    }

    /// The position in the store of the element at row-major position `k`, from 0 to
    /// `count() - 1`, without allocating. O(rank).
    pub(crate) fn position_at(&self, k: usize) -> Option<usize> {
        let mut rest = k;
        let mut position = self.offset;
        for axis in (0..self.shape.rank).rev() {
            let length = self.shape.get(axis)?;
            let stride = self.strides.get(axis)?;
            if length == 0 {
                return None;
            }
            position = position.checked_add((rest % length).checked_mul(stride)?)?;
            rest /= length;
        }
        (rest == 0).then_some(position)
    }

    /// The rows and columns of a matrix.
    fn matrix_shape(&self) -> (usize, usize) {
        (
            self.shape.get(0).unwrap_or(0),
            self.shape.get(1).unwrap_or(0),
        )
    }
}

/// The elements of a dense value (D1).
#[derive(Debug)]
pub(crate) enum Store {
    /// Canonical `F64` elements, raw: shared by the views of one value and by the copies of a
    /// `together` block, and written only while held once (ADR 0120 rule 28).
    F64(Arc<Vec<f64>>),
    /// Every other element, as words: a handle to an [`Object::Words`] of this task's heap,
    /// which the header references as its one cell.
    Words(i64),
}

/// A dense value's header: its layout over its store (D1).
#[derive(Debug)]
pub(crate) struct Dense {
    /// Where the elements are.
    pub(crate) layout: Layout,
    /// The elements.
    pub(crate) store: Store,
}

impl Dense {
    /// The words the header references: the words store, or none.
    pub(crate) fn cells(&self) -> &[i64] {
        match &self.store {
            Store::Words(words) => std::slice::from_ref(words),
            Store::F64(_) => &[],
        }
    }

    /// A copy of the header with the same store: the `Arc` is cloned, and the words store's
    /// handle is copied without a reference, which [`view`] adds when it keeps it.
    fn share(&self) -> Dense {
        Dense {
            layout: self.layout,
            store: match &self.store {
                Store::F64(items) => Store::F64(Arc::clone(items)),
                Store::Words(words) => Store::Words(*words),
            },
        }
    }

    /// The `F64` elements' storage: the store, or nothing for a value of no elements, whatever
    /// its store, as the interpreter's `f64_iter` reads an empty array of values.
    fn floats(&self) -> Option<&[f64]> {
        match &self.store {
            Store::F64(items) => Some(items.as_slice()),
            Store::Words(_) if self.layout.count() == 0 => Some(&[]),
            Store::Words(_) => None,
        }
    }

    /// The `F64` elements in row-major order, read in place: `None` when the store does not
    /// hold raw `F64` values, which checking excludes for the float methods.
    fn f64_iter(&self) -> Option<impl Iterator<Item = f64> + Clone + '_> {
        let items = self.floats()?;
        let layout = self.layout;
        Some(
            (0..layout.count())
                .filter_map(move |k| layout.position_at(k))
                .map(move |p| items.get(p).copied().unwrap_or(f64::NAN)),
        )
    }

    /// The `F64` elements as one row-major run of the store, when the value is laid out so: a
    /// compact value, or a row or a slice of one. O(rank).
    fn as_slice(&self) -> Option<&[f64]> {
        let Store::F64(items) = &self.store else {
            return None;
        };
        if !self.layout.contiguous() {
            return None;
        }
        items.get(self.layout.offset..self.layout.offset.checked_add(self.layout.count())?)
    }

    /// The `F64` element at `indices`, read in place.
    fn f64_at(&self, indices: &[usize]) -> Option<f64> {
        match (&self.store, self.layout.position(indices)) {
            (Store::F64(items), Some(p)) => items.get(p).copied(),
            _ => None,
        }
    }

    /// The element at position `p` of the store.
    fn element(&self, p: usize) -> Option<Element> {
        match &self.store {
            Store::F64(items) => items.get(p).copied().map(Element::Raw),
            Store::Words(words) => word_at(*words, p).map(Element::Word),
        }
    }
}

/// An element as its store holds it: a raw `F64`, or a word borrowed from the store.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Element {
    /// A canonical `F64`.
    Raw(f64),
    /// A word.
    Word(i64),
}

/// What rendering and equality read of a dense value: its layout and its elements in
/// row-major order, the words borrowed from the store.
pub(crate) struct Parts {
    /// The value's kind and shape.
    pub(crate) layout: Layout,
    /// Its elements, in row-major order.
    pub(crate) elements: Vec<Element>,
}

/// The kind, shape and row-major elements of the dense value `word`, or `None` when `word` is
/// no dense value. O(size).
pub(crate) fn parts(word: i64) -> Option<Parts> {
    let header = header(word)?;
    let layout = header.layout;
    let elements = (0..layout.count())
        .filter_map(|k| layout.position_at(k))
        .filter_map(|p| header.element(p))
        .collect();
    Some(Parts { layout, elements })
}

/// The `F64` of a word that holds a boxed one.
pub(crate) fn boxed(word: i64) -> Option<f64> {
    if !is_handle(word) {
        return None;
    }
    with(word, |object| match object {
        Object::F64(x) => Some(*x),
        _ => None,
    })
    .flatten()
}

/// `f` of the object of `word`, mutably: for an update in place by its only holder.
fn with_mut<R>(word: i64, f: impl FnOnce(&mut Object) -> R) -> Option<R> {
    let index = slot(word)?;
    HEAP.with(|heap| {
        heap.borrow_mut()
            .slots
            .get_mut(index)
            .and_then(|slot| slot.object.as_mut())
            .map(f)
    })
}

/// A copy of the header of the dense value `word`.
fn header(word: i64) -> Option<Dense> {
    with(word, |object| match object {
        Object::Dense(dense) => Some(dense.share()),
        _ => None,
    })
    .flatten()
}

/// The layout of the dense value `word` and its words store, if any: no `Arc` is cloned, so a
/// store held once stays so.
fn layout(word: i64) -> Option<(Layout, Option<i64>)> {
    with(word, |object| match object {
        Object::Dense(dense) => Some((
            dense.layout,
            match dense.store {
                Store::Words(words) => Some(words),
                Store::F64(_) => None,
            },
        )),
        _ => None,
    })
    .flatten()
}

/// Word `p` of the words store `words`, borrowed.
fn word_at(words: i64, p: usize) -> Option<i64> {
    with(words, |object| match object {
        Object::Words(items) => items.get(p).copied(),
        _ => None,
    })
    .flatten()
}

/// A word the caller owns for `element`: a new boxed `F64`, or the word retained.
fn owned_word(element: Element) -> i64 {
    match element {
        Element::Raw(x) => f64_word(x),
        Element::Word(word) => {
            retain(word);
            word
        }
    }
}

/// The word `f` makes, or 0 when it makes none, after it reported why: a refusal, an abort or a
/// fault.
fn word(f: impl FnOnce() -> Option<i64>) -> i64 {
    f().unwrap_or(0)
}

/// The site of a call, as the runtime indexes it.
fn site_of(site: i64) -> usize {
    usize::try_from(site).unwrap_or(usize::MAX)
}

/// The arguments of the wrong types, which checking excludes: a fault, which the debug runtime
/// reports, and no value.
fn bad<T>() -> Option<T> {
    fault();
    None
}

/// The call at `site` over the memory budget: U6003 there, never an abort of the process (D2,
/// ADR 0120 Security).
fn over<T>(site: i64) -> Option<T> {
    crate::runtime::refuse(site_of(site), None);
    None
}

/// A precondition that does not hold at `site`, which only a false proof reaches: U8001 with
/// the interpreter's message (D4, ADR 0112 EL-2, rule 30).
fn outside<T>(site: i64, message: &str) -> Option<T> {
    fault();
    crate::runtime::abort_at(
        site_of(site),
        una_diag::Code::U8001,
        format!("evaluation stopped: {message}"),
    );
    None
}

/// A vector with room for `n` elements of `T`, reserved within the budget with `extra` bytes
/// more and allocated fallibly, or `None` after the refusal at `site` (D2): the bytes are
/// computed in checked arithmetic, so a size whose computation overflows is refused.
fn buffer<T>(site: i64, n: usize, extra: usize) -> Option<Vec<T>> {
    let bytes = n
        .checked_mul(size_of::<T>())
        .and_then(|bytes| bytes.checked_add(extra));
    let Some(reservation) = bytes.and_then(crate::runtime::try_reserve) else {
        return over(site);
    };
    let mut out = Vec::new();
    let made = out.try_reserve_exact(n);
    drop(reservation);
    if made.is_err() {
        return over(site);
    }
    Some(out)
}

/// A store of raw `F64` elements.
fn f64_store(items: Vec<f64>) -> Store {
    Store::F64(Arc::new(items))
}

/// A store of `items`, words the store now owns.
fn words_store(items: Vec<i64>) -> Store {
    Store::Words(alloc(Object::Words(items)))
}

/// A new contiguous row-major value of `shape` over `store`.
fn compact(kind: DenseKind, shape: Axes, store: Store) -> i64 {
    alloc(Object::Dense(Box::new(Dense {
        layout: Layout::compact(kind, shape),
        store,
    })))
}

/// A new header `header`, a view over a store another value holds: O(1), the store shared
/// (rule 29).
fn view(header: Dense) -> i64 {
    if let Store::Words(words) = &header.store {
        retain(*words);
    }
    alloc(Object::Dense(Box::new(header)))
}

/// The new value of a method that takes its receiver by `edit`, and its result, `unit`
/// (`spec/17 §4`): the tuple that takes the caller's reference to `word`.
fn updated(word: i64) -> i64 {
    let tuple = data(0);
    push(tuple, word);
    release(word);
    push(tuple, 0);
    tuple
}

/// Adds `n` references to `word` at once, `false` when its count would pass its limit, which
/// the caller refuses rather than end the process. Immediates have none.
fn retain_many(word: i64, n: usize) -> bool {
    let Some(index) = slot(word) else {
        return true;
    };
    let Ok(n) = u32::try_from(n) else {
        return false;
    };
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        match heap.slots.get_mut(index) {
            Some(slot) if slot.object.is_some() => match slot.count.checked_add(n) {
                Some(count) if count <= count_limit() => {
                    slot.count = count;
                    true
                }
                _ => false,
            },
            _ => {
                heap.faults += 1;
                true
            }
        }
    })
}

/// The heap a copy of `word` would allocate of its own: what the interpreter reserves for each
/// clone of an element (`reserve_with`), so a fill is refused in both engines alike (D2).
fn element_heap(word: i64) -> usize {
    if !is_handle(word) {
        return 0;
    }
    with(word, Object::node_bytes).unwrap_or(0)
}

/// A dimension argument, an `Int` that the callee's precondition keeps at least 0. A
/// dimension beyond what this machine can index is refused by the memory budget, U6003, as in
/// the interpreter.
fn dimension(site: i64, word: i64) -> Option<usize> {
    let n = int_value(word);
    if n.sign() == num_bigint::Sign::Minus {
        return bad();
    }
    match usize::try_from(&n) {
        Ok(n) if isize::try_from(n).is_ok() => Some(n),
        _ => over(site),
    }
}

/// An index argument, `None` when it is negative or beyond any length.
fn index(word: i64) -> Option<usize> {
    usize::try_from(&int_value(word)).ok()
}

/// The indices of `words`, each an index argument.
fn indices(words: &[i64]) -> Option<Axes> {
    let mut out = [0; MAX_RANK];
    if words.len() > MAX_RANK {
        return None;
    }
    for (slot, word) in out.iter_mut().zip(words) {
        *slot = index(*word)?;
    }
    Some(Axes {
        rank: words.len(),
        lengths: out,
    })
}

/// The header of a dense argument; a fault for any other word.
fn dense(word: i64) -> Option<Dense> {
    match header(word) {
        Some(header) => Some(header),
        None => bad(),
    }
}

/// An `F64` argument.
fn scalar(word: i64) -> Option<f64> {
    match boxed(word) {
        Some(x) => Some(x),
        None => bad(),
    }
}

/// The store of the elements of the list `xs`, raw when every one is an `F64`, reserved before
/// any is copied. The list's items are read in place, one at a time.
fn from_list(site: i64, xs: i64) -> Option<(Store, usize)> {
    let n = usize::try_from(count(xs)).unwrap_or(0);
    let item = |i: usize| field(xs, i64::try_from(i).unwrap_or(i64::MAX));
    let floats = n > 0 && (0..n).all(|i| boxed(item(i)).is_some());
    if floats {
        let mut out = buffer::<f64>(site, n, 0)?;
        out.extend((0..n).map(|i| float::canonical(boxed(item(i)).unwrap_or(0.0))));
        Some((f64_store(out), n))
    } else {
        let mut out = buffer::<i64>(site, n, 0)?;
        for i in 0..n {
            let word = item(i);
            retain(word);
            out.push(word);
        }
        Some((words_store(out), n))
    }
}

/// `n` copies of `value`: raw for an `F64`; otherwise the word, held `n` more times, with the
/// heap of `n` clones of it reserved as the interpreter does (D2).
fn filled(site: i64, n: usize, value: i64) -> Option<Store> {
    if let Some(x) = boxed(value) {
        let mut out = buffer::<f64>(site, n, 0)?;
        out.resize(n, float::canonical(x));
        return Some(f64_store(out));
    }
    let Some(extra) = n.checked_mul(element_heap(value)) else {
        return over(site);
    };
    let mut out = buffer::<i64>(site, n, extra)?;
    if !retain_many(value, n) {
        return over(site);
    }
    out.resize(n, value);
    Some(words_store(out))
}

/// A compact copy of the elements of `header`, in its representation, reserved before any is
/// copied: a copy on write, `compact`, a reshape of a strided tensor. O(size).
fn copy(site: i64, header: &Dense) -> Option<Store> {
    let layout = header.layout;
    let n = layout.count();
    match &header.store {
        Store::F64(items) => {
            let mut out = buffer::<f64>(site, n, 0)?;
            out.extend(
                (0..n)
                    .filter_map(|k| layout.position_at(k))
                    .filter_map(|p| items.get(p).copied()),
            );
            Some(f64_store(out))
        }
        Store::Words(words) => {
            let mut out = buffer::<i64>(site, n, 0)?;
            for p in (0..n).filter_map(|k| layout.position_at(k)) {
                if let Some(word) = word_at(*words, p) {
                    retain(word);
                    out.push(word);
                }
            }
            Some(words_store(out))
        }
    }
}

/// Whether the store of the dense value `word` is held once: an `Arc` of count 1, or a words
/// store nothing else references. No `Arc` is cloned to find out.
fn store_held_once(word: i64) -> bool {
    let store = with(word, |object| match object {
        Object::Dense(dense) => Some(match &dense.store {
            Store::F64(items) => Ok(Arc::strong_count(items) == 1 && Arc::weak_count(items) == 0),
            Store::Words(words) => Err(*words),
        }),
        _ => None,
    })
    .flatten();
    match store {
        Some(Ok(once)) => once,
        Some(Err(words)) => unique(words),
        None => false,
    }
}

/// The dense value `word`, whose reference the caller gives up, held once with its store held
/// once, so it can be updated in place: itself when it is so, and a compact copy of its own
/// elements otherwise, after the budget check (D3, rule 31). O(1), or O(size) for the copy.
fn owned(site: i64, word: i64) -> Option<i64> {
    if unique(word) && store_held_once(word) {
        return Some(word);
    }
    let header = dense(word)?;
    let Some(store) = copy(site, &header) else {
        release(word);
        return None;
    };
    copied();
    let made = compact(header.layout.kind, header.layout.shape, store);
    drop(header);
    release(word);
    Some(made)
}

/// Writes `item` at position `p` of the store of `word`, which the caller holds once with its
/// store: a canonical `F64` into a raw store, any word into a words store. `false` when the
/// representation does not take the item or the store is shared, which [`owned`] excludes.
fn write(word: i64, p: usize, item: i64) -> bool {
    let Some((_, words)) = layout(word) else {
        return false;
    };
    match words {
        None => {
            let Some(x) = boxed(item) else {
                return false;
            };
            with_mut(word, |object| match object {
                Object::Dense(dense) => match &mut dense.store {
                    Store::F64(items) => {
                        match Arc::get_mut(items).and_then(|items| items.get_mut(p)) {
                            Some(slot) => {
                                *slot = float::canonical(x);
                                true
                            }
                            None => false,
                        }
                    }
                    Store::Words(_) => false,
                },
                _ => false,
            })
            .unwrap_or(false)
        }
        Some(words) => {
            if !unique(words) {
                return false;
            }
            retain(item);
            let old = with_mut(words, |object| match object {
                Object::Words(items) => items.get_mut(p).map(|slot| std::mem::replace(slot, item)),
                _ => None,
            })
            .flatten();
            match old {
                Some(old) => {
                    release(old);
                    true
                }
                None => {
                    release(item);
                    false
                }
            }
        }
    }
}

/// The `F64` elements of `header` in row-major order, in a vector reserved within the budget
/// with room for `extra` more: the workspace of a factorization or a solve (ADR 0120,
/// Security).
fn f64_copy(site: i64, header: &Dense, extra: usize) -> Option<Vec<f64>> {
    let Some(items) = header.f64_iter() else {
        return bad();
    };
    let Some(n) = header.layout.count().checked_add(extra) else {
        return over(site);
    };
    let mut out = buffer::<f64>(site, n, 0)?;
    out.extend(items);
    Some(out)
}

/// The `F64` elements of `header` in row-major order as one slice: its own store when it is
/// contiguous, and otherwise a compact copy reserved within the budget, the kernel's
/// workspace. O(1) or O(size).
fn slice_of(site: i64, header: &Dense) -> Option<Cow<'_, [f64]>> {
    match header.as_slice() {
        Some(items) => Some(Cow::Borrowed(items)),
        None => Some(Cow::Owned(f64_copy(site, header, 0)?)),
    }
}

/// A dense value of `shape` from row-major `F64` values, reserved before any is computed.
fn made_f64(
    site: i64,
    kind: DenseKind,
    shape: Axes,
    values: impl Iterator<Item = f64>,
) -> Option<i64> {
    let mut out = buffer::<f64>(site, shape.count(), 0)?;
    out.extend(values.map(float::canonical));
    Some(compact(kind, shape, f64_store(out)))
}

/// A kernel's `ensures` on its result's shape, checked before it returns (rule 53): a
/// violation is an internal abort, so a kernel bug never gives the verifier a false fact.
fn ensure(site: i64, word: i64, shape: &[usize], method: &str) -> Option<i64> {
    match layout(word) {
        Some((layout, _)) if layout.shape.as_slice() == shape => Some(word),
        _ => {
            release(word);
            outside(
                site,
                &format!("internal error: `{method}` gave a result of another shape than it ensures"),
            )
        }
    }
}

/// The messages of the preconditions, the interpreter's (D4).
const GET_OUTSIDE: &str =
    "`get` at an index outside the dense value, which its precondition excludes";
const SET_OUTSIDE: &str =
    "`set` at an index outside the dense value, which its precondition excludes";
const OTHER_RANK: &str =
    "a list literal of another length than the tensor's rank, which the precondition excludes";
const SLICE_OUTSIDE: &str = "`slice` outside the array, which its precondition excludes";
const LINE_OUTSIDE: &str = "a row or column outside the matrix, which its precondition excludes";
const BLOCK_OUTSIDE: &str = "`block` outside the matrix, which its precondition excludes";
const DIFFERENT_SHAPES: &str = "an element-wise operation of dense values of different shapes, which its precondition excludes";
const MULTIPLY_SHAPES: &str =
    "`multiply` of matrices whose shapes do not match, which its precondition excludes";
const MULTIPLY_ARRAY_SHAPES: &str = "`multiply_array` of a matrix and an array whose shapes do not match, which its precondition excludes";
const ADD_PRODUCT_SHAPES: &str =
    "`add_product` of matrices whose shapes do not match, which its precondition excludes";
const CONVERSION_RANK: &str =
    "a tensor of another rank than the conversion takes, which its precondition excludes";
const DIM_OUTSIDE: &str = "`dim` of an axis the tensor does not have, which its precondition excludes";
const RESHAPE_COUNT: &str =
    "`reshape` to another number of elements, which its precondition excludes";
const SUM_AXIS_OUTSIDE: &str =
    "`sum` along an axis the tensor does not have, which its precondition excludes";
const CONTRACT_AXES: &str =
    "`contract` over axes whose lengths do not match, which its precondition excludes";
const SOLVE_ROWS: &str = "`solve` with a right-hand side of another number of rows, which its precondition excludes";
const TRIANGULAR_ROWS: &str = "a triangular solve with a right-hand side of another number of rows, which its precondition excludes";
const MATRIX_COUNT: &str = "`matrix` with a number of elements other than `rows * columns`, which its precondition excludes";
const TENSOR_COUNT: &str = "`tensor` with a number of elements other than the product of its lengths, which its precondition excludes";

/// The precondition "same shape" of rule 34 at `site`.
fn same_shape(site: i64, a: &Dense, b: &Dense) -> Option<()> {
    if a.layout.shape == b.layout.shape {
        Some(())
    } else {
        outside(site, DIFFERENT_SHAPES)
    }
}

/// The size of a square matrix, or the precondition of `name` that fails.
fn square(site: i64, header: &Dense, name: &str) -> Option<usize> {
    let (rows, columns) = header.layout.matrix_shape();
    if rows == columns {
        Some(rows)
    } else {
        outside(
            site,
            &format!("`{name}` of a matrix that is not square, which its precondition excludes"),
        )
    }
}

// ----- construction (rule 32) -----

/// `array(xs)`: O(n).
pub(crate) fn array(site: i64, xs: i64) -> i64 {
    from_list(site, xs)
        .map(|(store, n)| compact(DenseKind::Array, Axes::one(n), store))
        .unwrap_or(0)
}

/// `array_filled(length:, value:)`: O(n).
pub(crate) fn array_filled(site: i64, length: i64, value: i64) -> i64 {
    word(|| {
        let n = dimension(site, length)?;
        let store = filled(site, n, value)?;
        Some(compact(DenseKind::Array, Axes::one(n), store))
    })
}

/// `matrix(rows:, columns:, elements:)`: O(rows × columns).
pub(crate) fn matrix(site: i64, rows: i64, columns: i64, elements: i64) -> i64 {
    word(|| {
        let rows = dimension(site, rows)?;
        let columns = dimension(site, columns)?;
        let n = usize::try_from(count(elements)).unwrap_or(0);
        if rows.checked_mul(columns) != Some(n) {
            return outside(site, MATRIX_COUNT);
        }
        let (store, _) = from_list(site, elements)?;
        Some(compact(DenseKind::Matrix, Axes::two(rows, columns), store))
    })
}

/// `matrix_filled(rows:, columns:, value:)`: O(rows × columns).
pub(crate) fn matrix_filled(site: i64, rows: i64, columns: i64, value: i64) -> i64 {
    word(|| {
        let rows = dimension(site, rows)?;
        let columns = dimension(site, columns)?;
        let Some(n) = rows.checked_mul(columns) else {
            return over(site);
        };
        let store = filled(site, n, value)?;
        Some(compact(DenseKind::Matrix, Axes::two(rows, columns), store))
    })
}

/// `matrix_diagonal(values)`: the n × n matrix with `values` on its diagonal and `+0` of the
/// element type elsewhere. O(n²).
pub(crate) fn matrix_diagonal(site: i64, values: i64) -> i64 {
    word(|| {
        let header = dense(values)?;
        let n = header.layout.count();
        let Some(size) = n.checked_mul(n) else {
            return over(site);
        };
        let first = header
            .layout
            .position_at(0)
            .and_then(|p| header.element(p));
        let store = match first {
            Some(Element::Raw(_)) | None => {
                let mut out = buffer::<f64>(site, size, 0)?;
                out.resize(size, 0.0);
                let Some(items) = header.floats() else {
                    return bad();
                };
                for i in 0..n {
                    let x = header
                        .layout
                        .position_at(i)
                        .and_then(|p| items.get(p).copied())
                        .unwrap_or(0.0);
                    if let Some(slot) = out.get_mut(i * n + i) {
                        *slot = x;
                    }
                }
                f64_store(out)
            }
            Some(Element::Word(_)) => {
                let Store::Words(words) = &header.store else {
                    return bad();
                };
                let words = *words;
                // The tagged `0` off the diagonal, an immediate.
                let mut out = buffer::<i64>(site, size, 0)?;
                out.resize(size, 1);
                for i in 0..n {
                    let item = header
                        .layout
                        .position_at(i)
                        .and_then(|p| word_at(words, p))
                        .unwrap_or(1);
                    if let Some(slot) = out.get_mut(i * n + i) {
                        retain(item);
                        *slot = item;
                    }
                }
                words_store(out)
            }
        };
        Some(compact(DenseKind::Matrix, Axes::two(n, n), store))
    })
}

/// A tensor of the lengths `dims`, then the list `elements` of its elements (rule 32).
/// O(size).
fn tensor_make(site: i64, dims: &[i64], elements: i64) -> i64 {
    word(|| {
        let shape = tensor_shape(site, dims)?;
        let Some(n) = shape.checked_count() else {
            return over(site);
        };
        if usize::try_from(count(elements)).ok() != Some(n) {
            return outside(site, TENSOR_COUNT);
        }
        let (store, _) = from_list(site, elements)?;
        Some(compact(DenseKind::Tensor, shape, store))
    })
}

/// A tensor of the lengths `dims`, each element `value` (rule 32). O(size).
fn tensor_filled_of(site: i64, dims: &[i64], value: i64) -> i64 {
    word(|| {
        let shape = tensor_shape(site, dims)?;
        let Some(n) = shape.checked_count() else {
            return over(site);
        };
        let store = filled(site, n, value)?;
        Some(compact(DenseKind::Tensor, shape, store))
    })
}

/// The shape of a tensor from 1 to 8 dimension arguments.
fn tensor_shape(site: i64, dims: &[i64]) -> Option<Axes> {
    if !(1..=MAX_RANK).contains(&dims.len()) {
        return bad();
    }
    let mut lengths = [0; MAX_RANK];
    for (slot, word) in lengths.iter_mut().zip(dims) {
        *slot = dimension(site, *word)?;
    }
    Some(Axes {
        rank: dims.len(),
        lengths,
    })
}

/// The words of the list `xs` of 1 to 8 lengths, the shape argument of `tensor`.
fn shape_list(xs: i64) -> Option<Vec<i64>> {
    let n = usize::try_from(count(xs)).ok()?;
    if !(1..=MAX_RANK).contains(&n) {
        return bad();
    }
    Some(
        (0..n)
            .map(|i| field(xs, i64::try_from(i).unwrap_or(i64::MAX)))
            .collect(),
    )
}

/// `tensor(shape:, elements:)` with the shape as a list (rule 32).
pub(crate) fn tensor(site: i64, shape: i64, elements: i64) -> i64 {
    match shape_list(shape) {
        Some(dims) => tensor_make(site, &dims, elements),
        None => 0,
    }
}

/// `tensor_filled(shape:, value:)` with the shape as a list (rule 32).
pub(crate) fn tensor_filled(site: i64, shape: i64, value: i64) -> i64 {
    match shape_list(shape) {
        Some(dims) => tensor_filled_of(site, &dims, value),
        None => 0,
    }
}

// ----- access (rule 33) -----

/// The element of `d` at the indices `at`, which a precondition keeps in range: O(rank). A
/// tensor's function of one rank checks that rank first.
fn get(site: i64, d: i64, at: &[i64], tensor: bool) -> i64 {
    word(|| {
        let header = dense(d)?;
        if tensor && at.len() != header.layout.shape.rank() {
            return outside(site, OTHER_RANK);
        }
        let p = indices(at).and_then(|at| header.layout.position(at.as_slice()));
        match p.and_then(|p| header.element(p)) {
            Some(element) => Some(owned_word(element)),
            None => outside(site, GET_OUTSIDE),
        }
    })
}

/// `Some` of the element of `d` at the indices `at`, or `None` outside the shape, or for a
/// tensor's function of another rank. O(rank).
fn at(d: i64, at: &[i64], tensor: bool) -> i64 {
    let Some(header) = dense(d) else {
        return NONE;
    };
    if tensor && at.len() != header.layout.shape.rank() {
        return NONE;
    }
    let p = indices(at).and_then(|at| header.layout.position(at.as_slice()));
    match p.and_then(|p| header.element(p)) {
        Some(element) => {
            let word = owned_word(element);
            let option = some(word);
            release(word);
            option
        }
        None => NONE,
    }
}

/// `set(edit d, at..., item)`: the tuple of the new value and `unit`. With `give`, the caller
/// gives up its reference to `d`, which is updated in place when it and its store are held
/// once (D3); otherwise the value's own elements are copied first (rule 31). O(rank), or
/// O(size) for the copy.
fn set(site: i64, d: i64, at: &[i64], item: i64, give: bool, tensor: bool) -> i64 {
    word(|| {
        let (layout, _) = match layout(d) {
            Some(found) => found,
            None => return bad(),
        };
        if tensor && at.len() != layout.shape.rank() {
            return outside(site, OTHER_RANK);
        }
        if !give {
            retain(d);
        }
        let owned = owned(site, d)?;
        let (layout, _) = self::layout(owned)?;
        let Some(p) = indices(at).and_then(|at| layout.position(at.as_slice())) else {
            release(owned);
            return outside(site, SET_OUTSIDE);
        };
        if !write(owned, p, item) {
            fault();
        }
        Some(updated(owned))
    })
}

/// `a.get(index:)`.
pub(crate) fn array_get(site: i64, a: i64, index: i64) -> i64 {
    get(site, a, &[index], false)
}

/// `a.at(index:)`.
pub(crate) fn array_at(_site: i64, a: i64, index: i64) -> i64 {
    at(a, &[index], false)
}

/// `a.set(index:, item:)`, by copy.
pub(crate) fn array_set(site: i64, a: i64, index: i64, item: i64) -> i64 {
    set(site, a, &[index], item, false, false)
}

/// `a.set(index:, item:)` at the array's last use: in place when held once.
pub(crate) fn array_set_owned(site: i64, a: i64, index: i64, item: i64) -> i64 {
    set(site, a, &[index], item, true, false)
}

/// `m.get(row:, column:)`.
pub(crate) fn matrix_get(site: i64, m: i64, row: i64, column: i64) -> i64 {
    get(site, m, &[row, column], false)
}

/// `m.at(row:, column:)`.
pub(crate) fn matrix_at(_site: i64, m: i64, row: i64, column: i64) -> i64 {
    at(m, &[row, column], false)
}

/// `m.set(row:, column:, item:)`, by copy.
pub(crate) fn matrix_set(site: i64, m: i64, row: i64, column: i64, item: i64) -> i64 {
    set(site, m, &[row, column], item, false, false)
}

/// `m.set(row:, column:, item:)` at the matrix's last use: in place when held once.
pub(crate) fn matrix_set_owned(site: i64, m: i64, row: i64, column: i64, item: i64) -> i64 {
    set(site, m, &[row, column], item, true, false)
}

/// The functions of a tensor of each rank, one-liners over the slice-based functions (D5): no
/// allocation per access, and no pointer across the boundary.
macro_rules! ranks {
    ($($make:ident $filled:ident $get:ident $at:ident $set:ident $set_owned:ident
        $permute:ident $reshape:ident ($($i:ident)*);)*) => {$(
        #[doc = concat!("`", stringify!($make), "`: a tensor of these lengths and elements.")]
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $make(site: i64, $($i: i64,)* elements: i64) -> i64 {
            tensor_make(site, &[$($i),*], elements)
        }
        #[doc = concat!("`", stringify!($filled), "`: a tensor of these lengths, filled.")]
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $filled(site: i64, $($i: i64,)* value: i64) -> i64 {
            tensor_filled_of(site, &[$($i),*], value)
        }
        #[doc = concat!("`", stringify!($get), "`: the element at these indices.")]
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $get(site: i64, t: i64, $($i: i64),*) -> i64 {
            get(site, t, &[$($i),*], true)
        }
        #[doc = concat!("`", stringify!($at), "`: `Some` of the element, or `None`.")]
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $at(_site: i64, t: i64, $($i: i64),*) -> i64 {
            at(t, &[$($i),*], true)
        }
        #[doc = concat!("`", stringify!($set), "`, by copy.")]
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $set(site: i64, t: i64, $($i: i64,)* item: i64) -> i64 {
            set(site, t, &[$($i),*], item, false, true)
        }
        #[doc = concat!("`", stringify!($set), "` at the tensor's last use.")]
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $set_owned(site: i64, t: i64, $($i: i64,)* item: i64) -> i64 {
            set(site, t, &[$($i),*], item, true, true)
        }
        #[doc = concat!("`", stringify!($permute), "`: a view with its axes permuted.")]
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $permute(site: i64, t: i64, $($i: i64),*) -> i64 {
            permute(site, t, &[$($i),*])
        }
        #[doc = concat!("`", stringify!($reshape), "`: the elements in another shape.")]
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $reshape(site: i64, t: i64, $($i: i64),*) -> i64 {
            reshape(site, t, &[$($i),*])
        }
    )*};
}

ranks! {
    tensor_make_1 tensor_filled_1 tensor_get_1 tensor_at_1 tensor_set_1 tensor_set_1_owned
        tensor_permute_1 tensor_reshape_1 (a0);
    tensor_make_2 tensor_filled_2 tensor_get_2 tensor_at_2 tensor_set_2 tensor_set_2_owned
        tensor_permute_2 tensor_reshape_2 (a0 a1);
    tensor_make_3 tensor_filled_3 tensor_get_3 tensor_at_3 tensor_set_3 tensor_set_3_owned
        tensor_permute_3 tensor_reshape_3 (a0 a1 a2);
    tensor_make_4 tensor_filled_4 tensor_get_4 tensor_at_4 tensor_set_4 tensor_set_4_owned
        tensor_permute_4 tensor_reshape_4 (a0 a1 a2 a3);
    tensor_make_5 tensor_filled_5 tensor_get_5 tensor_at_5 tensor_set_5 tensor_set_5_owned
        tensor_permute_5 tensor_reshape_5 (a0 a1 a2 a3 a4);
    tensor_make_6 tensor_filled_6 tensor_get_6 tensor_at_6 tensor_set_6 tensor_set_6_owned
        tensor_permute_6 tensor_reshape_6 (a0 a1 a2 a3 a4 a5);
    tensor_make_7 tensor_filled_7 tensor_get_7 tensor_at_7 tensor_set_7 tensor_set_7_owned
        tensor_permute_7 tensor_reshape_7 (a0 a1 a2 a3 a4 a5 a6);
    tensor_make_8 tensor_filled_8 tensor_get_8 tensor_at_8 tensor_set_8 tensor_set_8_owned
        tensor_permute_8 tensor_reshape_8 (a0 a1 a2 a3 a4 a5 a6 a7);
}

// ----- views (rule 29): O(1) or O(rank), the store shared -----

/// `a.slice(from:, until:)`: the elements from `from` until `until`, a view.
pub(crate) fn array_slice(site: i64, a: i64, from: i64, until: i64) -> i64 {
    word(|| {
        let mut header = dense(a)?;
        let length = header.layout.shape.get(0).unwrap_or(0);
        let stride = header.layout.strides.get(0).unwrap_or(1);
        match (index(from), index(until)) {
            (Some(from), Some(until)) if from <= until && until <= length => {
                header.layout.offset = header
                    .layout
                    .offset
                    .saturating_add(from.saturating_mul(stride));
                header.layout.shape = Axes::one(until - from);
                header.layout.strides = Axes::one(stride);
                Some(view(header))
            }
            _ => outside(site, SLICE_OUTSIDE),
        }
    })
}

/// `m.row(index:)` or, for `axis` 1, `m.column(index:)`: an array view.
fn line(site: i64, m: i64, i: i64, axis: usize) -> i64 {
    word(|| {
        let mut header = dense(m)?;
        let other = 1 - axis;
        let layout = header.layout;
        let Some(i) = index(i).filter(|i| layout.shape.get(axis).is_some_and(|n| *i < n)) else {
            return outside(site, LINE_OUTSIDE);
        };
        let stride = layout.strides.get(axis).unwrap_or(0);
        header.layout = Layout {
            kind: DenseKind::Array,
            offset: layout.offset.saturating_add(i.saturating_mul(stride)),
            shape: Axes::one(layout.shape.get(other).unwrap_or(0)),
            strides: Axes::one(layout.strides.get(other).unwrap_or(0)),
        };
        Some(view(header))
    })
}

/// `m.row(index:)`.
pub(crate) fn matrix_row(site: i64, m: i64, i: i64) -> i64 {
    line(site, m, i, 0)
}

/// `m.column(index:)`.
pub(crate) fn matrix_column(site: i64, m: i64, i: i64) -> i64 {
    line(site, m, i, 1)
}

/// `m.block(row_from:, row_until:, column_from:, column_until:)`: a view.
pub(crate) fn matrix_block(site: i64, m: i64, i: i64, j: i64, k: i64, l: i64) -> i64 {
    word(|| {
        let mut header = dense(m)?;
        let layout = header.layout;
        let (rows, columns) = layout.matrix_shape();
        match (index(i), index(j), index(k), index(l)) {
            (Some(i), Some(j), Some(k), Some(l))
                if i <= j && j <= rows && k <= l && l <= columns =>
            {
                // An empty block starts anywhere; its offset is never read.
                header.layout.offset = if i < rows && k < columns {
                    layout.position(&[i, k]).unwrap_or(layout.offset)
                } else {
                    layout.offset
                };
                header.layout.shape = Axes::two(j - i, l - k);
                Some(view(header))
            }
            _ => outside(site, BLOCK_OUTSIDE),
        }
    })
}

/// `m.transpose()`: a view with the shape and strides reversed.
pub(crate) fn matrix_transpose(_site: i64, m: i64) -> i64 {
    match dense(m) {
        Some(mut header) => {
            header.layout.shape = header.layout.shape.reversed();
            header.layout.strides = header.layout.strides.reversed();
            view(header)
        }
        None => 0,
    }
}

/// A view of `d` as another dense type of `rank` axes (rule 39), any rank for `None`.
fn convert(site: i64, d: i64, kind: DenseKind, rank: Option<usize>) -> i64 {
    word(|| {
        let mut header = dense(d)?;
        if rank.is_some_and(|rank| rank != header.layout.shape.rank()) {
            return outside(site, CONVERSION_RANK);
        }
        header.layout.kind = kind;
        Some(view(header))
    })
}

/// `a.to_tensor()`: O(1).
pub(crate) fn array_to_tensor(site: i64, a: i64) -> i64 {
    convert(site, a, DenseKind::Tensor, None)
}

/// `m.to_tensor()`: O(1).
pub(crate) fn matrix_to_tensor(site: i64, m: i64) -> i64 {
    convert(site, m, DenseKind::Tensor, None)
}

/// `t.to_array()` of a tensor of rank 1: O(1).
pub(crate) fn tensor_to_array(site: i64, t: i64) -> i64 {
    convert(site, t, DenseKind::Array, Some(1))
}

/// `t.to_matrix()` of a tensor of rank 2: O(1).
pub(crate) fn tensor_to_matrix(site: i64, t: i64) -> i64 {
    convert(site, t, DenseKind::Matrix, Some(2))
}

/// `t.dim(axis:)`: the length of an axis.
pub(crate) fn tensor_dim(site: i64, t: i64, axis: i64) -> i64 {
    word(|| {
        let header = dense(t)?;
        let rank = header.layout.shape.rank();
        let Some(axis) = index(axis).filter(|axis| *axis < rank) else {
            return outside(site, DIM_OUTSIDE);
        };
        Some(int_word(&BigInt::from(
            header.layout.shape.get(axis).unwrap_or(0),
        )))
    })
}

/// A view with the axes and their strides in the order of the permutation `axes` (rule 29):
/// O(rank).
fn permute(site: i64, t: i64, axes: &[i64]) -> i64 {
    word(|| {
        let mut header = dense(t)?;
        let layout = header.layout;
        let rank = layout.shape.rank();
        if axes.len() != rank {
            return outside(site, OTHER_RANK);
        }
        let Some(order) = indices(axes) else {
            return bad();
        };
        let mut seen = [false; MAX_RANK];
        for axis in order.as_slice() {
            match seen.get_mut(*axis) {
                Some(slot) if !*slot && *axis < rank => *slot = true,
                _ => return bad(),
            }
        }
        let pick = |of: &Axes| {
            let lengths: Vec<usize> = order
                .as_slice()
                .iter()
                .map(|axis| of.get(*axis).unwrap_or(0))
                .collect();
            Axes::of(&lengths)
        };
        header.layout.shape = pick(&layout.shape)?;
        header.layout.strides = pick(&layout.strides)?;
        Some(view(header))
    })
}

/// The elements of `t` in the shape of `dims` (rule 39): a view on a contiguous tensor, O(1);
/// otherwise a compacting copy after the budget check, O(size).
fn reshape(site: i64, t: i64, dims: &[i64]) -> i64 {
    word(|| {
        let mut header = dense(t)?;
        let shape = tensor_shape(site, dims)?;
        let Some(n) = shape.checked_count() else {
            return over(site);
        };
        if n != header.layout.count() {
            return outside(site, RESHAPE_COUNT);
        }
        if header.layout.contiguous() {
            header.layout.kind = DenseKind::Tensor;
            header.layout.strides = shape.row_major();
            header.layout.shape = shape;
            Some(view(header))
        } else {
            let store = copy(site, &header)?;
            Some(compact(DenseKind::Tensor, shape, store))
        }
    })
}

/// `d.compact()`: a contiguous copy of the elements, O(size).
fn compact_of(site: i64, d: i64) -> i64 {
    word(|| {
        let header = dense(d)?;
        let store = copy(site, &header)?;
        Some(compact(header.layout.kind, header.layout.shape, store))
    })
}

/// `a.compact()`.
pub(crate) fn array_compact(site: i64, a: i64) -> i64 {
    compact_of(site, a)
}

/// `m.compact()`.
pub(crate) fn matrix_compact(site: i64, m: i64) -> i64 {
    compact_of(site, m)
}

/// `t.compact()`.
pub(crate) fn tensor_compact(site: i64, t: i64) -> i64 {
    compact_of(site, t)
}

/// `a.to_list()`: the elements in row-major order, each `F64` boxed, reserved within the
/// budget first. O(n).
pub(crate) fn array_to_list(site: i64, a: i64) -> i64 {
    word(|| {
        let header = dense(a)?;
        let layout = header.layout;
        let n = layout.count();
        // A boxed `F64` per element besides its word.
        let boxes = match &header.store {
            Store::F64(_) => n.checked_mul(size_of::<f64>()),
            Store::Words(_) => Some(0),
        };
        let Some(boxes) = boxes else {
            return over(site);
        };
        let mut out = buffer::<i64>(site, n, boxes)?;
        for p in (0..n).filter_map(|k| layout.position_at(k)) {
            match header.element(p) {
                Some(element) => out.push(owned_word(element)),
                None => {
                    for word in out {
                        release(word);
                    }
                    return bad();
                }
            }
        }
        Some(alloc(Object::List(out)))
    })
}

// ----- element-wise functions and reductions (rules 34 to 37, 45, 47) -----

/// `a.multiply_elements(by:)` or `a.divide_elements(by:)`, each element by `float::mul` or
/// `float::div`. O(size).
fn elementwise(site: i64, a: i64, b: i64, multiply: bool) -> i64 {
    word(|| {
        let x = dense(a)?;
        let y = dense(b)?;
        same_shape(site, &x, &y)?;
        let (Some(xs), Some(ys)) = (x.f64_iter(), y.f64_iter()) else {
            return bad();
        };
        let values = xs.zip(ys).map(|(p, q)| {
            if multiply {
                float::mul(p, q)
            } else {
                float::div(p, q)
            }
        });
        made_f64(site, x.layout.kind, x.layout.shape, values)
    })
}

/// `a.multiply_elements(by:)`.
pub(crate) fn array_multiply_elements(site: i64, a: i64, by: i64) -> i64 {
    elementwise(site, a, by, true)
}

/// `a.divide_elements(by:)`.
pub(crate) fn array_divide_elements(site: i64, a: i64, by: i64) -> i64 {
    elementwise(site, a, by, false)
}

/// `m.multiply_elements(by:)`.
pub(crate) fn matrix_multiply_elements(site: i64, m: i64, by: i64) -> i64 {
    elementwise(site, m, by, true)
}

/// `m.divide_elements(by:)`.
pub(crate) fn matrix_divide_elements(site: i64, m: i64, by: i64) -> i64 {
    elementwise(site, m, by, false)
}

/// `t.multiply_elements(by:)`.
pub(crate) fn tensor_multiply_elements(site: i64, t: i64, by: i64) -> i64 {
    elementwise(site, t, by, true)
}

/// `t.divide_elements(by:)`.
pub(crate) fn tensor_divide_elements(site: i64, t: i64, by: i64) -> i64 {
    elementwise(site, t, by, false)
}

/// `d.sum()`, by the lanes of rule 45: the blocked kernel on one run of the store, the
/// reference chain otherwise. O(size).
fn sum_of(d: i64) -> i64 {
    let Some(header) = dense(d) else {
        return 0;
    };
    let total = match (header.as_slice(), header.f64_iter()) {
        (Some(items), _) => blocked::sum(items),
        (None, Some(items)) => chains::sum_f64(items),
        (None, None) => {
            fault();
            return 0;
        }
    };
    f64_word(total)
}

/// `a.sum()`.
pub(crate) fn array_sum(_site: i64, a: i64) -> i64 {
    sum_of(a)
}

/// `m.sum()`.
pub(crate) fn matrix_sum(_site: i64, m: i64) -> i64 {
    sum_of(m)
}

/// `t.sum()`.
pub(crate) fn tensor_sum(_site: i64, t: i64) -> i64 {
    sum_of(t)
}

/// `a.dot(with:)`, the inner product of rule 46. O(n).
pub(crate) fn array_dot(site: i64, a: i64, with: i64) -> i64 {
    word(|| {
        let x = dense(a)?;
        let y = dense(with)?;
        same_shape(site, &x, &y)?;
        let (Some(xs), Some(ys)) = (x.f64_iter(), y.f64_iter()) else {
            return bad();
        };
        let total = match (x.as_slice(), y.as_slice()) {
            (Some(p), Some(q)) => blocked::dot(p, q),
            _ => blocked::dot_pairs(0.0, xs.zip(ys)),
        };
        Some(f64_word(total))
    })
}

/// `a.norm2()`, scaled (rule 47). O(n).
pub(crate) fn array_norm2(_site: i64, a: i64) -> i64 {
    let header = dense(a);
    match header.as_ref().and_then(|header| header.f64_iter()) {
        Some(items) => f64_word(blocked::norm2(items)),
        None => {
            fault();
            0
        }
    }
}

/// `d.all_finite()`: the whole scan, then the answer (rule 54). O(size).
fn all_finite_of(d: i64) -> i64 {
    let header = dense(d);
    match header.as_ref().and_then(|header| header.f64_iter()) {
        Some(items) => i64::from(items.fold(true, |all, x| all & x.is_finite())),
        None => {
            fault();
            0
        }
    }
}

/// `a.all_finite()`.
pub(crate) fn array_all_finite(_site: i64, a: i64) -> i64 {
    all_finite_of(a)
}

/// `m.all_finite()`.
pub(crate) fn matrix_all_finite(_site: i64, m: i64) -> i64 {
    all_finite_of(m)
}

/// `t.all_finite()`.
pub(crate) fn tensor_all_finite(_site: i64, t: i64) -> i64 {
    all_finite_of(t)
}

/// `d.sum_exact()`: the exact sum of integer elements, in arbitrary precision. O(size).
fn sum_exact_of(d: i64) -> i64 {
    let total = dense(d).and_then(|header| {
        let layout = header.layout;
        let mut total = BigInt::from(0);
        for p in (0..layout.count()).filter_map(|k| layout.position_at(k)) {
            match header.element(p) {
                Some(Element::Word(word)) => total += int_value(word),
                _ => return bad(),
            }
        }
        Some(total)
    });
    total.map_or(0, |total| int_word(&total))
}

/// `a.sum_exact()`.
pub(crate) fn array_sum_exact(_site: i64, a: i64) -> i64 {
    sum_exact_of(a)
}

/// `m.sum_exact()`.
pub(crate) fn matrix_sum_exact(_site: i64, m: i64) -> i64 {
    sum_exact_of(m)
}

/// `t.sum_exact()`.
pub(crate) fn tensor_sum_exact(_site: i64, t: i64) -> i64 {
    sum_exact_of(t)
}

/// `a.dot_exact(with:)`: the exact inner product of integer elements. O(n).
pub(crate) fn array_dot_exact(site: i64, a: i64, with: i64) -> i64 {
    word(|| {
        let x = dense(a)?;
        let y = dense(with)?;
        same_shape(site, &x, &y)?;
        let (p, q) = (x.layout, y.layout);
        let mut total = BigInt::from(0);
        for k in 0..p.count() {
            let pair = (
                p.position_at(k).and_then(|at| x.element(at)),
                q.position_at(k).and_then(|at| y.element(at)),
            );
            match pair {
                (Some(Element::Word(a)), Some(Element::Word(b))) => {
                    total += int_value(a) * int_value(b);
                }
                _ => return bad(),
            }
        }
        Some(int_word(&total))
    })
}

/// `m.row_sums()` or `m.column_sums()`: each element a sum by rule 45. O(rows × columns).
fn line_sums(site: i64, m: i64, along_rows: bool) -> i64 {
    word(|| {
        let header = dense(m)?;
        let (rows, columns) = header.layout.matrix_shape();
        let (outer, inner) = if along_rows {
            (rows, columns)
        } else {
            (columns, rows)
        };
        if outer > 0 && header.floats().is_none() {
            return bad();
        }
        let line = |i: usize| {
            chains::sum_f64((0..inner).map(|j| {
                let at = if along_rows { [i, j] } else { [j, i] };
                header.f64_at(&at).unwrap_or(f64::NAN)
            }))
        };
        let made = made_f64(site, DenseKind::Array, Axes::one(outer), (0..outer).map(line))?;
        ensure(site, made, &[outer], if along_rows { "row_sums" } else { "column_sums" })
    })
}

/// `m.row_sums()`.
pub(crate) fn matrix_row_sums(site: i64, m: i64) -> i64 {
    line_sums(site, m, true)
}

/// `m.column_sums()`.
pub(crate) fn matrix_column_sums(site: i64, m: i64) -> i64 {
    line_sums(site, m, false)
}

// ----- products (rule 46) -----

/// `m.multiply(by:)`, by the blocked kernel: O(m × k × n).
pub(crate) fn matrix_multiply(site: i64, m: i64, by: i64) -> i64 {
    word(|| {
        let a = dense(m)?;
        let b = dense(by)?;
        let (rows, k) = a.layout.matrix_shape();
        let (k2, columns) = b.layout.matrix_shape();
        if k != k2 {
            return outside(site, MULTIPLY_SHAPES);
        }
        if a.floats().is_none() || b.floats().is_none() {
            return bad();
        }
        let Some(size) = rows.checked_mul(columns) else {
            return over(site);
        };
        let x = slice_of(site, &a)?;
        let y = slice_of(site, &b)?;
        let mut out = buffer::<f64>(site, size, 0)?;
        out.resize(size, 0.0);
        blocked::add_product(&mut out, &x, &y, rows, k, columns);
        let made = compact(DenseKind::Matrix, Axes::two(rows, columns), f64_store(out));
        ensure(site, made, &[rows, columns], "multiply")
    })
}

/// `m.multiply_array(by:)`, by the blocked kernel: O(m × k).
pub(crate) fn matrix_multiply_array(site: i64, m: i64, by: i64) -> i64 {
    word(|| {
        let a = dense(m)?;
        let v = dense(by)?;
        let (rows, k) = a.layout.matrix_shape();
        if v.layout.shape.get(0) != Some(k) {
            return outside(site, MULTIPLY_ARRAY_SHAPES);
        }
        if a.floats().is_none() || v.floats().is_none() {
            return bad();
        }
        let x = slice_of(site, &a)?;
        let y = slice_of(site, &v)?;
        let mut out = buffer::<f64>(site, rows, 0)?;
        out.resize(rows, 0.0);
        blocked::multiply_array(&x, rows, k, &y, &mut out);
        let made = compact(DenseKind::Array, Axes::one(rows), f64_store(out));
        ensure(site, made, &[rows], "multiply_array")
    })
}

/// `c.add_product(of: a, and: b)`, `C += A × B` in place on the owned receiver, compacted
/// first when it is a view of another layout (rule 46, D3). O(m × k × n).
fn add_product(site: i64, c: i64, a: i64, b: i64, give: bool) -> i64 {
    word(|| {
        let (layout, _) = match layout(c) {
            Some(found) => found,
            None => return bad(),
        };
        // The factors' headers hold their stores: a factor that is the receiver makes it
        // shared, so the receiver is copied before it is written (rule 31).
        let x = dense(a)?;
        let y = dense(b)?;
        let (m, n) = layout.matrix_shape();
        let (m2, k) = x.layout.matrix_shape();
        let (k2, n2) = y.layout.matrix_shape();
        if m != m2 || k != k2 || n != n2 {
            return outside(site, ADD_PRODUCT_SHAPES);
        }
        if x.floats().is_none() || y.floats().is_none() {
            return bad();
        }
        if !give {
            retain(c);
        }
        let mut owned = owned(site, c)?;
        let count = layout.count();
        if count == 0 {
            return Some(updated(owned));
        }
        let (current, _) = self::layout(owned)?;
        if !current.contiguous() {
            // A view held once, of another layout: compacted first, so the kernel updates one
            // run of storage in place.
            let header = dense(owned)?;
            let Some(store) = copy(site, &header) else {
                release(owned);
                return None;
            };
            let made = compact(header.layout.kind, header.layout.shape, store);
            drop(header);
            release(owned);
            owned = made;
        }
        let (p, q) = (slice_of(site, &x)?, slice_of(site, &y)?);
        let (current, _) = self::layout(owned)?;
        let start = current.offset;
        let ran = with_mut(owned, |object| match object {
            Object::Dense(dense) => match &mut dense.store {
                Store::F64(items) => match Arc::get_mut(items)
                    .and_then(|items| items.get_mut(start..start.checked_add(count)?))
                {
                    Some(out) => {
                        blocked::add_product(out, &p, &q, m, k, n);
                        true
                    }
                    None => false,
                },
                Store::Words(_) => false,
            },
            _ => false,
        });
        if ran != Some(true) {
            fault();
        }
        Some(updated(owned))
    })
}

/// `m.add_product(of:, and:)`, by copy.
pub(crate) fn matrix_add_product(site: i64, m: i64, of: i64, and: i64) -> i64 {
    add_product(site, m, of, and, false)
}

/// `m.add_product(of:, and:)` at the matrix's last use: in place when held once.
pub(crate) fn matrix_add_product_owned(site: i64, m: i64, of: i64, and: i64) -> i64 {
    add_product(site, m, of, and, true)
}

/// Applies `f` to each `F64` element of the owned value `word`, in row-major order, with the
/// element's row-major position: the in-place loop of `add_scaled` and `scale`.
fn each_in_place(word: i64, mut f: impl FnMut(usize, f64) -> f64) -> bool {
    let Some((found, words)) = layout(word) else {
        return false;
    };
    if words.is_some() {
        // A words store holds no `F64`: only a value of no elements has one here.
        return found.count() == 0;
    }
    with_mut(word, |object| match object {
        Object::Dense(dense) => match &mut dense.store {
            Store::F64(items) => match Arc::get_mut(items) {
                Some(items) => {
                    for k in 0..found.count() {
                        if let Some(slot) = found.position_at(k).and_then(|p| items.get_mut(p)) {
                            *slot = float::canonical(f(k, *slot));
                        }
                    }
                    true
                }
                None => false,
            },
            Store::Words(_) => false,
        },
        _ => false,
    })
    .unwrap_or(false)
}

/// `y.add_scaled(x:, by: k)`, `y_i = fma(k, x_i, y_i)` in place on the owned receiver (rule
/// 46, D3). O(n).
fn add_scaled(site: i64, y: i64, x: i64, by: i64, give: bool) -> i64 {
    word(|| {
        let k = scalar(by)?;
        let target = dense(y)?;
        // The source's header holds its store: a source that is the receiver makes it
        // shared, so the receiver is copied before it is written (rule 31).
        let source = dense(x)?;
        same_shape(site, &target, &source)?;
        drop(target);
        let Some(items) = source.floats() else {
            return bad();
        };
        let from = source.layout;
        if !give {
            retain(y);
        }
        let owned = owned(site, y)?;
        let ran = each_in_place(owned, |i, yi| {
            let xi = from
                .position_at(i)
                .and_then(|p| items.get(p).copied())
                .unwrap_or(f64::NAN);
            fma::baseline(k, xi, yi)
        });
        if !ran {
            fault();
        }
        Some(updated(owned))
    })
}

/// `a.add_scaled(x:, by:)`, by copy.
pub(crate) fn array_add_scaled(site: i64, a: i64, x: i64, by: i64) -> i64 {
    add_scaled(site, a, x, by, false)
}

/// `a.add_scaled(x:, by:)` at the array's last use: in place when held once.
pub(crate) fn array_add_scaled_owned(site: i64, a: i64, x: i64, by: i64) -> i64 {
    add_scaled(site, a, x, by, true)
}

/// `d.scale(by: k)`, each element `float::mul(x, k)` in place on the owned receiver (D3).
/// O(size).
fn scale(site: i64, d: i64, by: i64, give: bool) -> i64 {
    word(|| {
        let k = scalar(by)?;
        if layout(d).is_none() {
            return bad();
        }
        if !give {
            retain(d);
        }
        let owned = owned(site, d)?;
        if !each_in_place(owned, |_, x| float::mul(x, k)) {
            fault();
        }
        Some(updated(owned))
    })
}

/// `a.scale(by:)`, by copy.
pub(crate) fn array_scale(site: i64, a: i64, by: i64) -> i64 {
    scale(site, a, by, false)
}

/// `a.scale(by:)` at the array's last use.
pub(crate) fn array_scale_owned(site: i64, a: i64, by: i64) -> i64 {
    scale(site, a, by, true)
}

/// `m.scale(by:)`, by copy.
pub(crate) fn matrix_scale(site: i64, m: i64, by: i64) -> i64 {
    scale(site, m, by, false)
}

/// `m.scale(by:)` at the matrix's last use.
pub(crate) fn matrix_scale_owned(site: i64, m: i64, by: i64) -> i64 {
    scale(site, m, by, true)
}

// ----- tensors: reductions and contractions (rules 39, 45, 46) -----

/// `t.sum(axis:)`: each element of the result a sum along the axis by rule 45. O(size).
pub(crate) fn tensor_sum_axis(site: i64, t: i64, axis: i64) -> i64 {
    word(|| {
        let header = dense(t)?;
        let rank = header.layout.shape.rank();
        let Some(axis) = index(axis).filter(|axis| rank >= 2 && *axis < rank) else {
            return outside(site, SUM_AXIS_OUTSIDE);
        };
        if header.floats().is_none() {
            return bad();
        }
        let length = header.layout.shape.get(axis).unwrap_or(0);
        let shape = header.layout.shape.without(axis);
        let n = shape.count();
        let element = |p: usize| {
            let mut at = [0_usize; MAX_RANK];
            let mut rest = p;
            for (k, length) in shape.as_slice().iter().enumerate().rev() {
                let slot = if k < axis { k } else { k + 1 };
                if let Some(index) = at.get_mut(slot) {
                    *index = rest % length.max(&1);
                }
                rest /= length.max(&1);
            }
            chains::sum_f64((0..length).map(|i| {
                let mut at = at;
                if let Some(index) = at.get_mut(axis) {
                    *index = i;
                }
                header
                    .f64_at(at.get(..rank).unwrap_or(&[]))
                    .unwrap_or(f64::NAN)
            }))
        };
        let made = made_f64(site, DenseKind::Tensor, shape, (0..n).map(element))?;
        ensure(site, made, shape.as_slice(), "sum")
    })
}

/// `t.contract(with: u, axis:, other:)`: each element of the result an inner product over the
/// pair of axes by rule 46, so it has the bits of the matching element of `multiply` (rule
/// 39). O(size of the result × the contracted length).
pub(crate) fn tensor_contract(site: i64, t: i64, with: i64, axis: i64, other: i64) -> i64 {
    word(|| {
        let x = dense(t)?;
        let y = dense(with)?;
        let (rt, ru) = (x.layout.shape.rank(), y.layout.shape.rank());
        let (axis, other_axis) = match (index(axis), index(other)) {
            (Some(k), Some(l))
                if k < rt
                    && l < ru
                    && x.layout.shape.get(k) == y.layout.shape.get(l)
                    && (3..=10).contains(&(rt + ru)) =>
            {
                (k, l)
            }
            _ => return outside(site, CONTRACT_AXES),
        };
        if x.floats().is_none() || y.floats().is_none() {
            return bad();
        }
        let terms = x.layout.shape.get(axis).unwrap_or(0);
        let mut lengths: Vec<usize> = x.layout.shape.without(axis).as_slice().to_vec();
        lengths.extend_from_slice(y.layout.shape.without(other_axis).as_slice());
        let Some(shape) = Axes::of(&lengths) else {
            return bad();
        };
        let Some(n) = shape.checked_count() else {
            return over(site);
        };
        let element = |p: usize| {
            // The result's indices: the remaining axes of t, then those of u.
            let mut out = [0_usize; 2 * MAX_RANK];
            let mut rest = p;
            for (k, length) in shape.as_slice().iter().enumerate().rev() {
                if let Some(index) = out.get_mut(k) {
                    *index = rest % length.max(&1);
                }
                rest /= length.max(&1);
            }
            let (mut ti, mut ui) = ([0_usize; MAX_RANK], [0_usize; MAX_RANK]);
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
                let (mut ti, mut ui) = (ti, ui);
                if let Some(index) = ti.get_mut(axis) {
                    *index = i;
                }
                if let Some(index) = ui.get_mut(other_axis) {
                    *index = i;
                }
                (
                    x.f64_at(ti.get(..rt).unwrap_or(&[])).unwrap_or(f64::NAN),
                    y.f64_at(ui.get(..ru).unwrap_or(&[])).unwrap_or(f64::NAN),
                )
            });
            blocked::dot_pairs(0.0, pairs)
        };
        let made = made_f64(site, DenseKind::Tensor, shape, (0..n).map(element))?;
        ensure(site, made, shape.as_slice(), "contract")
    })
}

// ----- factorizations and solves (rules 48 to 50), each run to its end (rule 54) -----

/// The LU factorization of the square matrix `header` (rule 48): the order, the factors in the
/// first `n * n` places of a vector with room for `extra` more, and the pivots, or `None` in
/// the first place when it is singular. `None` after a refusal or an abort.
#[allow(clippy::type_complexity)]
fn factorize(
    site: i64,
    header: &Dense,
    extra: usize,
) -> Option<Option<(usize, Vec<f64>, Vec<usize>)>> {
    let n = square(site, header, "lu")?;
    let mut factors = f64_copy(site, header, extra)?;
    let mut pivots = buffer::<usize>(site, n, 0)?;
    pivots.resize(n, 0);
    let nonsingular = blocked::lu(&mut factors, n, &mut pivots);
    Some(nonsingular.then_some((n, factors, pivots)))
}

/// `rhs`, of `n` rows, solved with the factors and pivots of an LU factorization (rule 48): a
/// value of `kind` and `rhs`'s shape, checked before it is returned.
fn lu_solved(
    site: i64,
    factors: &[f64],
    pivots: &[usize],
    n: usize,
    rhs: &Dense,
    kind: DenseKind,
) -> Option<i64> {
    let shape = rhs.layout.shape;
    let r = if kind == DenseKind::Array {
        1
    } else {
        shape.get(1).unwrap_or(0)
    };
    if shape.get(0) != Some(n) {
        return outside(site, SOLVE_ROWS);
    }
    let mut x = f64_copy(site, rhs, 0)?;
    blocked::lu_solve(factors, pivots, n, &mut x, r);
    let made = compact(kind, shape, f64_store(x));
    ensure(site, made, shape.as_slice(), "solve")
}

/// `m.lu()`: `Some` of the factorization, an `Lu` of n + 1 rows of n, or `None` when `m` is
/// singular. O(n³).
pub(crate) fn matrix_lu(site: i64, m: i64) -> i64 {
    word(|| {
        let header = dense(m)?;
        // Room for the pivots is reserved with the factors: they are the last row.
        let rows = header.layout.matrix_shape().0;
        let Some((n, mut factors, pivots)) = factorize(site, &header, rows)? else {
            return Some(NONE);
        };
        factors.extend(pivots.iter().map(|p| *p as f64));
        let made = compact(DenseKind::Lu, Axes::two(n + 1, n), f64_store(factors));
        let made = ensure(site, made, &[n + 1, n], "lu")?;
        let option = some(made);
        release(made);
        Some(option)
    })
}

/// `m.solve(rhs:)` or `m.solve_array(rhs:)`: `Some` of the solution of `m x = rhs`, or `None`
/// when `m` is singular. O(n³ + n² r).
fn matrix_solve_as(site: i64, m: i64, rhs: i64, kind: DenseKind) -> i64 {
    word(|| {
        let header = dense(m)?;
        let rhs = dense(rhs)?;
        let Some((n, factors, pivots)) = factorize(site, &header, 0)? else {
            return Some(NONE);
        };
        let solved = lu_solved(site, &factors, &pivots, n, &rhs, kind)?;
        let option = some(solved);
        release(solved);
        Some(option)
    })
}

/// `m.solve(rhs:)`.
pub(crate) fn matrix_solve(site: i64, m: i64, rhs: i64) -> i64 {
    matrix_solve_as(site, m, rhs, DenseKind::Matrix)
}

/// `m.solve_array(rhs:)`.
pub(crate) fn matrix_solve_array(site: i64, m: i64, rhs: i64) -> i64 {
    matrix_solve_as(site, m, rhs, DenseKind::Array)
}

/// `f.solve(rhs:)` of an `Lu`: the solution with its factors and pivots. O(n² r).
pub(crate) fn lu_solve(site: i64, f: i64, rhs: i64) -> i64 {
    word(|| {
        let header = dense(f)?;
        let rhs = dense(rhs)?;
        let n = header.layout.shape.get(1).unwrap_or(0);
        let Store::F64(items) = &header.store else {
            return bad();
        };
        let start = header.layout.offset;
        let bounds = n.checked_mul(n).and_then(|size| {
            let middle = start.checked_add(size)?;
            Some((middle, middle.checked_add(n)?))
        });
        let (Some(factors), Some(stored)) = (
            bounds.and_then(|(middle, _)| items.get(start..middle)),
            bounds.and_then(|(middle, end)| items.get(middle..end)),
        ) else {
            return bad();
        };
        let mut pivots = buffer::<usize>(site, n, 0)?;
        pivots.extend(stored.iter().map(|p| *p as usize));
        lu_solved(site, factors, &pivots, n, &rhs, DenseKind::Matrix)
    })
}

/// `f.factors()`: the n × n matrix of the factors, a view.
pub(crate) fn lu_factors(_site: i64, f: i64) -> i64 {
    match dense(f) {
        Some(mut header) => {
            let n = header.layout.shape.get(1).unwrap_or(0);
            header.layout.kind = DenseKind::Matrix;
            header.layout.shape = Axes::two(n, n);
            view(header)
        }
        None => 0,
    }
}

/// `f.pivots()`: the row exchanged at each step, a list of n integers. O(n).
pub(crate) fn lu_pivots(site: i64, f: i64) -> i64 {
    word(|| {
        let header = dense(f)?;
        let n = header.layout.shape.get(1).unwrap_or(0);
        let mut out = buffer::<i64>(site, n, 0)?;
        for k in 0..n {
            let Some(p) = header.f64_at(&[n, k]) else {
                return bad();
            };
            out.push(int_word(&BigInt::from(p as usize)));
        }
        Some(alloc(Object::List(out)))
    })
}

/// `m.cholesky()`: `Some` of the lower factor, or `None` when `m` is not positive definite.
/// O(n³).
pub(crate) fn matrix_cholesky(site: i64, m: i64) -> i64 {
    word(|| {
        let header = dense(m)?;
        let n = square(site, &header, "cholesky")?;
        let mut factor = f64_copy(site, &header, 0)?;
        if !blocked::cholesky(&mut factor, n) {
            return Some(NONE);
        }
        let made = compact(DenseKind::Matrix, Axes::two(n, n), f64_store(factor));
        let made = ensure(site, made, &[n, n], "cholesky")?;
        let option = some(made);
        release(made);
        Some(option)
    })
}

/// `m.solve_lower(rhs:, unit_diagonal:)` or `m.solve_upper(...)` (rule 50). O(n² r).
fn solve_triangular(site: i64, m: i64, rhs: i64, unit: i64, lower: bool) -> i64 {
    word(|| {
        let header = dense(m)?;
        let rhs = dense(rhs)?;
        let unit = match unit {
            0 => false,
            1 => true,
            _ => return bad(),
        };
        let name = if lower { "solve_lower" } else { "solve_upper" };
        let n = square(site, &header, name)?;
        let (rows, r) = rhs.layout.matrix_shape();
        if rows != n {
            return outside(site, TRIANGULAR_ROWS);
        }
        let t = f64_copy(site, &header, 0)?;
        let mut x = f64_copy(site, &rhs, 0)?;
        let triangle = if lower {
            chains::Triangle::Lower
        } else {
            chains::Triangle::Upper
        };
        blocked::solve_triangular(&t, n, &mut x, r, triangle, unit);
        let made = compact(DenseKind::Matrix, Axes::two(n, r), f64_store(x));
        ensure(site, made, &[n, r], name)
    })
}

/// `m.solve_lower(rhs:, unit_diagonal:)`.
pub(crate) fn matrix_solve_lower(site: i64, m: i64, rhs: i64, unit: i64) -> i64 {
    solve_triangular(site, m, rhs, unit, true)
}

/// `m.solve_upper(rhs:, unit_diagonal:)`.
pub(crate) fn matrix_solve_upper(site: i64, m: i64, rhs: i64, unit: i64) -> i64 {
    solve_triangular(site, m, rhs, unit, false)
}

// ----- the loops of the higher-order methods (D6) and the shape functions -----

/// The number of elements of `d`, as a machine integer: the bound of the loop the codegen
/// emits for `map`, `zip_map`, `update` and `fold`. O(rank).
pub(crate) fn dense_count(d: i64) -> i64 {
    layout(d)
        .and_then(|(layout, _)| i64::try_from(layout.count()).ok())
        .unwrap_or(0)
}

/// Element `k` of `d` in row-major order, `k` a machine integer below [`dense_count`]: a word
/// the caller owns, a new boxed `F64` or the element retained. O(rank).
pub(crate) fn dense_item(d: i64, k: i64) -> i64 {
    let Ok(k) = usize::try_from(k) else {
        fault();
        return 0;
    };
    let Some(header) = header(d) else {
        fault();
        return 0;
    };
    match header
        .layout
        .position_at(k)
        .and_then(|p| header.element(p))
    {
        Some(element) => owned_word(element),
        None => {
            fault();
            0
        }
    }
}

/// A builder of the result of `map` or `zip_map` over `like`: a compact value of its kind and
/// shape whose elements [`dense_push`] adds in row-major order, its store chosen and reserved
/// at the first (the interpreter's `Builder`). O(1).
pub(crate) fn dense_builder(_site: i64, like: i64) -> i64 {
    match layout(like) {
        Some((layout, _)) => compact(
            layout.kind,
            layout.shape,
            words_store(Vec::new()),
        ),
        None => {
            fault();
            0
        }
    }
}

/// [`dense_builder`] for `zip_map` of `like` and `other`, whose shapes its precondition keeps
/// equal (rule 34).
pub(crate) fn dense_zip_builder(site: i64, like: i64, other: i64) -> i64 {
    let (Some((a, _)), Some((b, _))) = (layout(like), layout(other)) else {
        fault();
        return 0;
    };
    if a.shape != b.shape {
        return outside::<i64>(site, DIFFERENT_SHAPES).unwrap_or(0);
    }
    dense_builder(site, like)
}

/// Adds `item`, borrowed, as the next element of the builder `b`, which the caller holds once:
/// at the first, the store for all of them is chosen by the item (raw for an `F64`) and
/// reserved within the budget, so a refusal is U6003 at `site` before any is stored. O(1).
pub(crate) fn dense_push(site: i64, b: i64, item: i64) -> i64 {
    let Some((layout, words)) = layout(b) else {
        fault();
        return 0;
    };
    let n = layout.count();
    let x = boxed(item);
    let stored = match words {
        Some(words) => {
            let first = with(words, |object| match object {
                Object::Words(items) => items.is_empty(),
                _ => false,
            })
            .unwrap_or(false);
            if first && n > 0 {
                if let Some(x) = x {
                    let Some(mut out) = buffer::<f64>(site, n, 0) else {
                        return 0;
                    };
                    out.push(float::canonical(x));
                    let replaced = with_mut(b, |object| match object {
                        Object::Dense(dense) => {
                            dense.store = f64_store(out);
                            true
                        }
                        _ => false,
                    });
                    if replaced == Some(true) {
                        release(words);
                    }
                    replaced == Some(true)
                } else {
                    let Some(mut out) = buffer::<i64>(site, n, 0) else {
                        return 0;
                    };
                    retain(item);
                    out.push(item);
                    with_mut(words, |object| match object {
                        Object::Words(items) => {
                            *items = out;
                            true
                        }
                        _ => false,
                    })
                    .unwrap_or(false)
                }
            } else {
                retain(item);
                let pushed = with_mut(words, |object| match object {
                    Object::Words(items) if items.len() < n => {
                        items.push(item);
                        true
                    }
                    _ => false,
                })
                .unwrap_or(false);
                if !pushed {
                    release(item);
                }
                pushed
            }
        }
        None => match x {
            Some(x) => with_mut(b, |object| match object {
                Object::Dense(dense) => match &mut dense.store {
                    Store::Words(_) => false,
                    Store::F64(items) => match Arc::get_mut(items) {
                        Some(items) if items.len() < n => {
                            items.push(float::canonical(x));
                            true
                        }
                        _ => false,
                    },
                },
                _ => false,
            })
            .unwrap_or(false),
            None => false,
        },
    };
    if !stored {
        fault();
    }
    0
}

/// The dense value `d`, whose reference the caller gives up, held once with its store, for
/// `update` to write in place: itself, or a compact copy after the budget check (D3, rule 31).
pub(crate) fn dense_unique(site: i64, d: i64) -> i64 {
    if layout(d).is_none() {
        fault();
        return 0;
    }
    owned(site, d).unwrap_or(0)
}

/// Writes `item`, borrowed, as element `k` in row-major order of `d`, which [`dense_unique`]
/// made held once: the step of `update`. O(rank).
pub(crate) fn dense_write(d: i64, k: i64, item: i64) -> i64 {
    let p = usize::try_from(k)
        .ok()
        .and_then(|k| layout(d).and_then(|(layout, _)| layout.position_at(k)));
    if !p.is_some_and(|p| write(d, p, item)) {
        fault();
    }
    0
}

/// A shape function of `d` (`spec/29 §17`, ADR 0120 rule 33): the length of axis `axis` for
/// `axis` from 0, `len`, `rows`, `columns` and `size` among them; the rank for -1; the number
/// of elements for -2. An `Int` word.
pub(crate) fn dense_shape(d: i64, axis: i64) -> i64 {
    let Some((layout, _)) = layout(d) else {
        fault();
        return 1;
    };
    let value = match axis {
        -1 => layout.shape.rank(),
        -2 => layout.count(),
        axis => usize::try_from(axis)
            .ok()
            .and_then(|axis| layout.shape.get(axis))
            .unwrap_or(0),
    };
    int_word(&BigInt::from(value))
}

