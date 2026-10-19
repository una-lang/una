//! Attention words (ADR 0116 rule 18): one 64-bit atomic word per thread that runs Una code, in
//! a static table, which a stop request, a deadline, the end of a time slice or a cancellation
//! increments, and which each check point loads once and compares with the value it last saw.
//!
//! No `unsafe` here: the table is a static array of atomics, and a signal handler only adds to
//! it, which is async-signal-safe because the atomics are lock-free.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// The entries of the table. A thread that finds it full shares entry 0.
pub const ENTRIES: usize = 4096;

static WORDS: [AtomicU64; ENTRIES] = [const { AtomicU64::new(0) }; ENTRIES];

/// The entries handed out so far: entry 0 is shared, so the next one starts at 1. Never more
/// than [`ENTRIES`] are read.
static NEXT: AtomicUsize = AtomicUsize::new(1);

/// The host stop requests that have arrived (ADR 0116 rule 14): sticky, never reset.
static STOP_REQUESTS: AtomicU32 = AtomicU32::new(0);

/// A thread's attention word: an index into the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word(usize);

impl Word {
    /// The word's current value, a relaxed load: the one load of a check point's fast path.
    pub fn load(self) -> u64 {
        WORDS
            .get(self.0)
            .map_or(0, |word| word.load(Ordering::Relaxed))
    }

    /// Raises the word: its thread's next check point takes the slow path.
    pub fn raise(self) {
        if let Some(word) = WORDS.get(self.0) {
            word.fetch_add(1, Ordering::Release);
        }
    }
}

/// A word for the calling thread: a new entry, or entry 0 when the table is full. Entries are
/// never given back, so a word stays valid for the life of the process. O(1).
pub fn register() -> Word {
    let index = NEXT.fetch_add(1, Ordering::Relaxed);
    if index < ENTRIES {
        Word(index)
    } else {
        // Keep the counter from wrapping around into valid indices again.
        NEXT.store(ENTRIES, Ordering::Relaxed);
        Word(0)
    }
}

/// Raises every word handed out, and entry 0: a loop of atomic adds over the registered
/// entries, async-signal-safe. O(threads registered).
pub fn raise_all() {
    let handed = NEXT.load(Ordering::Relaxed).min(ENTRIES);
    for word in WORDS.iter().take(handed) {
        word.fetch_add(1, Ordering::Release);
    }
}

/// Records a host stop request and raises every word; returns how many requests have arrived,
/// this one included. Async-signal-safe: atomics only.
pub fn request_stop() -> u32 {
    let count = STOP_REQUESTS
        .fetch_add(1, Ordering::AcqRel)
        .saturating_add(1);
    raise_all();
    count
}

/// How many host stop requests have arrived. A request is sticky: once one has arrived, this
/// stays at least 1 for the rest of the process.
pub fn stop_requests() -> u32 {
    STOP_REQUESTS.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raised_word_changes_and_a_stop_raises_every_word() {
        let a = register();
        let b = register();
        let (a0, b0) = (a.load(), b.load());
        a.raise();
        assert_ne!(a.load(), a0);
        assert_eq!(b.load(), b0);
        let before = stop_requests();
        assert_eq!(request_stop(), before + 1);
        assert_ne!(b.load(), b0);
        assert!(stop_requests() > before);
    }
}
