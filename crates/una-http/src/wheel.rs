//! The timing wheel of a run's timers (ADR 0117 rule 5 and Costs, slice S2,
//! `docs/plan/0117-s2-design.md`; ADR 0116 amendment of 2026-10-25, P13, #1151).
//!
//! A `within` arms one timer when it starts and cancels it when its expression ends, both in
//! O(1). The wheel counts time in **nanoseconds** of the run's monotonic clock: a slot of level
//! 0 is one nanosecond, so a timer fires at its own nanosecond, never at the end of a coarser
//! slot (P13). The wheel is hierarchical: 11 levels of 64 slots each, where level `L` holds the
//! timers whose deadline first differs from the wheel's time in the 6-bit group `L` of the
//! nanosecond count. Eleven groups of 6 bits cover every `u64`, about 584 years of nanoseconds,
//! so no deadline is too far for the wheel and nothing is clamped or wrapped. Each slot is an
//! intrusive doubly linked list over one slab of entries, so arming and cancelling are O(1),
//! and each level keeps a bitmap of its occupied slots, so the next slot to expire is found in
//! O(levels).
//!
//! As time advances, the earliest occupied slot is reached: a level-0 slot holds the timers of
//! exactly one nanosecond, which fire; a higher slot is cascaded, each of its timers moving to
//! a lower level. A timer moves down at most 10 times in its life, so firing costs O(1)
//! amortized per timer.
//!
//! **No state from ended code.** Removal is eager everywhere: a cancelled timer leaves its list
//! at once and its entry goes back to the free list, so after any history of arming and
//! cancelling the wheel holds exactly its live timers, and every occupied slot holds a live
//! timer. So [`Wheel::next_expiration`] is always the moment of a live timer, its exact
//! deadline, or the start of a slot that holds it and is to be cascaded.
//!
//! **Costs.** Arming, cancelling and firing are O(1), firing amortized over at most 10 moves
//! down the levels per timer, each O(1); finding the next expiration is O(levels). Space: one
//! slab entry per live timer, reused after it fires or is cancelled.
//!
//! **Determinism.** Timers that fire together come out ordered by deadline, then by the order
//! they were armed, whatever path they took through the levels, so a run under virtual time
//! (ADR 0116 rule 19) sees the same order in every engine.
//!
//! **Time never goes back.** [`Wheel::advance`] to a time before the wheel's own is an advance
//! to the wheel's time. A timer armed with a deadline at or before the wheel's time is due at
//! once, and fires at the next advance.

/// Bits of a nanosecond count that one level resolves.
const SLOT_BITS: u32 = 6;

/// Slots in each level.
const SLOTS: usize = 1 << SLOT_BITS;

/// Levels: 11 groups of 6 bits cover the 64 bits of a deadline.
const LEVELS: usize = 11;

/// The list of timers already due, after the `LEVELS * SLOTS` lists of the slots.
const DUE: usize = LEVELS * SLOTS;

/// The end of a list, and the list of a free entry.
const NIL: u32 = u32::MAX;

/// A timer armed in a [`Wheel`]: its entry and that entry's generation, so that cancelling a
/// timer that already fired, or was cancelled, does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimerId {
    index: u32,
    /// 64 bits, so an entry is reused far fewer times than it would take to wrap: a stale id
    /// never matches a live timer.
    generation: u64,
}

/// The wheel cannot hold another timer: it already holds `u32::MAX - 1`. Timers are bounded by
/// the run's tasks, which `--max-memory` bounds long before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WheelFull;

/// One entry of the slab: a timer, or a free entry when `value` is `None`.
struct Entry<T> {
    deadline: u64,
    /// The order the timer was armed in, to order timers that fire together.
    seq: u64,
    generation: u64,
    /// The list the entry is on: a slot (`level * SLOTS + slot`), [`DUE`], or `NIL` when free.
    list: u32,
    prev: u32,
    next: u32,
    value: Option<T>,
}

/// The vectors of the wheel that grow with its timers: `Vec` itself in every build but the
/// unit tests of this crate, where [`counted::Slab`] is a `Vec` that counts each access for the
/// test of the steps of [`Wheel::arm`] and [`Wheel::cancel`] (#2130).
#[cfg(not(test))]
type Slab<T> = Vec<T>;
#[cfg(test)]
use counted::Slab;

