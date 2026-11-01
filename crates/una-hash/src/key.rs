//! The process key and the key of each table (ADR 0119 rule 9).
//!
//! The process key is 128 bits from the operating system's cryptographic random source, read
//! through the `getrandom` crate once a run can make a hash table, before `main` and before any
//! effect. When the host gives no random bytes the run aborts with U6009: no fixed or guessable
//! key ever replaces it. The process key hashes nothing but the table keys: table n's key is the
//! two 64-bit results of SipHash-2-4, under the process key, of x1, n as 8 bytes little-endian,
//! and of x2, the same 8 bytes followed by the byte 0x01.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::siphash::{SipKey, sip24};

/// Why no key could be had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// The host gave no random bytes for the process key: the run aborts with U6009.
    NoRandomBytes,
    /// The table counter reached 2^64 - 1: the run aborts with U6003, as an exhausted arena
    /// counter does (ADR 0111 rule 3).
    CounterExhausted,
}

/// The process key: 128 bits of the host's random source. It hashes only table keys.
pub struct ProcessKey {
    key: SipKey,
    /// The number of tables made, the input of the next table key. Never observable.
    tables: AtomicU64,
}

// No `Debug` with the key: a key never appears in a message, a trace or a report.
impl core::fmt::Debug for ProcessKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ProcessKey(..)")
    }
}

impl ProcessKey {
    /// The process key from the host's random source: `getrandom` (on Linux the `getrandom`
    /// system call with no flags, which blocks until the kernel's pool is seeded, with the
    /// crate's own fallback; on macOS `getentropy`; on Windows `ProcessPrng`).
    pub fn from_host() -> Result<ProcessKey, KeyError> {
        #[cfg(feature = "test-hooks")]
        if let Some(key) = crate::hooks::process_key() {
            return key;
        }
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| KeyError::NoRandomBytes)?;
        Ok(ProcessKey::from_key(SipKey::from_bytes(bytes)))
    }

    /// A process key with the given bits, for the test builds' fixed key.
    #[cfg(feature = "test-hooks")]
    pub(crate) fn from_key(key: SipKey) -> ProcessKey {
        ProcessKey {
            key,
            tables: AtomicU64::new(crate::hooks::first_table()),
        }
    }

    #[cfg(not(feature = "test-hooks"))]
    fn from_key(key: SipKey) -> ProcessKey {
        ProcessKey {
            key,
            tables: AtomicU64::new(0),
        }
    }

    /// The key of the next table, from the counter's next value. A copy of a table, a `with` or
    /// `without` on a shared table, and the copy at a task's fork or join keep their table's
    /// key and call this only for a new table.
    pub fn next_table_key(&self) -> Result<SipKey, KeyError> {
        let n = self
            .tables
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| KeyError::CounterExhausted)?;
        Ok(table_key(self.key, n))
    }
}

impl SipKey {
    /// A fresh 128-bit key from the host's random source, for a use that is not a table's
    /// (the serve threads' placement of connections, ADR 0123 rule 19). When the host gives no
    /// random bytes the run aborts with U6009, as for the process key: no fixed or guessable key
    /// ever replaces it. The key has no `Debug` that shows it.
    pub fn from_host() -> Result<SipKey, KeyError> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| KeyError::NoRandomBytes)?;
        Ok(SipKey::from_bytes(bytes))
    }
}

/// Table n's key under the process key `process`.
pub fn table_key(process: SipKey, n: u64) -> SipKey {
    let x1 = n.to_le_bytes();
    let mut x2 = [0u8; 9];
    for (to, from) in x2.iter_mut().zip(x1) {
        *to = from;
    }
    if let Some(last) = x2.last_mut() {
        *last = 0x01;
    }
    SipKey {
        k0: sip24(process, &x1),
        k1: sip24(process, &x2),
    }
}
