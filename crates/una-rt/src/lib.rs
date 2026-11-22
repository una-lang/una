//! `una-rt`: the runtime library that every native Una executable links (`spec/16`,
//! ADR 0055).
//!
//! The executable's `main` calls [`ffi`]'s `una_rt_start`, which reads the program's embedded
//! description and the command line, checks `main`'s capabilities against the grants
//! (U3012), runs the compiled `main`, and writes the trace when `--trace` asks for it. The
//! compiled code keeps its values in [`values`], performs host operations and records events
//! through [`host`], and reports a failure of `main` (U6001) with [`render`].
//!
//! Host capabilities have the interpreter's semantics and grants, and traces use its JSON
//! form ([`una_interp::trace_json`]), so a native run and `una run` agree (`spec/16 §5`).
//!
//! Only [`ffi`] may use `unsafe`, at the C ABI boundary (ADR 0058). This runtime is the
//! process, so unlike the library crates it writes to stdout and stderr.
#![deny(missing_docs)]

use std::alloc::System;

use cap::Cap;

pub mod deadline;
pub mod decimal;
mod driver;
#[allow(unsafe_code)]
pub mod ffi;
pub mod generic;
pub mod host;
mod interrupt;
mod json;
mod reactor;
pub mod render;
pub mod runtime;
mod sched;
pub mod serve;
mod sockets;
mod started;
pub mod tasks;
mod timer;
pub mod values;
pub use una_http::wheel;

/// The allocator of the executable, which counts the bytes it holds and fails past a ceiling,
/// for the memory limit of runs (ADR 0061). The `cap` crate wraps the system allocator, so the
/// runtime writes no `unsafe` for it.
#[cfg(not(debug_assertions))]
#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

/// The allocator of a debug build: the same `Cap<System>`, wrapped to keep the high-water mark
/// of what the process holds and each thread's allocations (ADR 0113, I3).
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: ffi::Peak = ffi::Peak::new(Cap::new(System, usize::MAX));

/// The `Cap<System>` that counts and limits the process's heap, in either build.
fn cap() -> &'static Cap<System> {
    #[cfg(debug_assertions)]
    {
        ALLOCATOR.inner()
    }
    #[cfg(not(debug_assertions))]
    {
        &ALLOCATOR
    }
}

/// The sequence lock that orders every change of the allocator's limit with every read of the
/// bytes it holds (#2151). `cap` 0.1.2 derives the bytes held as `limit - remaining` and raises
/// its limit in two writes, the limit first and `remaining` after: a read between them returns
/// close to `usize::MAX`, a count a budget check then reports as U6003 at a site where nothing
/// was over.
static LIMIT: LimitLock = LimitLock::new();

/// The bytes the process holds on the heap. Never read across a change of the limit
/// ([`LIMIT`]). O(1): it retries only while a [`ceiling`] runs on another thread.
pub(crate) fn held() -> usize {
    LIMIT.read(|| cap().allocated())
}

/// The bytes the process holds on the heap, or `None` while a change of the limit is in
/// progress, for the allocator itself, which never waits ([`ffi::Peak`]).
#[cfg(debug_assertions)]
pub(crate) fn held_now() -> Option<usize> {
    LIMIT.try_read(|| cap().allocated())
}

/// A sequence lock (#2151): a writer makes the sequence odd while it changes the data and even
/// again after, and a reader keeps a read only when the sequence was even and the same before
/// and after it. Writers take turns on the sequence itself; readers never write, so a read takes
/// no lock and costs two loads and a fence, and a writer never waits for a reader. A read that
/// meets a write retries until the write is done, however long that takes: correctness never
/// depends on the write being short.
pub(crate) struct LimitLock {
    seq: std::sync::atomic::AtomicUsize,
}

