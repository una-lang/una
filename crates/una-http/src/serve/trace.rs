//! The key of a run's reduced trace form (ADR 0123 rule 1, `spec/31 §1` rule 7): the events of
//! `next` and `reply` of the host's `HttpServer` write each body, and each value of a query
//! parameter, as its length and an HMAC-SHA-256 of it under this key, so that two runs of one
//! script compare while a guessable body cannot be found from a trace.
//!
//! The key is drawn from the host's random source when the run starts, and is all zeros under a
//! script of connections (`--serve-script`, and the runs of `tests/serve_runs/`). It is never
//! written anywhere: it has no `Debug` that shows it.

use una_hash::{KeyError, SipKey};

/// The 32 bytes of a run's trace key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TraceKey([u8; 32]);

// No `Debug` with the key: a key never appears in a message, a trace or a report.
impl core::fmt::Debug for TraceKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TraceKey(..)")
    }
}

impl TraceKey {
    /// The key of a run under a script of connections: all zeros (ADR 0123 rule 1).
    pub const ZERO: TraceKey = TraceKey([0; 32]);

    /// A fresh key from the host's random source, for a run that serves live peers. When the
    /// host gives no random bytes the run cannot start: no fixed or guessable key ever replaces
    /// it, as for the placement key of the serve threads.
    pub fn from_host() -> Result<TraceKey, KeyError> {
        let (a, b) = (SipKey::from_host()?, SipKey::from_host()?);
        let mut bytes = [0u8; 32];
        for (to, from) in bytes.iter_mut().zip(
            [a.k0, a.k1, b.k0, b.k1]
                .into_iter()
                .flat_map(u64::to_le_bytes),
        ) {
            *to = from;
        }
        Ok(TraceKey(bytes))
    }

    /// Its bytes, for the HMAC of the reduced form only.
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_key_from_the_host_is_not_the_zero_key_and_shows_nothing() {
        let a = TraceKey::from_host().unwrap();
        let b = TraceKey::from_host().unwrap();
        assert_ne!(a, TraceKey::ZERO);
        assert_ne!(a, b);
        assert_eq!(format!("{a:?}"), "TraceKey(..)");
    }
}