/// The growing vector of the wheel in the unit tests (#2130): a `Vec` that gives only the
/// accesses the wheel uses and counts each one on its thread, one step for an access to one
/// element or to the length, and one step per element for the one walk the wheel makes
/// ([`Wheel::into_armed`]). It gives no slice, no index and no borrowed iterator, so a walk of
/// the slab or of the free list, or of a slot's list through the slab, is counted element by
/// element, whatever the machine's load.
///
/// Invariant: every method of `Slab` counts, at least one step per element it reaches. A
/// method added without its count, or a way to the inner `Vec`, takes the walk it allows out
/// of the count.
#[cfg(test)]
mod counted {
    use std::cell::Cell;

    thread_local! {
        /// The accesses to the wheels' growing vectors made on this thread.
        static STEPS: Cell<u64> = const { Cell::new(0) };
    }

    /// The accesses to the wheels' growing vectors made on this thread so far.
    pub(super) fn steps() -> u64 {
        STEPS.with(Cell::get)
    }

    fn step(count: u64) {
        STEPS.with(|steps| steps.set(steps.get() + count));
    }

    /// A `Vec` whose every access counts.
    pub(super) struct Slab<T>(Vec<T>);

    impl<T> Slab<T> {
        pub(super) fn with_capacity(capacity: usize) -> Slab<T> {
            Slab(Vec::with_capacity(capacity))
        }

        pub(super) fn len(&self) -> usize {
            step(1);
            self.0.len()
        }

        pub(super) fn get(&self, index: usize) -> Option<&T> {
            step(1);
            self.0.get(index)
        }

        pub(super) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
            step(1);
            self.0.get_mut(index)
        }

        pub(super) fn push(&mut self, value: T) {
            step(1);
            self.0.push(value);
        }

        pub(super) fn pop(&mut self) -> Option<T> {
            step(1);
            self.0.pop()
        }
    }

    /// The walk of every element, by value: one step for the walk and one per element.
    impl<T> IntoIterator for Slab<T> {
        type Item = T;
        type IntoIter = std::vec::IntoIter<T>;

        fn into_iter(self) -> std::vec::IntoIter<T> {
            step(1 + self.0.len() as u64);
            self.0.into_iter()
        }
    }
}

/// A hierarchical timing wheel over a clock in nanoseconds. See the module documentation.
///
/// Invariant of the counted steps (#2130): the two collections that grow with the timers,
/// `entries` and `free`, are `Slab`s, counted in the unit tests. The others cannot hold a
/// cost that grows with the timers between two calls: `heads` keeps its `LEVELS * SLOTS + 1`
/// lists and `occupied` its `LEVELS` words for good, and `batch` and `due` are empty outside
/// [`Wheel::advance`]. A collection added here that grows with the timers must be a `Slab`;
/// the test `arming_and_cancelling_take_constant_steps` names every field and its type, so it
/// does not compile until the new field is listed there.
pub struct Wheel<T> {
    now: u64,
    heads: Vec<u32>,
    occupied: [u64; LEVELS],
    entries: Slab<Entry<T>>,
    free: Slab<u32>,
    len: usize,
    seq: u64,
    /// Lists taken out while advancing, kept for their capacity so that advancing allocates
    /// nothing once warm.
    batch: Vec<u32>,
    due: Vec<u32>,
    /// Entry moves between lists, for the scaling tests.
    #[cfg(test)]
    moves: u64,
}

impl<T> Wheel<T> {
    /// An empty wheel whose time is `now`, in nanoseconds.
    pub fn new(now: u64) -> Wheel<T> {
        Wheel::with_capacity(now, 0)
    }

    /// An empty wheel whose time is `now`, in nanoseconds, with room for `timers` timers in its
    /// slab, its free list and the lists it takes out while advancing, so that arming up to
    /// that many, and firing them, allocates nothing.
    pub fn with_capacity(now: u64, timers: usize) -> Wheel<T> {
        Wheel {
            now,
            heads: vec![NIL; DUE + 1],
            occupied: [0; LEVELS],
            entries: Slab::with_capacity(timers),
            free: Slab::with_capacity(timers),
            len: 0,
            seq: 0,
            batch: Vec::with_capacity(timers),
            due: Vec::with_capacity(timers),
            #[cfg(test)]
            moves: 0,
        }
    }

