//! The processor time of the children a scaling test runs (#1656), the measure that
//! `timing::assert_growth` judges for the tests whose claim is about the time a run of `una` or of a built executable takes: a child's user and system time stay what
//! its work costs when other tests run beside it, where its elapsed time does not, so these tests
//! need no slot of their own (`.config/nextest.toml`, the names that hold `cpu_time`).
//!
//! The run is measured as the growth of `getrusage(RUSAGE_CHILDREN)` across it
//! (`una_sys::solver::usage`), which counts every child the test process waited for; a test is
//! one process under nextest, and the run's own children (a solver, a linker) are counted with it.

/// What `run` returns, and the processor seconds its waited-for children took: `run` must wait
/// for every child it starts, as `Command::output` and `Command::status` do.
pub fn cpu_timed<T>(run: impl FnOnce() -> T) -> (T, f64) {
    let before = children_seconds();
    let value = run();
    (value, children_seconds() - before)
}

/// The processor seconds of this process's waited-for children so far.
fn children_seconds() -> f64 {
    una_sys::solver::usage::children_cpu_time()
        .unwrap()
        .as_secs_f64()
}
