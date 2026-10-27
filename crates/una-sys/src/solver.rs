//! The solver child's confinement record (ADR 0124 rules 2 and 7), behind the feature `solver`,
//! which only `una-cli` enables: the confinement code sets it once every step has succeeded,
//! and `una-z3` refuses to call Z3's front end unless it is set, so no path of `una` reaches
//! the engine unconfined. The confinement itself, the limits, the spawn attributes and the
//! footprint reading come in their own slice of #1206 and live in this module.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set once this process has confined itself; never cleared.
static CONFINED: AtomicBool = AtomicBool::new(false);

/// Records that this process confined itself: every step of rule 7 succeeded. Called by the
/// confinement code only, as its last step.
pub fn record_confined() {
    CONFINED.store(true, Ordering::Release);
}

/// Whether this process recorded a successful confinement.
pub fn confined() -> bool {
    CONFINED.load(Ordering::Acquire)
}