    /// The wheel's time: the last time it was advanced to.
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

    /// Arms a timer that fires with `value` once the wheel reaches `deadline`. O(1).
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

    /// Every timer still armed, with its deadline, in no particular order: what a serve thread
    /// that ends hands on to another. O(slab size).
    pub fn into_armed(self) -> Vec<(u64, T)> {
        self.entries
            .into_iter()
            .filter_map(|entry| entry.value.map(|value| (entry.deadline, value)))
            .collect()
    }

    /// Cancels the timer `id` and gives its value back, or `None` when it already fired or was
    /// cancelled. O(1).
    pub fn cancel(&mut self, id: TimerId) -> Option<T> {
        let entry = self.entries.get(id.index as usize)?;
        if entry.generation != id.generation || entry.value.is_none() {
            return None;
        }
        self.unlink(id.index);
        self.release(id.index)
    }

    /// The earliest time at which a timer may fire: no timer fires before it, and a timer due
    /// now gives the wheel's time. `None` when no timer is armed. O(levels).
    ///
    /// Every occupied slot holds a live timer, since removal is eager. It is that timer's exact
    /// deadline when the earliest slot is of level 0, one nanosecond wide. Otherwise it is the
    /// start of a slot that only cascades, and a caller that waits for the next timer (the
    /// timer thread, or virtual time that jumps) advances to it and asks again, at most 10
    /// times per timer.
    pub fn next_expiration(&self) -> Option<u64> {
        if self.head(DUE) != NIL {
            return Some(self.now);
        }
        self.next_slot().map(|(_, start)| start)
    }

    /// Advances the wheel to `to` and appends to `fired` every timer whose deadline is at or
    /// before it, ordered by deadline and then by the order they were armed. A `to` before the
    /// wheel's time advances to the wheel's time. O(1) amortized per timer, plus O(levels) per
    /// slot reached.
    pub fn advance(&mut self, to: u64, fired: &mut Vec<(TimerId, T)>) {
        self.advance_in(to, fired, true);
    }

    /// [`Wheel::advance`] without ordering the timers that fire together: O(1) amortized per
    /// timer, with no allocation once `fired` and the wheel's own lists have grown, for a
    /// caller that holds a lock while it advances and has no use for the order (the real
    /// clock's timer thread).
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

    /// Frees the entry `index`, already off every list, and gives its value back.
    fn release(&mut self, index: u32) -> Option<T> {
        let entry = self.entries.get_mut(index as usize)?;
        let value = entry.value.take()?;
        entry.generation = entry.generation.wrapping_add(1);
        entry.list = NIL;
        self.free.push(index);
        self.len -= 1;
        Some(value)
    }

    /// The list a deadline belongs on at the wheel's time: [`DUE`] when it is not after it,
    /// else the slot of the highest 6-bit group in which it differs from the wheel's time, in
    /// nanoseconds.
    fn list_of(&self, deadline: u64) -> usize {
        if deadline <= self.now {
            return DUE;
        }
        let differs = deadline ^ self.now;
        let level = ((63 - differs.leading_zeros()) / SLOT_BITS) as usize;
        let slot = ((deadline >> (level as u32 * SLOT_BITS)) as usize) & (SLOTS - 1);
        level * SLOTS + slot
    }

    /// Puts the entry `index`, off every list, on the list of its deadline. O(1).
    fn place(&mut self, index: u32) {
        let deadline = match self.entries.get(index as usize) {
            Some(entry) => entry.deadline,
            None => return,
        };
        let list = self.list_of(deadline);
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
        #[cfg(test)]
        {
            self.moves += 1;
        }
    }