impl LimitLock {
    /// A lock with no write in progress.
    pub(crate) const fn new() -> LimitLock {
        LimitLock {
            seq: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Runs `change` as the only writer, with the sequence odd for its whole duration.
    pub(crate) fn write<T>(&self, change: impl FnOnce() -> T) -> T {
        use std::sync::atomic::{Ordering, fence};
        let mut seq = self.seq.load(Ordering::Relaxed);
        loop {
            if seq & 1 == 1 {
                std::hint::spin_loop();
                seq = self.seq.load(Ordering::Relaxed);
                continue;
            }
            // `Acquire`: the changes of the writer before are seen by this one.
            match self.seq.compare_exchange_weak(
                seq,
                seq.wrapping_add(1),
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(now) => seq = now,
            }
        }
        // The odd sequence is ordered before every write of `change`: a reader that sees one
        // of them, and then fences, sees the odd sequence or a later one.
        fence(Ordering::Release);
        let result = change();
        // `Release`: a reader that sees the even sequence sees every write of `change`.
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
        result
    }

    /// Reads with `read`, again until no write ran during the read.
    pub(crate) fn read<T>(&self, read: impl Fn() -> T) -> T {
        loop {
            if let Some(value) = self.try_read(&read) {
                return value;
            }
            std::hint::spin_loop();
        }
    }

    /// Reads with `read` once, and keeps the value only when no write ran during the read.
    pub(crate) fn try_read<T>(&self, read: impl FnOnce() -> T) -> Option<T> {
        use std::sync::atomic::{Ordering, fence};
        let before = self.seq.load(Ordering::Acquire);
        if before & 1 == 1 {
            return None;
        }
        let value = read();
        // Orders the loads of `read` before the second load of the sequence: a write `read`
        // saw makes the sequence read below differ from `before`.
        fence(Ordering::Acquire);
        (self.seq.load(Ordering::Relaxed) == before).then_some(value)
    }
}

/// The most bytes the process held at any point so far, in a debug build (ADR 0113, I3).
#[cfg(debug_assertions)]
pub(crate) fn peak() -> usize {
    ALLOCATOR.peak()
}

/// Sets the ceiling past which an allocation fails; `usize::MAX` lifts it. A ceiling below what
/// is held already is refused, and the next check reports the run.
pub(crate) fn ceiling(bytes: usize) {
    LIMIT.write(|| {
        let _ = cap().set_limit(bytes);
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::LimitLock;

    /// The limit and the bytes left of an allocator that, as `cap` 0.1.2 does, derives the
    /// bytes held as `limit - remaining` and raises its limit in two writes, the limit first.
    struct TwoWrites {
        limit: AtomicUsize,
        remaining: AtomicUsize,
    }

    impl TwoWrites {
        /// An allocator with `limit` that holds `held` bytes.
        fn new(limit: usize, held: usize) -> TwoWrites {
            TwoWrites {
                limit: AtomicUsize::new(limit),
                remaining: AtomicUsize::new(limit - held),
            }
        }

        /// `cap::Cap::allocated`: retried while the limit read twice differs, and while the
        /// bytes left exceed it.
        fn allocated(&self) -> usize {
            loop {
                let before = self.limit.load(Ordering::SeqCst);
                let remaining = self.remaining.load(Ordering::SeqCst);
                let limit = self.limit.load(Ordering::SeqCst);
                if before == limit && limit >= remaining {
                    return limit - remaining;
                }
            }
        }

        /// `cap::Cap::set_limit` raising the limit, with `between` run between its two writes.
        fn raise(&self, limit: usize, between: impl FnOnce()) {
            let old = self.limit.swap(limit, Ordering::Relaxed);
            between();
            self.remaining.fetch_add(limit - old, Ordering::Relaxed);
        }
    }

    #[test]
    fn a_read_of_the_bytes_held_never_returns_a_count_torn_by_a_raise_of_the_limit() {
        // #2151, reproduced with no timing: the raise stops between its two writes until the
        // reads below are done. Read then, the allocator's own count is torn, close to
        // `usize::MAX`; read under the lock, it is retried until the raise is done, and exact.
        let (held, old, new) = (44 << 20, 4 << 30, usize::MAX);
        let pair = TwoWrites::new(old, held);
        let lock = LimitLock::new();
        let (inside, finish) = (Barrier::new(2), Barrier::new(2));
        let read_done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                lock.write(|| {
                    pair.raise(new, || {
                        inside.wait();
                        finish.wait();
                    });
                });
            });
            inside.wait();
            // The defect: between the two writes the count is the new limit less the old
            // headroom.
            let torn = pair.allocated();
            assert_eq!(torn, new - (old - held));
            assert!(torn > usize::MAX / 2);
            // Under the lock no read completes while the raise is in progress.
            assert_eq!(lock.try_read(|| pair.allocated()), None);
            let reader = scope.spawn(|| {
                let value = lock.read(|| pair.allocated());
                read_done.store(true, Ordering::SeqCst);
                value
            });
            assert!(!read_done.load(Ordering::SeqCst));
            finish.wait();
            assert_eq!(reader.join().unwrap(), held);
        });
        assert_eq!(lock.read(|| pair.allocated()), held);
    }

