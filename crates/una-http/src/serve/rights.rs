//! The reply rights of a run (ADR 0123 rule 4, `spec/31 §1` rule 3): which task holds the right
//! to answer each exchange that `next` delivered, in both engines.
//!
//! - **A right** is one 64-bit word, `(slot << 32) | generation`, a handle into this table. The
//!   program cannot read either half, and no constructor makes one: `next` alone does. Two
//!   rights are equal when their words are, so `==` names one exchange. A slot's generation
//!   grows at every reuse, and a slot whose generation would wrap is retired, so a word is never
//!   given to two exchanges of one run.
//! - **The holder.** Each live right has one holder, a task, and each task keeps the rights it
//!   holds in the order it received them (rule 4.5 answers them in that order). `next` gives the
//!   right to the task that called it (rule 4.1), `start` moves it to the started task (rule
//!   4.2, [`Rights::transfer`]), and `reply` checks the holder (rule 4.4): a task that does not
//!   hold a right, or holds it no more, gets [`NotHeld`].
//! - **The end of a task** takes every right it still holds, in order ([`Rights::end`]), for the
//!   engine's auto-answers.
//!
//! Every operation is O(1) amortized: a right's entry records where it stands in its holder's
//! list, a removal leaves a hole, and a list is compacted when most of it is holes. The table is
//! behind one lock, held for O(1) work and never while an exchange is answered: a reply takes
//! the exchange out ([`Rights::take`]), answers it with no lock held, and then releases the
//! right or puts the exchange back. Only the holder ever takes, moves or ends a right, and a
//! task runs one operation at a time, so nothing else can touch an exchange while it is out.
//!
//! A panic with the lock held (none of this code panics) poisons it: the table may be
//! inconsistent, so from then on it holds no right for anyone, every `reply` is [`NotHeld`], and
//! [`Rights::deliver`] gives the exchange back to its caller, which answers it. That fails
//! closed: nothing is answered twice, and nothing answers a right its task may not hold.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// A task of the run, as the engine numbers it: the root is 0.
pub type Task = u64;

/// A reply right as the program holds it: `(slot << 32) | generation`.
pub type Word = u64;

/// `reply` for a right that the calling task does not hold, or holds no more (rule 4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotHeld;

/// Holes a held list may hold before its compaction is worth it.
const SLACK: usize = 16;

/// The rights of one run, over the exchanges `X` of its engine.
pub struct Rights<X> {
    table: Mutex<Table<X>>,
}

struct Table<X> {
    slots: Vec<Slot<X>>,
    /// Slots whose entry ended, ready for reuse with their next generation.
    free: Vec<u32>,
    /// The rights each task holds, in the order it received them.
    held: HashMap<Task, Held>,
}

struct Slot<X> {
    generation: u32,
    entry: Option<Entry<X>>,
}

struct Entry<X> {
    holder: Task,
    /// Where the right stands in its holder's list.
    at: usize,
    /// The exchange; `None` while its holder answers it.
    exchange: Option<X>,
}

/// The rights of one task: holes where a right left, and how many are not holes.
#[derive(Default)]
struct Held {
    order: Vec<Option<Word>>,
    live: usize,
}

fn word(slot: u32, generation: u32) -> Word {
    (u64::from(slot) << 32) | u64::from(generation)
}

fn parts(right: Word) -> (usize, u32) {
    // The halves of a word: `as` keeps exactly the bits each half holds.
    ((right >> 32) as u32 as usize, right as u32)
}

impl<X> Default for Rights<X> {
    fn default() -> Self {
        Rights::new()
    }
}

impl<X> Rights<X> {
    /// A run's table, with no right.
    pub fn new() -> Rights<X> {
        Rights {
            table: Mutex::new(Table {
                slots: Vec::new(),
                free: Vec::new(),
                held: HashMap::new(),
            }),
        }
    }

