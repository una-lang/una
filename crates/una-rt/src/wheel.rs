//! The timing wheel of a run's timers (ADR 0117 rule 5 and Costs, slice S2,
//! `docs/plan/0117-s2-design.md`; ADR 0116 amendment of 2026-10-25, P13, #1151).
//!
//! A `within` arms one timer when it starts and cancels it when its expression ends. Deadlines
//! are in **nanoseconds** of the run's monotonic clock, and every comparison of a deadline with
//! the wheel's time is exact, in nanoseconds: a timer never fires before its own deadline, nor
//! waits for the end of the millisecond it falls in (P13, "firing at the exact moment").
//!
//! - **Levels.** The wheel is hierarchical over the millisecond count of a deadline, its
//!   nanoseconds divided by 1,000,000: 11 levels of 64 slots each, where level `L` holds the
//!   timers whose millisecond first differs from the wheel's in the 6-bit group `L`. Eleven
//!   groups of 6 bits cover every `u64`, so no deadline is too far for the wheel and nothing is
//!   clamped or wrapped. Each level keeps a bitmap of its occupied slots, so the next slot to
//!   reach is found in O(levels).
//! - **The finest level keeps exact order.** A slot of level 0 holds the timers of one
//!   millisecond in a small binary min-heap ordered by exact deadline, then by arming order. The
//!   wheel's current millisecond is such a slot too, so a timer armed for later in the current
//!   millisecond waits for its own nanosecond.
//! - **The higher levels and the due list** are intrusive doubly linked lists over one slab of
//!   entries, as before, so removal from them is eager and O(1).
//! - **Cancelling is O(1).** In a higher level or the due list the entry is unlinked at once. In
//!   level 0 it is deleted lazily: cancelling frees the entry and bumps its generation, and the
//!   heap item, which carries the generation it was pushed with, is dropped unfired when it
//!   reaches the top of its heap. Lazy deletion is confined to level 0, whose slots span the
//!   next 64 ms only, so dead items are bounded by the arming rate times 64 ms and are popped as
//!   time passes them. Long deadlines armed and cancelled at a high rate, such as `within 1 hour`
//!   10,000 times a second, sit in the higher levels and leave nothing behind.
//! - **Costs,** for `s` the timers of one level-0 slot, few on average:
//!   - arming: O(1) in a higher level or the due list, O(log s) into a level-0 heap, so O(1) on
//!     average;
//!   - cancelling: O(1);
//!   - firing: O(log s) for each item taken from a heap, dead ones included, so O(log s)
//!     amortized per armed timer; a timer moves down the levels at most 10 times in its life,
//!     each move O(1), or O(log s) into level 0;
//!   - finding the next expiration: O(levels);
//!   - space: one slab entry per armed timer, and one heap item per timer that entered level 0
//!     until it fires or its dead item is popped.
//!
//! **Determinism.** Timers that fire together come out ordered by deadline, then by the order
//! they were armed, whatever path they took through the levels, so a run under virtual time
//! (ADR 0116 rule 19), whose readings are whole milliseconds, sees the same order in every
//! engine.
//!
//! **Time never goes back.** [`Wheel::advance`] to a time before the wheel's own is an advance
//! to the wheel's time. A timer armed with a deadline at or before the wheel's time is due at
//! once, and fires at the next advance.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// The nanoseconds of one slot of level 0: one millisecond.
const SLOT_NANOS: u64 = 1_000_000;

/// Bits of a millisecond count that one level resolves.
const SLOT_BITS: u32 = 6;

/// Slots in each level.
const SLOTS: usize = 1 << SLOT_BITS;

/// Levels: 11 groups of 6 bits cover the 64 bits of a millisecond count.
const LEVELS: usize = 11;

/// The list of timers already due, after the `LEVELS * SLOTS` lists of the slots. The lists of
/// level 0, `0..SLOTS`, are heaps instead, and their heads stay unused.
const DUE: usize = LEVELS * SLOTS;

/// The end of a list, and the list of a free entry.
const NIL: u32 = u32::MAX;

/// An item of a level-0 heap: the exact deadline, the arming order, the entry and the entry's
/// generation when it was pushed, smallest deadline first. An item whose generation no longer
/// matches its entry's is dead: its timer was cancelled, or already fired.
type Item = Reverse<(u64, u64, u32, u64)>;