    /// Takes the entry `index` off its list. O(1).
    fn unlink(&mut self, index: u32) {
        let (list, prev, next) = match self.entries.get(index as usize) {
            Some(entry) => (entry.list, entry.prev, entry.next),
            None => return,
        };
        if list == NIL {
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

    /// Empties the list `list` into `out`. O(length).
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

    /// How many slots are occupied, over every level, and whether the due list holds a timer,
    /// in tests.
    #[cfg(test)]
    fn occupied_slots(&self) -> (u32, bool) {
        (
            self.occupied.iter().map(|bits| bits.count_ones()).sum(),
            self.head(DUE) != NIL,
        )
    }

    /// The occupied slot that the wheel reaches first, as its list and the time it starts.
    /// On a tie the higher level comes first, so its timers cascade before a lower slot of the
    /// same time fires. O(levels).
    fn next_slot(&self) -> Option<(usize, u64)> {
        let mut best: Option<(usize, u64)> = None;
        for (level, &bits) in self.occupied.iter().enumerate() {
            if bits == 0 {
                continue;
            }
            let shift = level as u32 * SLOT_BITS;
            let current = ((self.now >> shift) as usize) & (SLOTS - 1);
            let ahead = bits & (u64::MAX << current);
            // Every occupied slot of a level lies at or after the wheel's slot in it, since a
            // timer is placed by the group in which its deadline first exceeds the wheel's
            // time. Should one not, it starts now, and is cascaded or fired at once.
            let (slot, start) = if ahead == 0 {
                (bits.trailing_zeros() as usize, self.now)
            } else {
                let slot = ahead.trailing_zeros() as usize;
                let window = shift + SLOT_BITS;
                let level_start = if window >= u64::BITS {
                    0
                } else {
                    self.now & !((1u64 << window) - 1)
                };
                let start = level_start | ((slot as u64) << shift);
                (slot, start.max(self.now))
            };
            let list = level * SLOTS + slot;
            if best.is_none_or(|(_, earliest)| start <= earliest) {
                best = Some((list, start));
            }
        }
        best
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
        // The freed entry is reused, many times over, by timers armed and cancelled in turn:
        // 200 times under Miri, where each step is slow (#1239), 10,000 otherwise.
        let rounds = if cfg!(miri) { 200 } else { 10_000 };
        for value in 0..rounds {
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
        // 200 timers under Miri (#1239), 2,000 otherwise.
        let timers = if cfg!(miri) { 200 } else { 2_000u64 };
        for value in 0..timers {
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

    /// The level of the list the live timer `id` is on, in tests.
    fn level(wheel: &Wheel<u64>, id: TimerId) -> usize {
        wheel.entries.get(id.index as usize).unwrap().list as usize / SLOTS
    }

    #[test]
    fn timers_that_fire_together_keep_their_arming_order() {
        let mut wheel = Wheel::new(0);
        // The first goes high in the wheel, the second low, by the time it is armed: one
        // minute of nanoseconds differs from 0 in group 5, and from one minute less 10 ns only
        // in group 0 or 1.
        let minute = 60_000_000_000u64;
        let high = wheel.arm(minute, 1).unwrap();
        assert!(level(&wheel, high) >= 5, "level {}", level(&wheel, high));
        wheel.advance(minute - 10, &mut Vec::new());
        let low = wheel.arm(minute, 2).unwrap();
        assert!(level(&wheel, low) <= 1, "level {}", level(&wheel, low));
        wheel.arm(minute - 5, 3).unwrap();
        assert_eq!(fire(&mut wheel, minute + 1), vec![3, 1, 2]);
    }

    #[test]
    fn a_timer_fires_at_its_exact_nanosecond_whatever_its_offset_in_the_millisecond() {
        // P13, #1151: a deadline armed at any offset within a millisecond fires exactly at its
        // own nanosecond, `d` after it was armed, for `d` that stays in level 0 and `d` that
        // cascades down from high levels; the wheel's origin is not on a millisecond either.
        let ms = 1_000_000u64;
        let stride = if cfg!(miri) { 99_991 } else { 7_919 };
        let mut wheel = Wheel::new(7 * ms + 123);
        let mut value = 0u64;
        for offset in (0..ms).step_by(stride).chain([1, ms - 1]) {
            for d in [1, 63, 999_999, ms, 5 * ms + 17, 70 * ms + 1, 61_000 * ms] {
                let armed = (wheel.now() / ms + 1) * ms + offset;
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
            }
        }
        assert!(wheel.is_empty());
        assert_eq!(wheel.occupied_slots(), (0, false));
    }

    #[test]
    fn after_any_history_of_cancels_the_wheel_holds_nothing() {
        // #1151, the deterministic guard of M1: removal is eager, so 20,000 timers of mixed
        // lengths, from a nanosecond to a year, armed and cancelled in any order, leave no
        // item and no occupied slot, and their entries are reused, so the wheel's state after
        // such a history is that of a wheel that never saw it.
        let mut random = Lcg(1151);
        let mut wheel = Wheel::new(5);
        let spans = [
            1u64,
            1_000,
            1_000_000,
            60_000_000_000,
            31_536_000_000_000_000,
        ];
        let mut live = Vec::new();
        for value in 0..20_000u64 {
            let span = spans[(random.next() % spans.len() as u64) as usize];
            let at = wheel.now() + 1 + random.next() % span;
            live.push(wheel.arm(at, value).unwrap());
            if random.next().is_multiple_of(3) {
                let at = (random.next() as usize) % live.len();
                let id = live.swap_remove(at);
                assert!(wheel.cancel(id).is_some());
            }
        }
        for id in live.drain(..) {
            assert!(wheel.cancel(id).is_some());
        }
        assert!(wheel.is_empty());
        assert_eq!(wheel.occupied_slots(), (0, false));
        assert_eq!(wheel.next_expiration(), None);
        assert_eq!(wheel.free.len(), wheel.entries.len());
        // Nothing fires, and a timer armed now behaves as on a fresh wheel.
        assert!(fire(&mut wheel, u64::MAX / 2).is_empty());
        wheel.arm(u64::MAX / 2 + 10, 7).unwrap();
        assert_eq!(
            wheel.next_expiration().map(|at| at <= u64::MAX / 2 + 10),
            Some(true)
        );
        assert_eq!(fire(&mut wheel, u64::MAX / 2 + 10), vec![7]);
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
        // 500 operations under Miri (#1239), 20,000 otherwise: the same mix of arms, cancels
        // and advances, checked against the model at every step.
        let operations = if cfg!(miri) { 500 } else { 20_000u64 };
        for value in 0..operations {
            match random.next() % 4 {
                0 | 1 => {
                    // In nanoseconds: within one level-0 window, a millisecond, a second and
                    // an hour, so timers cascade through most levels against the model.
                    let span = [10, 1_000_000, 1_000_000_000, 3_600_000_000_000]
                        [(random.next() % 4) as usize];
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
                    now += random.next()
                        % [5_000, 5_000_000, 5_000_000_000][(random.next() % 3) as usize];
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
                ids.push(wheel.arm(random.next() % (n * 1_000_000), value).unwrap());
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
        // Under Miri 200 then 2,000 timers (#1239), else 1,000 then 100,000: flat either way.
        let (few, many) = if cfg!(miri) {
            (200, 2_000)
        } else {
            (1_000, 100_000)
        };
        let small = per_timer(few);
        let large = per_timer(many);
        assert!(
            small <= LEVELS as f64 && large <= LEVELS as f64,
            "{small} {large}"
        );
        assert!(
            large <= small * 2.0 + 1.0,
            "{small} then {large} moves per timer"
        );
    }

    /// The most accesses to the slab and to the free list that one call makes, whatever the
    /// number of timers (#2130), each read off the code path.
    ///
    /// `arm` of a new entry: `free.pop`, `entries.len`, `entries.push`, `entries.get_mut` for
    /// the deadline, then `place`: `entries.get` for the deadline, `entries.get_mut` for the
    /// links, `entries.get_mut` for the old head of the list (asked for even when the list is
    /// empty).
    const ARM_STEPS: u64 = 7;
    /// `arm` that reuses a freed entry: `free.pop`, `entries.get_mut`, and the three of
    /// `place`.
    const REARM_STEPS: u64 = 5;
    /// `cancel` of a live timer: `entries.get` for the generation, then `unlink`:
    /// `entries.get` for the links and `entries.get_mut` for the entry before, the entry after
    /// and the entry itself (the first two asked for even at an end of the list), then
    /// `release`: `entries.get_mut` and `free.push`.
    const CANCEL_STEPS: u64 = 7;
    /// `cancel` of a timer that is gone: `entries.get` for the generation.
    const STALE_STEPS: u64 = 1;

    /// Arms `count` timers, cancels them, cancels them again, arms `count` more into the freed
    /// entries and cancels those: the most counted steps of one call of each of the five
    /// passes, and the steps of all.
    ///
    /// Every deadline lies in one slot (group 6 of a time near 2^40 against a wheel at 0), so
    /// that slot's list holds every live timer: a walk of the list, of the slab or of the free
    /// list is as long as the wheel is full. The first timers are cancelled in the order they
    /// were armed, from the tail of the list, and the second in the reverse order, from its
    /// head, so a walk from either end meets its longest case. The second arming starts with
    /// `count` freed entries, the longest free list.
    fn arm_cancel_rearm(count: u64) -> ([u64; 5], u64) {
        let mut wheel: Wheel<u64> = Wheel::new(0);
        let far = 1u64 << 40;
        let mut most = [0u64; 5];
        let keep = |most: &mut u64, before: u64| {
            *most = (*most).max(counted::steps() - before);
        };
        let first = counted::steps();
        let mut ids = Vec::new();
        for value in 0..count {
            let before = counted::steps();
            let id = wheel.arm(far + value, value).unwrap();
            keep(&mut most[0], before);
            ids.push(id);
        }
        assert_eq!(wheel.len() as u64, count);
        assert_eq!(wheel.occupied_slots(), (1, false));
        for (value, id) in ids.iter().enumerate() {
            let before = counted::steps();
            let cancelled = wheel.cancel(*id);
            keep(&mut most[1], before);
            assert_eq!(cancelled, Some(value as u64));
        }
        assert!(wheel.is_empty());
        for id in &ids {
            let before = counted::steps();
            let cancelled = wheel.cancel(*id);
            keep(&mut most[2], before);
            assert_eq!(cancelled, None);
        }
        ids.clear();
        for value in 0..count {
            let before = counted::steps();
            let id = wheel.arm(far + value, value).unwrap();
            keep(&mut most[3], before);
            ids.push(id);
        }
        assert_eq!(wheel.len() as u64, count);
        for (value, id) in ids.iter().enumerate().rev() {
            let before = counted::steps();
            let cancelled = wheel.cancel(*id);
            keep(&mut most[4], before);
            assert_eq!(cancelled, Some(value as u64));
        }
        let steps = counted::steps() - first;
        // The shape of the wheel (#2130): every field by name and type, with no `..`, so a
        // field added to `Wheel` does not compile here until it is listed. A collection that
        // grows with the timers must be a counted `Slab`; the others are of a fixed size or
        // empty between two calls, which the checks below hold them to.
        let Wheel {
            now,
            heads,
            occupied,
            entries,
            free,
            len,
            seq,
            batch,
            due,
            moves,
        } = wheel;
        let (entries, free): (counted::Slab<Entry<u64>>, counted::Slab<u32>) = (entries, free);
        let (heads, occupied): (Vec<u32>, [u64; LEVELS]) = (heads, occupied);
        let (batch, due): (Vec<u32>, Vec<u32>) = (batch, due);
        let (now, len, seq, moves): (u64, usize, u64, u64) = (now, len, seq, moves);
        assert_eq!(heads.len(), DUE + 1);
        assert!(heads.iter().all(|head| *head == NIL));
        assert_eq!(occupied, [0; LEVELS]);
        assert!(batch.is_empty() && due.is_empty());
        assert_eq!((now, len, seq, moves), (0, 0, 2 * count, 2 * count));
        // The second arming took its entries from the free list: the slab did not grow.
        assert_eq!(entries.len() as u64, count);
        assert_eq!(free.len() as u64, count);
        (most, steps)
    }

    /// `arm` and `cancel` are O(1), counted (#2130): no call takes more accesses to the slab
    /// and the free list at 100,000 timers than its constant, and the steps of all are exactly
    /// the constants times the timers. A scan added to either (a sorted insert into a slot's
    /// list, a search of the slab for an id, a walk of the free list) fails here at every
    /// size, on any machine and under any load: nothing is timed.
    #[test]
    fn arming_and_cancelling_take_constant_steps() {
        for count in [1_000u64, 10_000, 100_000] {
            let (most, steps) = arm_cancel_rearm(count);
            assert_eq!(
                most,
                [
                    ARM_STEPS,
                    CANCEL_STEPS,
                    STALE_STEPS,
                    REARM_STEPS,
                    CANCEL_STEPS
                ],
                "{count} timers: the most steps of an arm, a cancel, a stale cancel, an arm \
                 into a freed entry and a cancel"
            );
            assert_eq!(
                steps,
                count * (ARM_STEPS + CANCEL_STEPS + STALE_STEPS + REARM_STEPS + CANCEL_STEPS),
                "{count} timers"
            );
        }
    }
}