    #[test]
    fn a_read_that_a_change_of_the_limit_overlaps_is_retried() {
        // A raise that starts after a read began and ends before it ends: the read saw the old
        // limit and the new bytes left, so its value is thrown away and it is read again.
        let (held, old) = (1 << 20, 1 << 30);
        let pair = TwoWrites::new(old, held);
        let lock = LimitLock::new();
        let attempts = AtomicUsize::new(0);
        let value = lock.read(|| {
            let limit = pair.limit.load(Ordering::SeqCst);
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                std::thread::scope(|scope| {
                    scope.spawn(|| lock.write(|| pair.raise(usize::MAX, || {})));
                });
            }
            let remaining = pair.remaining.load(Ordering::SeqCst);
            limit.wrapping_sub(remaining)
        });
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(value, held);
    }

    #[test]
    fn writers_of_the_limit_take_turns() {
        // Writers never overlap: each sees the count of the writers before it unchanged
        // during its own write.
        let lock = LimitLock::new();
        let (count, overlaps) = (AtomicUsize::new(0), AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..10_000 {
                        lock.write(|| {
                            let seen = count.fetch_add(1, Ordering::SeqCst);
                            std::hint::spin_loop();
                            if count.load(Ordering::SeqCst) != seen + 1 {
                                overlaps.fetch_add(1, Ordering::SeqCst);
                            }
                        });
                    }
                });
            }
        });
        assert_eq!(overlaps.load(Ordering::SeqCst), 0);
        assert_eq!(count.load(Ordering::SeqCst), 40_000);
        assert_eq!(lock.seq.load(Ordering::SeqCst), 80_000);
    }

    #[test]
    fn reads_of_a_real_allocator_raised_and_lowered_meanwhile_are_never_torn() {
        // #2151 on `cap` itself: a `Cap` of this test, which allocates nothing, so it holds
        // nothing whatever its limit. One thread lowers and raises its limit in turn; the
        // others read the bytes it holds under the lock, which must be 0 every time. Without
        // the lock, the reads made during a raise return close to 2^62.
        let allocator = cap::Cap::new(std::alloc::System, usize::MAX);
        let lock = LimitLock::new();
        let done = AtomicBool::new(false);
        let low = usize::MAX - (1 << 62);
        let torn = std::thread::scope(|scope| {
            let readers: Vec<_> = (0..3)
                .map(|_| {
                    scope.spawn(|| {
                        let (mut reads, mut torn) = (0usize, 0usize);
                        while reads < 100_000 || !done.load(Ordering::SeqCst) {
                            torn += usize::from(lock.read(|| allocator.allocated()) != 0);
                            reads += 1;
                        }
                        torn
                    })
                })
                .collect();
            for round in 0..200_000usize {
                let limit = if round % 2 == 0 { low } else { usize::MAX };
                lock.write(|| allocator.set_limit(limit)).unwrap();
            }
            done.store(true, Ordering::SeqCst);
            readers
                .into_iter()
                .map(|reader| reader.join().unwrap())
                .sum::<usize>()
        });
        assert_eq!(torn, 0);
    }
}