    /// The table, or `None` once a panic left it poisoned, which then holds no right.
    fn table(&self) -> Option<MutexGuard<'_, Table<X>>> {
        self.table.lock().ok()
    }

    /// `next` delivered `exchange` to `task`: a fresh right, which `task` holds, last in its
    /// list (rule 4.1). The exchange comes back when the table cannot take it (it is poisoned,
    /// or every slot's generations are spent), and the caller answers it. O(1) amortized.
    pub fn deliver(&self, task: Task, exchange: X) -> Result<Word, X> {
        let Some(mut table) = self.table() else {
            return Err(exchange);
        };
        let slot = match table.free.pop() {
            Some(slot) => slot,
            None => match u32::try_from(table.slots.len()) {
                Ok(slot) => {
                    table.slots.push(Slot {
                        generation: 1,
                        entry: None,
                    });
                    slot
                }
                Err(_) => return Err(exchange),
            },
        };
        let Some(generation) = table.slots.get(slot as usize).map(|s| s.generation) else {
            return Err(exchange);
        };
        let right = word(slot, generation);
        let at = table.push(task, right);
        if let Some(cell) = table.slots.get_mut(slot as usize) {
            cell.entry = Some(Entry {
                holder: task,
                at,
                exchange: Some(exchange),
            });
        }
        Ok(right)
    }

    /// Whether `task` holds `right`.
    pub fn holds(&self, task: Task, right: Word) -> bool {
        self.table()
            .is_some_and(|table| table.entry(right).is_some_and(|e| e.holder == task))
    }

    /// `reply` begins: the exchange of `right`, which `task` must hold, taken out for its answer;
    /// the right stays held until [`Rights::release`] or [`Rights::restore`]. [`NotHeld`] when
    /// `task` does not hold it (rule 4.4), and nothing is taken. O(1).
    pub fn take(&self, task: Task, right: Word) -> Result<X, NotHeld> {
        let mut table = self.table().ok_or(NotHeld)?;
        let entry = table.entry_mut(right).ok_or(NotHeld)?;
        if entry.holder != task {
            return Err(NotHeld);
        }
        entry.exchange.take().ok_or(NotHeld)
    }

    /// The answer of a taken exchange failed its checks (`BadReply`): the exchange goes back, and
    /// `task` still holds the right (rule 4.4, `spec/31 §5` rule 1). When the right is no
    /// longer `task`'s, which no engine does, the exchange is dropped, which answers it.
    pub fn restore(&self, task: Task, right: Word, exchange: X) {
        let Some(mut table) = self.table() else {
            return;
        };
        if let Some(entry) = table.entry_mut(right)
            && entry.holder == task
            && entry.exchange.is_none()
        {
            entry.exchange = Some(exchange);
        }
    }

    /// The answer of a taken exchange was handed over, or discarded by another transition of
    /// rule 4.7: `task` no longer holds the right, and no task ever will. O(1) amortized.
    pub fn release(&self, task: Task, right: Word) {
        let Some(mut table) = self.table() else {
            return;
        };
        if table.entry(right).is_some_and(|e| e.holder == task) {
            table.free_slot(right);
        }
    }

    /// `start` moves `right` from `from`, the task that runs it, to `to`, the task it starts,
    /// when `from` holds it (rule 4.2): `true` when it moved, and it is then last in `to`'s list.
    /// A right `from` does not hold stays where it is. O(1) amortized.
    pub fn transfer(&self, from: Task, to: Task, right: Word) -> bool {
        let Some(mut table) = self.table() else {
            return false;
        };
        let Some(&Entry { holder, at, .. }) = table.entry(right) else {
            return false;
        };
        if holder != from || from == to {
            return false;
        }
        table.unlist(from, at);
        let at = table.push(to, right);
        if let Some(entry) = table.entry_mut(right) {
            entry.holder = to;
            entry.at = at;
        }
        true
    }

    /// The end of `task` (rule 4.5): every right it still holds, released, with its exchange,
    /// in the order the task received them. An exchange its task had taken out and not given
    /// back is not among them: its answer was under way. O(rights held).
    pub fn end(&self, task: Task) -> Vec<(Word, X)> {
        let Some(mut table) = self.table() else {
            return Vec::new();
        };
        let Some(held) = table.held.remove(&task) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(held.live);
        for right in held.order.into_iter().flatten() {
            let Some(slot) = table.slot_mut(right) else {
                continue;
            };
            let exchange = slot.entry.take().and_then(|entry| entry.exchange);
            table.retire_or_free(right);
            if let Some(exchange) = exchange {
                out.push((right, exchange));
            }
        }
        out
    }

    /// The rights `task` holds, in the order received, for tests.
    pub fn held_by(&self, task: Task) -> Vec<Word> {
        self.table()
            .and_then(|table| {
                table
                    .held
                    .get(&task)
                    .map(|held| held.order.iter().flatten().copied().collect())
            })
            .unwrap_or_default()
    }

    /// The live rights of the run, for tests.
    pub fn live(&self) -> usize {
        self.table().map_or(0, |table| {
            table.slots.iter().filter(|s| s.entry.is_some()).count()
        })
    }
}

