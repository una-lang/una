//! The toolchain's test builds only (ADR 0119 rule 9): a fixed process key, a simulated missing
//! random source, a counter that starts where a test puts it, the constant-hash mode, in which
//! every key collides, and the `probes` counter of the table's index. Under a random key the
//! probe counter would show a table's collision structure, so it exists only here.
//!
//! No shipped `una` and no shipped `libuna_rt.a` enables this module, and no environment
//! variable, flag or `una.toml` key reaches it: a test sets it through these functions. The
//! release gate fails when `nm` finds [`UNA_HASH_TEST_HOOKS_MARKER`] in a shipped artifact.

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use crate::key::{KeyError, ProcessKey};
use crate::siphash::SipKey;

/// The marker the release gate looks for in the shipped `una` and `libuna_rt.a` (`nm`). The key
/// path reads it, so a build with this module keeps it.
#[used]
pub static UNA_HASH_TEST_HOOKS_MARKER: [u8; 26] = *b"UNA_HASH_TEST_HOOKS_MARKER";

/// The source of the process key: the host, a fixed key, or none (a simulated missing source).
const HOST: u8 = 0;
const FIXED: u8 = 1;
const MISSING: u8 = 2;

static SOURCE: AtomicU8 = AtomicU8::new(HOST);
static FIXED_K0: AtomicU64 = AtomicU64::new(0);
static FIXED_K1: AtomicU64 = AtomicU64::new(0);
static FIRST_TABLE: AtomicU64 = AtomicU64::new(0);
static CONSTANT: AtomicBool = AtomicBool::new(false);
static PROBES: AtomicU64 = AtomicU64::new(0);

/// The process key is `key` from now on.
pub fn fix_key(key: SipKey) {
    FIXED_K0.store(key.k0, Ordering::SeqCst);
    FIXED_K1.store(key.k1, Ordering::SeqCst);
    SOURCE.store(FIXED, Ordering::SeqCst);
}

/// The host's random source gives nothing from now on, as on a host without one.
pub fn simulate_missing_source() {
    SOURCE.store(MISSING, Ordering::SeqCst);
}

/// The process key comes from the host again, and the other hooks are off.
pub fn reset() {
    SOURCE.store(HOST, Ordering::SeqCst);
    FIRST_TABLE.store(0, Ordering::SeqCst);
    CONSTANT.store(false, Ordering::SeqCst);
    PROBES.store(0, Ordering::SeqCst);
}

/// The next process key's table counter starts at `n` (to reach 2^64 - 1 in a test).
pub fn start_tables_at(n: u64) {
    FIRST_TABLE.store(n, Ordering::SeqCst);
}

/// Every key hashes to 0 from now on, or no longer.
pub fn set_constant_hash(on: bool) {
    CONSTANT.store(on, Ordering::SeqCst);
}

/// The slots the tables' lookups have read since the last [`reset`].
pub fn probes() -> u64 {
    PROBES.load(Ordering::SeqCst)
}

pub(crate) fn probe() {
    PROBES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn constant_hash() -> bool {
    CONSTANT.load(Ordering::Relaxed)
}

pub(crate) fn first_table() -> u64 {
    FIRST_TABLE.load(Ordering::SeqCst)
}

/// The process key the hooks give, if they give one: `None` lets the host's source give it.
pub(crate) fn process_key() -> Option<Result<ProcessKey, KeyError>> {
    let _ = core::hint::black_box(&UNA_HASH_TEST_HOOKS_MARKER);
    match SOURCE.load(Ordering::SeqCst) {
        FIXED => Some(Ok(ProcessKey::from_key(SipKey {
            k0: FIXED_K0.load(Ordering::SeqCst),
            k1: FIXED_K1.load(Ordering::SeqCst),
        }))),
        MISSING => Some(Err(KeyError::NoRandomBytes)),
        _ => None,
    }
}
