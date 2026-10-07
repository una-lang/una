//! Timing assertions that scheduler noise cannot fail (#341). A scaling test measures each size
//! once; only when a growth bound is missed does it measure every size twice more and judge the
//! best of the three runs, so a normal run costs what one measurement costs, and a run slowed
//! by a busy machine is measured again before it fails. Where a test has a deterministic
//! counter (the debug runtime's copies, a count of bytes), that counter is the primary check
//! and the timing is the secondary one.

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

/// Asserts that `measure(size)`, the seconds a run of `size` takes, grows past the fixed cost
/// `fixed` within `bound` from each size to the next: measured once, and on a miss twice more,
/// keeping each size's best (#341). The growth from one size to the next is the ratio of their
/// times less `fixed`, each at least `floor` seconds. `what` names the case in the messages.
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
    if growths(&times, fixed, floor).iter().any(|&g| g >= bound) {
        for _ in 0..2 {
            for (time, &size) in times.iter_mut().zip(sizes) {
                *time = time.min(measure(size));
            }
        }
    }
    for (step, growth) in growths(&times, fixed, floor).into_iter().enumerate() {
        let ratio = sizes[step + 1] as f64 / sizes[step] as f64;
        assert!(
            growth < bound,
            "{what}: {ratio}x the size took {growth:.1}x as long, past the fixed cost of \
             {fixed:.4}s, against a bound of {bound}x (sizes {sizes:?}, best of three times \
             {times:?}, first times {first:?})"
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