impl<X> Table<X> {
    fn slot_mut(&mut self, right: Word) -> Option<&mut Slot<X>> {
        let (slot, generation) = parts(right);
        self.slots
            .get_mut(slot)
            .filter(|cell| cell.generation == generation)
    }

    fn entry(&self, right: Word) -> Option<&Entry<X>> {
        let (slot, generation) = parts(right);
        self.slots
            .get(slot)
            .filter(|cell| cell.generation == generation)
            .and_then(|cell| cell.entry.as_ref())
    }

    fn entry_mut(&mut self, right: Word) -> Option<&mut Entry<X>> {
        self.slot_mut(right).and_then(|cell| cell.entry.as_mut())
    }

    /// Appends `right` to `task`'s list: where it stands.
    fn push(&mut self, task: Task, right: Word) -> usize {
        let held = self.held.entry(task).or_default();
        held.order.push(Some(right));
        held.live += 1;
        held.order.len() - 1
    }

    /// Leaves a hole where `task`'s right at `at` stood, and compacts the list when most of it
    /// is holes, renumbering the entries that stay: O(1) amortized, the order kept.
    fn unlist(&mut self, task: Task, at: usize) {
        let Some(held) = self.held.get_mut(&task) else {
            return;
        };
        if let Some(cell) = held.order.get_mut(at)
            && cell.take().is_some()
        {
            held.live = held.live.saturating_sub(1);
        }
        if held.live == 0 {
            self.held.remove(&task);
            return;
        }
        if held.order.len() <= SLACK || held.live * 2 > held.order.len() {
            return;
        }
        let kept: Vec<Option<Word>> = held.order.drain(..).filter(Option::is_some).collect();
        held.order = kept;
        let moved: Vec<(usize, Word)> = held
            .order
            .iter()
            .enumerate()
            .filter_map(|(at, right)| right.map(|right| (at, right)))
            .collect();
        for (at, right) in moved {
            if let Some(entry) = self.entry_mut(right) {
                entry.at = at;
            }
        }
    }

    /// Ends the entry of `right`: out of its holder's list, its slot free for reuse.
    fn free_slot(&mut self, right: Word) {
        let Some(slot) = self.slot_mut(right) else {
            return;
        };
        let Some(entry) = slot.entry.take() else {
            return;
        };
        self.unlist(entry.holder, entry.at);
        self.retire_or_free(right);
    }

