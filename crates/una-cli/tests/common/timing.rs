//! Scaling assertions that scheduler noise cannot fail (#341, #1197), shared by every scaling
//! test of the workspace (`una-cli`, `una-check`, `una-link` include this file by path).
//!
//! Counted first: where the algorithm has a deterministic measure of its work (a test-only
//! counter of `una_core::label_counters` behind the feature `counters`, the debug runtime's
//! copies, a count of bytes or queries), [`assert_count_growth`] bounds that count, which no
//! load on the machine moves. Timed as well, or alone where no counter exists:
//! [`assert_growth`] measures each size once, and only when a growth bound is missed measures
//! every size again, in interleaved rounds, up to [`ROUNDS`] in all, keeping each size's
//! shortest time and judging again after each round. In those rounds a smaller size is run as
//! many times as make up one run of the largest and its time averaged, so that every
//! measurement spans about the same wall time and a loaded machine slows each about as much:
//! a short run alone would more often fit between two preemptions than a long one. A burst of
//! load that slows one size is measured past; a quadratic algorithm is not, since the shortest
//! of any number of runs, and the mean of any runs, is still at least its real cost, so a retry
//! never hides one.

// Each test file uses its own subset of these helpers.
#![allow(dead_code)]

/// The least time, in seconds, that a measurement past the fixed cost counts as: below it, a
/// ratio would measure the clock's noise rather than the work.
pub const FLOOR: f64 = 0.02;

/// What `run` returns, and the seconds it took.
pub fn timed<T>(run: impl FnOnce() -> T) -> (T, f64) {
    let started = std::time::Instant::now();
    let value = run();
    (value, started.elapsed().as_secs_f64())
}

/// The least of three calls of `measure`, in seconds: for a fixed cost, measured on a tiny size,
/// so cheap to take three times.
pub fn best_of_three(mut measure: impl FnMut() -> f64) -> f64 {
    (0..3).map(|_| measure()).fold(f64::INFINITY, f64::min)
}

/// The most rounds of measurement [`assert_growth`] takes, the first included.
pub const ROUNDS: usize = 5;

/// Asserts that `measure(size)`, the seconds a run of `size` takes, grows past the fixed cost
/// `fixed` within `bound` from each size to the next: measured once, and on a miss again in
/// interleaved rounds, up to [`ROUNDS`] in all, each size run as often as makes up one run of
/// the largest and averaged, keeping each size's best (#341, #1197). The
/// growth from one size to the next is the ratio of their times less `fixed`, each at least
/// `floor` seconds. `what` names the case in the messages.
pub fn assert_growth(
    what: &str,
    sizes: &[usize],
    fixed: f64,
    floor: f64,
    bound: f64,
    mut measure: impl FnMut(usize) -> f64,
) {
    let mut times: Vec<f64> = sizes.iter().map(|&size| measure(size)).collect();
    let first = times.clone();
    let mut rounds = 1;
    let largest = sizes.iter().copied().max().unwrap_or(1).max(1);
    while rounds < ROUNDS && growths(&times, fixed, floor).iter().any(|&g| g >= bound) {
        for (time, &size) in times.iter_mut().zip(sizes) {
            // As many runs of a smaller size as make up one run of the largest, averaged: every
            // measurement then spans about as much wall time, so a busy machine slows each
            // about as much, rather than the longest the most (#1197).
            let runs = largest.div_ceil(size.max(1));
            let total: f64 = (0..runs).map(|_| measure(size)).sum();
            *time = time.min(total / runs as f64);
        }
        rounds += 1;
    }
    for (step, growth) in growths(&times, fixed, floor).into_iter().enumerate() {
        let ratio = sizes[step + 1] as f64 / sizes[step] as f64;
        assert!(
            growth < bound,
            "{what}: {ratio}x the size took {growth:.1}x as long, past the fixed cost of \
             {fixed:.4}s, against a bound of {bound}x (sizes {sizes:?}, best of {rounds} rounds \
             {times:?}, first times {first:?})"
        );
    }
}

/// Asserts that `counts`, a deterministic measure of the work done at each of `sizes` (a step
/// counter, a number of queries or copies), grows within `bound` times the growth of the size
/// from each size to the next, and that the counter counts at all. Linear work at a doubling
/// size takes a `bound` a little over 1, such as 1.1, for the steps that do not grow; a
/// quadratic one doubles it. `what` names the case in the messages.
pub fn assert_count_growth(what: &str, sizes: &[usize], counts: &[usize], bound: f64) {
    assert_eq!(sizes.len(), counts.len(), "{what}: one count per size");
    assert!(
        counts.first().is_some_and(|&count| count > 0),
        "{what}: the counter counts nothing (is the feature `counters` on?): {counts:?}"
    );
    for (pair, sizes_pair) in counts.windows(2).zip(sizes.windows(2)) {
        let ratio = sizes_pair[1] as f64 / sizes_pair[0] as f64;
        assert!(
            pair[1] as f64 <= bound * ratio * pair[0] as f64,
            "{what}: {ratio}x the size, {} to {}, took {} steps against {}, past {bound} times \
             the growth of the size (sizes {sizes:?}, counts {counts:?})",
            sizes_pair[0],
            sizes_pair[1],
            pair[1],
            pair[0]
        );
    }
}

/// The growth from each time to the next, past `fixed`, each time at least `floor`.
fn growths(times: &[f64], fixed: f64, floor: f64) -> Vec<f64> {
    times
        .windows(2)
        .map(|pair| (pair[1] - fixed).max(floor) / (pair[0] - fixed).max(floor))
        .collect()
}