/// A timer armed in a [`Wheel`]: its entry and that entry's generation, so that cancelling a
/// timer that already fired, or was cancelled, does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimerId {
    index: u32,
    /// 64 bits, so an entry is reused far fewer times than it would take to wrap: a stale id
    /// never matches a live timer, and a dead heap item never matches a live entry.
    generation: u64,
}

/// The wheel cannot hold another timer: it already holds `u32::MAX - 1`. Timers are bounded by
/// the run's tasks, which `--max-memory` bounds long before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WheelFull;

/// One entry of the slab: a timer, or a free entry when `value` is `None`.
struct Entry<T> {
    /// The exact deadline, in nanoseconds.
    deadline: u64,
    /// The order the timer was armed in, to order timers that fire together.
    seq: u64,
    generation: u64,
    /// Where the entry is: a slot of level 0 (`0..SLOTS`, in that slot's heap), a list of a
    /// higher slot (`level * SLOTS + slot`), [`DUE`], or `NIL` when free or taken out.
    list: u32,
    prev: u32,
    next: u32,
    value: Option<T>,
}

/// A hierarchical timing wheel over a clock in nanoseconds, with millisecond slots whose
/// finest level keeps exact order. See the module documentation.
pub struct Wheel<T> {
    /// The wheel's time, in nanoseconds.
    now: u64,
    heads: Vec<u32>,
    /// The heaps of level 0's slots.
    heaps: Vec<BinaryHeap<Item>>,
    occupied: [u64; LEVELS],
    entries: Vec<Entry<T>>,
    free: Vec<u32>,
    len: usize,
    seq: u64,
    /// Lists taken out while advancing, kept for their capacity so that advancing allocates
    /// nothing once warm.
    batch: Vec<u32>,
    due: Vec<u32>,
    /// Entry moves between lists and heaps, for the scaling tests.
    #[cfg(test)]
    moves: u64,
}

/// The millisecond of the nanosecond time `nanos`: the slot it falls in.
fn tick(nanos: u64) -> u64 {
    nanos / SLOT_NANOS
}

impl<T> Wheel<T> {
    /// An empty wheel whose time is `now`, in nanoseconds.
    pub fn new(now: u64) -> Wheel<T> {
        Wheel {
            now,
            heads: vec![NIL; DUE + 1],
            heaps: (0..SLOTS).map(|_| BinaryHeap::new()).collect(),
            occupied: [0; LEVELS],
            entries: Vec::new(),
            free: Vec::new(),
            len: 0,
            seq: 0,
            batch: Vec::new(),
            due: Vec::new(),
            #[cfg(test)]
            moves: 0,
        }
    }

    /// The wheel's time: the last time it was advanced to, in nanoseconds.
    pub fn now(&self) -> u64 {
        self.now
    }

