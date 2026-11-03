//! Scaling assertions that scheduler noise cannot fail (#341, #1197), shared by every scaling
//! test of the workspace (`una-cli`, `una-check`, `una-link` and `una-project` include this file
//! by path).
//!
//! Counted first: where the algorithm has a deterministic measure of its work (a test-only
//! counter of `una_core::label_counters` behind the feature `counters`, the debug runtime's
//! copies, a count of bytes or queries), [`assert_count_growth`] bounds that count, which no
//! load on the machine moves. Timed as well, or alone where no counter exists:
//! [`assert_growth`] measures each size once, and only when a growth bound is missed measures
//! every size again, in interleaved rounds, up to [`ROUNDS`] in all, keeping each size's
//! shortest time and judging again after each round. In those rounds a smaller size is run
//! about as many times as its measured time fits into the largest size's, at most
//! [`REPEATS`] times, so that each size gets about the same wall time of measurement and the
//! retry costs about one run of the largest size per size and round, whatever the fixed cost.
//!
//! Every time kept is the shortest of all runs of its size, never a mean. A run is never
//! shorter than its real cost, so each kept time is at least that cost, and a retry can only
//! bring it closer. The judged quantity is a ratio: a smaller size kept too high (its
//! denominator) would lower it. The smaller size, though, is run at least as often as the
//! larger in every round, so its shortest time is the closer to its real cost, and the
//! retries lean towards a higher ratio, that is towards failing. What a retry does hide is a
//! burst of load on the larger size, which is what it is for; it cannot turn a cost that grows
//! past the bound in every run into a pass.

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

/// The most runs of one size in a retry round of [`assert_growth`]: a bound on the cost of a
/// retry where a size's time is mostly a fixed cost (#1197).
pub const REPEATS: usize = 16;

/// Asserts that `measure(size)`, the seconds a run of `size` takes, grows past the fixed cost
/// `fixed` within `bound` from each size to the next: measured once, and on a miss again in
/// interleaved rounds, up to [`ROUNDS`] in all, each size run about as often as its time fits
/// into the largest size's (at most [`REPEATS`] times), keeping each size's shortest run (#341,
/// #1197). The growth from one size to the next is the ratio of their times less `fixed`, each
/// at least `floor` seconds. `what` names the case in the messages.
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
    while rounds < ROUNDS && growths(&times, fixed, floor).iter().any(|&g| g >= bound) {
        // The repeats follow the measured times, the fixed cost included, rather than the sizes:
        // a size whose run is mostly fixed cost is run about as often as the largest (#1197).
        let longest = times.iter().copied().fold(0.0, f64::max);
        let repeats: Vec<usize> = times.iter().map(|&time| repeats(longest, time)).collect();
        for ((time, &size), &runs) in times.iter_mut().zip(sizes).zip(&repeats) {
            // The shortest of the runs, never their mean: see the module documentation.
            for _ in 0..runs {
                *time = time.min(measure(size));
            }
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

/// How often a size measured at `time` seconds is run in a retry round: as often as `time`
/// fits into `longest`, the time of the longest size, at least once and at most [`REPEATS`].
fn repeats(longest: f64, time: f64) -> usize {
    // Counted up rather than divided and rounded, which would take the platform's `ceil`.
    let mut runs = 1;
    while runs < REPEATS && (runs as f64) * time < longest {
        runs += 1;
    }
    runs
}

/// The growth from each time to the next, past `fixed`, each time at least `floor`.
fn growths(times: &[f64], fixed: f64, floor: f64) -> Vec<f64> {
    times
        .windows(2)
        .map(|pair| (pair[1] - fixed).max(floor) / (pair[0] - fixed).max(floor))
        .collect()
}