    /// The slot of `right`, whose entry is gone, takes its next generation and goes back to the
    /// free list, or is retired when its generations are spent.
    fn retire_or_free(&mut self, right: Word) {
        let (index, generation) = parts(right);
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        if slot.generation != generation {
            return;
        }
        match generation.checked_add(1) {
            Some(next) => {
                slot.generation = next;
                if let Ok(index) = u32::try_from(index) {
                    self.free.push(index);
                }
            }
            // Retired: its words are never given again.
            None => slot.generation = 0,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn next_gives_the_right_to_its_caller_and_reply_checks_the_holder() {
        // Rule 4.1 and 4.4: the caller holds it, another task does not.
        let rights: Rights<&str> = Rights::new();
        let a = rights.deliver(7, "a").unwrap();
        assert!(rights.holds(7, a));
        assert!(!rights.holds(8, a));
        assert_eq!(rights.take(8, a), Err(NotHeld));
        assert_eq!(rights.take(7, a), Ok("a"));
        // Taken out: a second take while the answer is under way is refused.
        assert_eq!(rights.take(7, a), Err(NotHeld));
        rights.release(7, a);
        // A successful reply releases it: a second one is `NotHeld` (single use).
        assert_eq!(rights.take(7, a), Err(NotHeld));
        assert!(!rights.holds(7, a));
        assert_eq!(rights.live(), 0);
    }

    #[test]
    fn a_bad_reply_keeps_the_right() {
        // `spec/31 §5` rule 1: nothing is sent, and the right stays held.
        let rights: Rights<&str> = Rights::new();
        let a = rights.deliver(1, "a").unwrap();
        let x = rights.take(1, a).unwrap();
        rights.restore(1, a, x);
        assert!(rights.holds(1, a));
        assert_eq!(rights.take(1, a), Ok("a"));
    }

    #[test]
    fn a_word_is_never_given_to_two_exchanges() {
        // A freed slot comes back with its next generation: the old word names nothing.
        let rights: Rights<u32> = Rights::new();
        let a = rights.deliver(1, 1).unwrap();
        rights.take(1, a).unwrap();
        rights.release(1, a);
        let b = rights.deliver(1, 2).unwrap();
        assert_ne!(a, b);
        assert_eq!(a >> 32, b >> 32);
        assert_eq!(rights.take(1, a), Err(NotHeld));
        assert_eq!(rights.take(1, b), Ok(2));
    }

    #[test]
    fn a_slot_whose_generations_are_spent_is_retired() {
        let rights: Rights<u32> = Rights::new();
        let a = rights.deliver(1, 1).unwrap();
        rights.take(1, a).unwrap();
        rights.release(1, a);
        // Slot 0 is free with generation 2; give it its last generation, then spend it.
        {
            let mut table = rights.table().unwrap();
            table.slots[0].generation = u32::MAX;
        }
        let last = rights.deliver(1, 9).unwrap();
        assert_eq!(last, word(0, u32::MAX));
        rights.take(1, last).unwrap();
        rights.release(1, last);
        // Retired: the next delivery takes a new slot, and the old words name nothing.
        let b = rights.deliver(1, 2).unwrap();
        assert_eq!(b >> 32, 1);
        assert_eq!(rights.take(1, a), Err(NotHeld));
        assert_eq!(rights.take(1, last), Err(NotHeld));
        assert!(rights.table().unwrap().free.is_empty());
    }

    #[test]
    fn start_moves_only_what_the_caller_holds_and_keeps_the_order() {
        // Rule 4.2: the right moves, the caller no longer holds it, and the started task's list
        // is in the order the moves came.
        let rights: Rights<&str> = Rights::new();
        let a = rights.deliver(0, "a").unwrap();
        let b = rights.deliver(0, "b").unwrap();
        let c = rights.deliver(5, "c").unwrap();
        assert!(rights.transfer(0, 1, b));
        assert!(rights.transfer(0, 1, a));
        // Not held by 0: it does not move.
        assert!(!rights.transfer(0, 1, c));
        assert!(!rights.transfer(0, 1, a));
        assert_eq!(rights.held_by(0), Vec::<Word>::new());
        assert_eq!(rights.held_by(1), vec![b, a]);
        assert_eq!(rights.take(0, a), Err(NotHeld));
        assert!(rights.holds(5, c));
    }

    #[test]
    fn the_end_of_a_task_takes_its_rights_once_in_order() {
        // Rule 4.5: in the order received, each once; an answered one is not among them.
        let rights: Rights<&str> = Rights::new();
        let a = rights.deliver(3, "a").unwrap();
        let b = rights.deliver(3, "b").unwrap();
        let c = rights.deliver(3, "c").unwrap();
        rights.take(3, b).unwrap();
        rights.release(3, b);
        assert_eq!(rights.end(3), vec![(a, "a"), (c, "c")]);
        // Exactly once.
        assert_eq!(rights.end(3), Vec::new());
        assert_eq!(rights.live(), 0);
        assert_eq!(rights.take(3, a), Err(NotHeld));
    }

    #[test]
    fn an_exchange_out_for_its_answer_is_not_auto_answered() {
        let rights: Rights<&str> = Rights::new();
        let a = rights.deliver(3, "a").unwrap();
        let _out = rights.take(3, a).unwrap();
        assert_eq!(rights.end(3), Vec::new());
        assert_eq!(rights.live(), 0);
    }

    #[test]
    fn compaction_keeps_the_order_and_the_positions() {
        // Many removals from the front: the list compacts, and every entry still finds itself.
        let rights: Rights<usize> = Rights::new();
        let words: Vec<Word> = (0..1000).map(|i| rights.deliver(1, i).unwrap()).collect();
        for &w in words.iter().step_by(2) {
            rights.take(1, w).unwrap();
            rights.release(1, w);
        }
        for &w in words.iter().skip(1).step_by(4) {
            assert!(rights.transfer(1, 2, w));
        }
        let left: Vec<Word> = words.iter().skip(3).step_by(4).copied().collect();
        assert_eq!(rights.held_by(1), left);
        let ended: Vec<Word> = rights.end(1).into_iter().map(|(w, _)| w).collect();
        assert_eq!(ended, left);
        let moved: Vec<Word> = words.iter().skip(1).step_by(4).copied().collect();
        assert_eq!(rights.held_by(2), moved);
    }

    #[test]
    fn holding_moving_and_releasing_are_constant_time() {
        // `spec/31 §1` rule 9: O(1) each, amortized. A doubling of the rights held must not
        // double the time per right more than a constant would.
        fn work(n: usize) -> std::time::Duration {
            let rights: Rights<usize> = Rights::new();
            let start = std::time::Instant::now();
            let words: Vec<Word> = (0..n).map(|i| rights.deliver(0, i).unwrap()).collect();
            for &w in &words {
                assert!(rights.transfer(0, 1, w));
            }
            for &w in words.iter().step_by(2) {
                rights.take(1, w).unwrap();
                rights.release(1, w);
            }
            assert_eq!(rights.end(1).len(), n / 2);
            start.elapsed()
        }
        let small = (0..3).map(|_| work(20_000)).min().unwrap();
        let large = (0..3).map(|_| work(320_000)).min().unwrap();
        // 16 times the rights: linear is 16 times the time, quadratic 256 times.
        assert!(
            large < small * 64,
            "{small:?} for 20,000 rights, {large:?} for 320,000"
        );
    }

    #[test]
    fn a_poisoned_table_holds_no_right() {
        let rights = std::sync::Arc::new(Rights::<u32>::new());
        let a = rights.deliver(1, 1).unwrap();
        let poisoner = std::sync::Arc::clone(&rights);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.table.lock();
            #[allow(clippy::panic)]
            {
                panic!("a test poisons the table");
            }
        })
        .join();
        assert_eq!(rights.take(1, a), Err(NotHeld));
        assert!(!rights.holds(1, a));
        assert_eq!(rights.deliver(1, 2), Err(2));
        assert!(rights.end(1).is_empty());
        assert!(!rights.transfer(1, 2, a));
    }
}