    /// The timers armed and not yet fired or cancelled.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no timer is armed.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Arms a timer that fires with `value` once the wheel reaches `deadline`, in nanoseconds.
    /// O(1) into a higher level or the due list, O(log s) into a slot of level 0 that holds `s`
    /// timers: O(1) on average.
    pub fn arm(&mut self, deadline: u64, value: T) -> Result<TimerId, WheelFull> {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                let index = u32::try_from(self.entries.len()).map_err(|_| WheelFull)?;
                if index == NIL {
                    return Err(WheelFull);
                }
                self.entries.push(Entry {
                    deadline: 0,
                    seq: 0,
                    generation: 0,
                    list: NIL,
                    prev: NIL,
                    next: NIL,
                    value: None,
                });
                index
            }
        };
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        let generation = match self.entries.get_mut(index as usize) {
            Some(entry) => {
                entry.deadline = deadline;
                entry.seq = seq;
                entry.value = Some(value);
                entry.generation
            }
            // INVARIANT: `index` came from the free list or was just pushed.
            None => return Err(WheelFull),
        };
        self.len += 1;
        self.place(index);
        Ok(TimerId { index, generation })
    }

    /// Cancels the timer `id` and gives its value back, or `None` when it already fired or was
    /// cancelled. O(1): unlinked at once from a higher level or the due list, and left as a dead
    /// item in a heap of level 0, dropped when it reaches the top.
    pub fn cancel(&mut self, id: TimerId) -> Option<T> {
        let entry = self.entries.get(id.index as usize)?;
        if entry.generation != id.generation || entry.value.is_none() {
            return None;
        }
        if entry.list as usize >= SLOTS {
            self.unlink(id.index);
        }
        self.release(id.index)
    }

    /// The earliest time at which a timer may fire: no timer fires before it, and a timer due
    /// now gives the wheel's time. `None` when no timer is armed. O(levels).
    ///
    /// When the earliest slot is of level 0, it is the exact deadline at the top of its heap:
    /// the earliest timer's own nanosecond, or that of a dead item, which only wakes a waiting
    /// caller once more. Otherwise it is the start of a slot that only cascades, and a caller
    /// that waits for the next timer (the timer thread, or virtual time that jumps) advances to
    /// it and asks again, at most 10 times.
    pub fn next_expiration(&self) -> Option<u64> {
        if self.head(DUE) != NIL {
            return Some(self.now);
        }
        let (list, start) = self.next_slot()?;
        if list < SLOTS
            && let Some(&Reverse((deadline, ..))) = self.heaps.get(list).and_then(BinaryHeap::peek)
        {
            // A higher slot starts on a millisecond, and a tie with this slot's start puts the
            // higher one first, so none starts between this slot's start and its top item.
            return Some(deadline.max(self.now));
        }
        Some(start)
    }

    /// Advances the wheel to `to` and appends to `fired` every timer whose deadline is at or
    /// before it, ordered by deadline and then by the order they were armed. A `to` before the
    /// wheel's time advances to the wheel's time. O(log s) amortized per timer, plus O(levels)
    /// per slot reached.
    pub fn advance(&mut self, to: u64, fired: &mut Vec<(TimerId, T)>) {
        self.advance_in(to, fired, true);
    }

    /// [`Wheel::advance`] without ordering the timers that fire together, with no allocation
    /// once `fired` and the wheel's own lists have grown, for a caller that holds a lock while
    /// it advances and has no use for the order (the real clock's timer thread).
    pub fn advance_unordered(&mut self, to: u64, fired: &mut Vec<(TimerId, T)>) {
        self.advance_in(to, fired, false);
    }

    fn advance_in(&mut self, to: u64, fired: &mut Vec<(TimerId, T)>, ordered: bool) {
        let to = to.max(self.now);
        let mut batch = std::mem::take(&mut self.batch);
        let mut due = std::mem::take(&mut self.due);
        self.take_list(DUE, &mut batch);
        self.fire(&mut batch, fired, ordered);
        while let Some((list, start)) = self.next_slot() {
            if start > to {
                break;
            }
            self.now = start.max(self.now);
            if list < SLOTS {
                // A slot of level 0: every item whose exact deadline is at or before `to`
                // leaves its heap, the live ones to fire. When one is left, its deadline is
                // after `to`, in the millisecond `to` falls in, and every other occupied slot
                // starts after that millisecond: the advance ends here.
                let waiting = self.drain_heap(list, to, &mut due);
                self.fire(&mut due, fired, ordered);
                if waiting {
                    break;
                }
                continue;
            }
            self.take_list(list, &mut batch);
            for index in batch.drain(..) {
                let deadline = match self.entries.get(index as usize) {
                    Some(entry) => entry.deadline,
                    None => continue,
                };
                if deadline <= self.now {
                    due.push(index);
                } else {
                    self.place(index);
                }
            }
            self.fire(&mut due, fired, ordered);
        }
        self.now = to;
        self.batch = batch;
        self.due = due;
    }

    /// Takes out of the heap of level-0 slot `list` every item whose deadline is at or before
    /// `to`, appending the live ones' entries to `due` and dropping the dead ones, and clears
    /// the slot's bit once its heap is empty. Whether an item is left. O(log s) per item.
    fn drain_heap(&mut self, list: usize, to: u64, due: &mut Vec<u32>) -> bool {
        let Some(heap) = self.heaps.get_mut(list) else {
            return false;
        };
        while let Some(&Reverse((deadline, _, index, generation))) = heap.peek() {
            if deadline > to {
                return true;
            }
            heap.pop();
            if let Some(entry) = self.entries.get_mut(index as usize)
                && entry.generation == generation
                && entry.value.is_some()
            {
                entry.list = NIL;
                due.push(index);
            }
        }
        if let Some(bits) = self.occupied.first_mut() {
            *bits &= !(1 << list);
        }
        false
    }

    /// Removes the entries of `batch` from the slab and appends their timers to `fired`, when
    /// `ordered` by deadline, then arming order.
    fn fire(&mut self, batch: &mut Vec<u32>, fired: &mut Vec<(TimerId, T)>, ordered: bool) {
        let entries = &self.entries;
        if ordered {
            batch.sort_by_key(|&index| {
                entries
                    .get(index as usize)
                    .map_or((u64::MAX, u64::MAX), |entry| (entry.deadline, entry.seq))
            });
        }
        for index in batch.drain(..) {
            let generation = match self.entries.get(index as usize) {
                Some(entry) => entry.generation,
                None => continue,
            };
            if let Some(value) = self.release(index) {
                fired.push((TimerId { index, generation }, value));
            }
        }
    }

    /// Frees the entry `index`, already off every list (a dead item of it may stay in a heap
    /// of level 0), and gives its value back.
    fn release(&mut self, index: u32) -> Option<T> {
        let entry = self.entries.get_mut(index as usize)?;
        let value = entry.value.take()?;
        entry.generation = entry.generation.wrapping_add(1);
        entry.list = NIL;
        self.free.push(index);
        self.len -= 1;
        Some(value)
    }

    /// Where a deadline belongs at the wheel's time: [`DUE`] when it is not after it; else the
    /// slot of level 0 of its millisecond when that millisecond is in the wheel's current
    /// window of 64 (the current millisecond included); else the slot of the highest 6-bit
    /// group in which its millisecond differs from the wheel's.
    fn list_of(&self, deadline: u64) -> usize {
        if deadline <= self.now {
            return DUE;
        }
        let at = tick(deadline);
        let differs = at ^ tick(self.now);
        let level = if differs == 0 {
            0
        } else {
            ((63 - differs.leading_zeros()) / SLOT_BITS) as usize
        };
        let slot = ((at >> (level as u32 * SLOT_BITS)) as usize) & (SLOTS - 1);
        level * SLOTS + slot
    }

    /// Puts the entry `index`, off every list, where its deadline belongs: O(log s) into a heap
    /// of level 0, O(1) onto a list.
    fn place(&mut self, index: u32) {
        let (deadline, seq, generation) = match self.entries.get(index as usize) {
            Some(entry) => (entry.deadline, entry.seq, entry.generation),
            None => return,
        };
        let list = self.list_of(deadline);
        if list < SLOTS {
            if let Some(entry) = self.entries.get_mut(index as usize) {
                entry.list = list as u32;
                entry.prev = NIL;
                entry.next = NIL;
            }
            if let Some(heap) = self.heaps.get_mut(list) {
                heap.push(Reverse((deadline, seq, index, generation)));
            }
            if let Some(bits) = self.occupied.first_mut() {
                *bits |= 1 << list;
            }
        } else {
            let head = self.head(list);
            if let Some(entry) = self.entries.get_mut(index as usize) {
                entry.list = list as u32;
                entry.prev = NIL;
                entry.next = head;
            }
            if let Some(old) = self.entries.get_mut(head as usize) {
                old.prev = index;
            }
            if let Some(slot) = self.heads.get_mut(list) {
                *slot = index;
            }
            if list < DUE
                && let Some(bits) = self.occupied.get_mut(list / SLOTS)
            {
                *bits |= 1 << (list % SLOTS);
            }
        }
        #[cfg(test)]
        {
            self.moves += 1;
        }
    }

    /// Takes the entry `index` off its list, a higher slot's or the due list. O(1).
    fn unlink(&mut self, index: u32) {
        let (list, prev, next) = match self.entries.get(index as usize) {
            Some(entry) => (entry.list, entry.prev, entry.next),
            None => return,
        };
        if list == NIL || (list as usize) < SLOTS {
            return;
        }
        match self.entries.get_mut(prev as usize) {
            Some(before) => before.next = next,
            None => {
                if let Some(head) = self.heads.get_mut(list as usize) {
                    *head = next;
                }
            }
        }
        if let Some(after) = self.entries.get_mut(next as usize) {
            after.prev = prev;
        }
        if let Some(entry) = self.entries.get_mut(index as usize) {
            entry.list = NIL;
            entry.prev = NIL;
            entry.next = NIL;
        }
        let list = list as usize;
        if list < DUE
            && self.head(list) == NIL
            && let Some(bits) = self.occupied.get_mut(list / SLOTS)
        {
            *bits &= !(1 << (list % SLOTS));
        }
    }

    /// Empties the list `list`, a higher slot's or the due list, into `out`. O(length).
    fn take_list(&mut self, list: usize, out: &mut Vec<u32>) {
        let mut index = self.head(list);
        while let Some(entry) = self.entries.get_mut(index as usize) {
            let next = entry.next;
            entry.list = NIL;
            entry.prev = NIL;
            entry.next = NIL;
            out.push(index);
            index = next;
        }
        if let Some(head) = self.heads.get_mut(list) {
            *head = NIL;
        }
        if list < DUE
            && let Some(bits) = self.occupied.get_mut(list / SLOTS)
        {
            *bits &= !(1 << (list % SLOTS));
        }
    }

    /// The first entry of the list `list`, or `NIL`.
    fn head(&self, list: usize) -> u32 {
        self.heads.get(list).copied().unwrap_or(NIL)
    }

    /// The occupied slot that the wheel reaches first, as its list and the time it starts, in
    /// nanoseconds: the start of its millisecond, or the wheel's time when that is later. On a
    /// tie the higher level comes first, so its timers cascade before a lower slot of the same
    /// time fires. O(levels).
    fn next_slot(&self) -> Option<(usize, u64)> {
        let now = tick(self.now);
        let mut best: Option<(usize, u64)> = None;
        for (level, &bits) in self.occupied.iter().enumerate() {
            if bits == 0 {
                continue;
            }
            let shift = level as u32 * SLOT_BITS;
            let current = ((now >> shift) as usize) & (SLOTS - 1);
            let ahead = bits & (u64::MAX << current);
            // Every occupied slot of a level lies at or after the wheel's slot in it, since a
            // timer is placed by the group in which its millisecond first exceeds the wheel's,
            // and a slot is reached before the wheel's time passes it. Should one not, it
            // starts now, and is cascaded or drained at once.
            let (slot, start) = if ahead == 0 {
                (bits.trailing_zeros() as usize, self.now)
            } else {
                let slot = ahead.trailing_zeros() as usize;
                let window = shift + SLOT_BITS;
                let level_start = if window >= u64::BITS {
                    0
                } else {
                    now & !((1u64 << window) - 1)
                };
                let start = (level_start | ((slot as u64) << shift)).saturating_mul(SLOT_NANOS);
                (slot, start.max(self.now))
            };
            let list = level * SLOTS + slot;
            if best.is_none_or(|(_, earliest)| start <= earliest) {
                best = Some((list, start));
            }
        }
        best
    }

    /// The items the heaps of level 0 hold, dead ones included, in tests.
    #[cfg(test)]
    fn heap_items(&self) -> usize {
        self.heaps.iter().map(BinaryHeap::len).sum()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// A small deterministic generator, so the tests need no dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }
    }

    fn fire(wheel: &mut Wheel<u64>, to: u64) -> Vec<u64> {
        let mut fired = Vec::new();
        wheel.advance(to, &mut fired);
        fired.into_iter().map(|(_, value)| value).collect()
    }

    #[test]
    fn fires_at_its_deadline_and_not_before() {
        let mut wheel = Wheel::new(1_000);
        wheel.arm(1_250, 1).unwrap();
        assert!(fire(&mut wheel, 1_249).is_empty());
        assert_eq!(fire(&mut wheel, 1_250), vec![1]);
        assert!(wheel.is_empty());
    }

    #[test]
    fn a_deadline_not_after_now_is_due_at_once() {
        let mut wheel = Wheel::new(500);
        wheel.arm(500, 1).unwrap();
        wheel.arm(10, 2).unwrap();
        assert_eq!(wheel.next_expiration(), Some(500));
        assert_eq!(fire(&mut wheel, 400), vec![2, 1]);
        assert_eq!(wheel.now(), 500);
    }

    #[test]
    fn cancel_gives_the_value_back_once() {
        let mut wheel = Wheel::new(0);
        let id = wheel.arm(70_000, 7).unwrap();
        assert_eq!(wheel.cancel(id), Some(7));
        assert_eq!(wheel.cancel(id), None);
        assert!(fire(&mut wheel, 100_000).is_empty());
        // A reused entry does not answer to the old id.
        let other = wheel.arm(100_001, 8).unwrap();
        assert_eq!(wheel.cancel(id), None);
        assert_eq!(wheel.cancel(other), Some(8));
    }

    #[test]
    fn a_stale_id_never_cancels_the_timer_that_reuses_its_entry() {
        let mut wheel = Wheel::new(0);
        let stale = wheel.arm(5, 1).unwrap();
        assert_eq!(fire(&mut wheel, 5), vec![1]);
        // The freed entry is reused, many times over, by timers armed and cancelled in turn.
        for value in 0..10_000 {
            let id = wheel.arm(1_000, value).unwrap();
            assert_eq!(id.index, stale.index);
            assert_eq!(wheel.cancel(id), Some(value));
        }
        let live = wheel.arm(1_000, 7).unwrap();
        assert_eq!(live.index, stale.index);
        assert_eq!(wheel.cancel(stale), None);
        assert_eq!(fire(&mut wheel, 1_000), vec![7]);
    }

    #[test]
    fn unordered_advance_fires_the_same_timers() {
        let mut ordered = Wheel::new(0);
        let mut unordered = Wheel::new(0);
        let mut random = Lcg(7);
        for value in 0..2_000u64 {
            let deadline = random.next() % 50_000;
            ordered.arm(deadline, value).unwrap();
            unordered.arm(deadline, value).unwrap();
        }
        let mut a = Vec::new();
        let mut b = Vec::new();
        ordered.advance(25_000, &mut a);
        unordered.advance_unordered(25_000, &mut b);
        let mut a: Vec<u64> = a.into_iter().map(|(_, v)| v).collect();
        let mut b: Vec<u64> = b.into_iter().map(|(_, v)| v).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        assert_eq!(ordered.len(), unordered.len());
    }

    #[test]
    fn a_fired_timer_cannot_be_cancelled() {
        let mut wheel = Wheel::new(0);
        let id = wheel.arm(5, 1).unwrap();
        assert_eq!(fire(&mut wheel, 5), vec![1]);
        assert_eq!(wheel.cancel(id), None);
    }

    #[test]
    fn timers_that_fire_together_keep_their_arming_order() {
        let mut wheel = Wheel::new(0);
        // The first goes high in the wheel, the second low, by the time it is armed.
        wheel.arm(300_000, 1).unwrap();
        wheel.advance(299_990, &mut Vec::new());
        wheel.arm(300_000, 2).unwrap();
        wheel.arm(299_995, 3).unwrap();
        assert_eq!(fire(&mut wheel, 400_000), vec![3, 1, 2]);
    }

    #[test]
    fn the_farthest_deadlines_fit() {
        let mut wheel = Wheel::new(0);
        wheel.arm(u64::MAX, 1).unwrap();
        wheel.arm(1 << 63, 2).unwrap();
        assert!(fire(&mut wheel, (1 << 63) - 1).is_empty());
        assert_eq!(fire(&mut wheel, 1 << 63), vec![2]);
        assert_eq!(fire(&mut wheel, u64::MAX), vec![1]);
    }

    #[test]
    fn next_expiration_is_a_lower_bound_reached_in_a_few_steps() {
        let mut wheel = Wheel::new(3);
        wheel.arm(9_876_543, 1).unwrap();
        let mut steps = 0;
        let mut fired = Vec::new();
        while fired.is_empty() {
            let next = wheel.next_expiration().unwrap();
            assert!(next <= 9_876_543);
            wheel.advance(next, &mut fired);
            steps += 1;
        }
        assert!(steps <= LEVELS, "{steps} steps");
        assert_eq!(wheel.now(), 9_876_543);
        assert_eq!(wheel.next_expiration(), None);
    }

    /// The wheel against a sorted map, over random arms, cancels and advances.
    #[test]
    fn agrees_with_a_sorted_map() {
        let mut random = Lcg(42);
        let mut wheel = Wheel::new(0);
        let mut model: BTreeMap<(u64, u64), TimerId> = BTreeMap::new();
        let mut live: Vec<(u64, u64, TimerId)> = Vec::new();
        let mut now = 0u64;
        for value in 0..20_000u64 {
            match random.next() % 4 {
                0 | 1 => {
                    let span = [10, 1_000, 100_000, 10_000_000][(random.next() % 4) as usize];
                    let deadline = now + random.next() % span;
                    let id = wheel.arm(deadline, value).unwrap();
                    model.insert((deadline, value), id);
                    live.push((deadline, value, id));
                }
                2 if !live.is_empty() => {
                    let at = (random.next() as usize) % live.len();
                    let (deadline, value, id) = live.swap_remove(at);
                    let expected = model.remove(&(deadline, value)).map(|_| value);
                    assert_eq!(wheel.cancel(id), expected);
                }
                _ => {
                    now += random.next() % 5_000;
                    let fired = fire(&mut wheel, now);
                    let mut expected = Vec::new();
                    while let Some((&(deadline, value), _)) = model.first_key_value() {
                        if deadline > now {
                            break;
                        }
                        model.remove(&(deadline, value));
                        expected.push(value);
                    }
                    assert_eq!(fired, expected);
                }
            }
            assert_eq!(wheel.len(), model.len());
        }
    }

    /// Arming, cancelling and firing n timers moves entries O(n) times in all: the count per
    /// timer stays flat from 1,000 to 100,000 timers, which quadratic work would not.
    #[test]
    fn work_per_timer_is_flat() {
        let per_timer = |n: u64| {
            let mut random = Lcg(n);
            let mut wheel = Wheel::new(0);
            let mut ids = Vec::new();
            for value in 0..n {
                ids.push(wheel.arm(random.next() % (n * 1_000), value).unwrap());
            }
            for id in ids.iter().step_by(2) {
                wheel.cancel(*id);
            }
            let mut fired = Vec::new();
            while let Some(next) = wheel.next_expiration() {
                wheel.advance(next, &mut fired);
            }
            assert_eq!(fired.len() as u64, n / 2);
            wheel.moves as f64 / n as f64
        };
        let small = per_timer(1_000);
        let large = per_timer(100_000);
        assert!(
            small <= LEVELS as f64 && large <= LEVELS as f64,
            "{small} {large}"
        );
        assert!(
            large <= small * 2.0 + 1.0,
            "{small} then {large} moves per timer"
        );
    }

    /// Fires the wheel to `to` with the unordered advance of the timer thread.
    fn fire_unordered(wheel: &mut Wheel<u64>, to: u64) -> Vec<u64> {
        let mut fired = Vec::new();
        wheel.advance_unordered(to, &mut fired);
        let mut values: Vec<u64> = fired.into_iter().map(|(_, value)| value).collect();
        values.sort_unstable();
        values
    }

    #[test]
    fn a_timer_fires_at_its_exact_nanosecond_whatever_its_offset_in_the_millisecond() {
        // P13, #1151: a deadline armed at any offset within a millisecond fires exactly at its
        // own nanosecond, `d` after it was armed, never at the end of its slot, for deadlines
        // that start in the current slot, in a later slot of level 0, and above level 0. The
        // wheel's origin is not on a millisecond either. On the old wheel, which compared
        // millisecond counts, every offset but 0 fired early or late.
        let mut wheel = Wheel::new(7 * SLOT_NANOS + 123);
        let mut value = 0u64;
        let offsets = (0..SLOT_NANOS).step_by(7_919).chain([1, SLOT_NANOS - 1]);
        for offset in offsets {
            for d in [
                1,
                999_999,
                SLOT_NANOS,
                5 * SLOT_NANOS + 17,
                70 * SLOT_NANOS + 1,
            ] {
                // Time passes to the offset within the next millisecond, where the timer is armed.
                let armed = (tick(wheel.now()) + 1) * SLOT_NANOS + offset;
                assert!(fire(&mut wheel, armed).is_empty());
                value += 1;
                wheel.arm(armed + d, value).unwrap();
                assert!(
                    fire(&mut wheel, armed + d - 1).is_empty(),
                    "offset {offset}, d {d}: fired early"
                );
                assert_eq!(
                    fire(&mut wheel, armed + d),
                    vec![value],
                    "offset {offset}, d {d}: not fired at its deadline"
                );
                assert_eq!(wheel.heap_items(), 0);
            }
        }
        assert!(wheel.is_empty());
    }

    #[test]
    fn a_deadline_one_nanosecond_ahead_is_not_due() {
        // The due list and the current slot compare exact deadlines: a timer whose slot has
        // begun but whose deadline has not passed waits.
        let mut wheel = Wheel::new(5 * SLOT_NANOS + 400);
        wheel.arm(5 * SLOT_NANOS + 401, 1).unwrap();
        wheel.arm(5 * SLOT_NANOS + 400, 2).unwrap();
        assert_eq!(wheel.next_expiration(), Some(5 * SLOT_NANOS + 400));
        assert_eq!(fire(&mut wheel, 5 * SLOT_NANOS + 400), vec![2]);
        assert_eq!(wheel.next_expiration(), Some(5 * SLOT_NANOS + 401));
        assert!(fire_unordered(&mut wheel, 5 * SLOT_NANOS + 400).is_empty());
        assert_eq!(fire_unordered(&mut wheel, 5 * SLOT_NANOS + 401), vec![1]);
    }

    #[test]
    fn timers_of_one_slot_fire_in_the_order_of_their_exact_deadlines() {
        // Several timers in one slot of level 0, armed in the reverse order of their deadlines,
        // each fire at their own nanosecond, one by one, in the order of their deadlines; the
        // next expiration is always the next exact deadline.
        let base = 40 * SLOT_NANOS;
        let mut wheel = Wheel::new(base + 3);
        let offsets = [900_000, 700_001, 500_000, 250_002, 10, 9];
        for (value, offset) in offsets.iter().enumerate() {
            wheel.arm(base + SLOT_NANOS + offset, value as u64).unwrap();
        }
        let mut order = offsets.to_vec();
        order.sort_unstable();
        for offset in order {
            let at = base + SLOT_NANOS + offset;
            assert_eq!(wheel.next_expiration(), Some(at));
            assert!(fire_unordered(&mut wheel, at - 1).is_empty());
            let value = offsets.iter().position(|&o| o == offset).unwrap() as u64;
            assert_eq!(fire_unordered(&mut wheel, at), vec![value]);
        }
        assert!(wheel.is_empty());
        assert_eq!(wheel.next_expiration(), None);
    }

    #[test]
    fn a_cancelled_timer_of_level_0_is_dropped_lazily_and_its_entry_rearmed() {
        // Cancel then re-arm: the cancelled timer's heap item stays, dead, until it reaches the
        // top of its heap, where it is dropped unfired; the entry is reused at once by the
        // re-armed timer, which fires at its own deadline, earlier or later than the dead one.
        let base = 3 * SLOT_NANOS;
        let mut wheel = Wheel::new(base);
        let first = wheel.arm(base + 600_000, 1).unwrap();
        assert_eq!(wheel.cancel(first), Some(1));
        let later = wheel.arm(base + 800_000, 2).unwrap();
        assert_eq!(later.index, first.index);
        assert_eq!(wheel.heap_items(), 2);
        // The stale id cancels nothing, and the dead item fires nothing.
        assert_eq!(wheel.cancel(first), None);
        assert!(fire(&mut wheel, base + 600_000).is_empty());
        assert_eq!(wheel.heap_items(), 1);
        assert!(fire(&mut wheel, base + 799_999).is_empty());
        assert_eq!(fire(&mut wheel, base + 800_000), vec![2]);
        // Re-armed earlier than the dead item, in the same slot.
        let dead = wheel.arm(base + SLOT_NANOS + 500, 3).unwrap();
        assert_eq!(wheel.cancel(dead), Some(3));
        wheel.arm(base + SLOT_NANOS + 100, 4).unwrap();
        assert!(fire(&mut wheel, base + SLOT_NANOS + 99).is_empty());
        assert_eq!(fire(&mut wheel, base + SLOT_NANOS + 100), vec![4]);
        assert!(fire(&mut wheel, base + 2 * SLOT_NANOS).is_empty());
        assert_eq!(wheel.heap_items(), 0);
        assert!(wheel.is_empty());
    }

    #[test]
    fn long_deadlines_cancelled_at_a_high_rate_leave_nothing_behind() {
        // Lazy deletion is confined to level 0: `within 1 hour` armed and cancelled 20,000
        // times leaves no dead item, and its entry is reused, so memory stays flat. Short ones
        // leave dead items only until time passes their millisecond.
        let hour = 3_600_000 * SLOT_NANOS;
        let mut wheel = Wheel::new(11);
        for value in 0..20_000u64 {
            let id = wheel.arm(wheel.now() + hour, value).unwrap();
            assert_eq!(wheel.cancel(id), Some(value));
        }
        assert_eq!(wheel.heap_items(), 0);
        assert_eq!(wheel.entries.len(), 1);
        for value in 0..1_000u64 {
            let id = wheel.arm(wheel.now() + 10 * SLOT_NANOS, value).unwrap();
            assert_eq!(wheel.cancel(id), Some(value));
        }
        assert_eq!(wheel.heap_items(), 1_000);
        assert!(fire(&mut wheel, 64 * SLOT_NANOS).is_empty());
        assert_eq!(wheel.heap_items(), 0);
        assert!(wheel.is_empty());
    }
}
