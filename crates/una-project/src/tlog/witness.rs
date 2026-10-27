//! Time in witness time (ADR 0122 rule 36.4). No rule uses a publisher's, the registry's or a
//! single witness's clock: with k the witness threshold of the root metadata, every time is an
//! order statistic over k distinct witnesses, so fewer than k witnesses can move none of them
//! in an attacker's favor.
//!
//! Costs: O(c log c) for c cosignatures, to sort the distinct witnesses' times.

use std::collections::BTreeMap;

/// A cosignature seen: which witness, on which checkpoint (by size), at what time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cosigned {
    /// The witness, by its key's name.
    pub witness: String,
    /// The size of the checkpoint it cosigned.
    pub size: u64,
    /// The cosignature's timestamp, in seconds.
    pub time: u64,
}

/// Why witness time cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Fewer than the threshold of distinct witnesses cosigned.
    TooFew {
        /// How many did.
        found: usize,
        /// The threshold.
        needed: usize,
    },
    /// A witness's timestamps decrease as the log grows (U7010 against that witness).
    Decreasing {
        /// The witness.
        witness: String,
    },
}

/// The time an entry was logged, the start of a wait, where an attacker wants an earlier time:
/// for each distinct witness, its earliest cosignature on a checkpoint that includes the entry
/// (of a size above `index`); the entry is logged at the k-th earliest of these.
pub fn logged_at(index: u64, cosigned: &[Cosigned], k: usize) -> Result<u64, Problem> {
    let mut earliest: BTreeMap<&str, u64> = BTreeMap::new();
    for c in cosigned.iter().filter(|c| c.size > index) {
        let entry = earliest.entry(c.witness.as_str()).or_insert(c.time);
        *entry = (*entry).min(c.time);
    }
    kth(earliest.into_values().collect(), k, false)
}

/// The witness time of one checkpoint, the end of a wait and the measure of freshness, where an
/// attacker wants a later time: the k-th latest of the distinct witnesses' timestamps on it.
/// A time T is reached at the checkpoint only if at least k distinct witnesses cosigned it at or
/// after T. `cosigned` holds the cosignatures of that checkpoint.
pub fn checkpoint_time(cosigned: &[Cosigned], k: usize) -> Result<u64, Problem> {
    let mut latest: BTreeMap<&str, u64> = BTreeMap::new();
    for c in cosigned {
        let entry = latest.entry(c.witness.as_str()).or_insert(c.time);
        *entry = (*entry).max(c.time);
    }
    kth(latest.into_values().collect(), k, true)
}

/// The k-th earliest, or with `latest` the k-th latest, of the distinct witnesses' times.
fn kth(mut times: Vec<u64>, k: usize, latest: bool) -> Result<u64, Problem> {
    if k == 0 || times.len() < k {
        return Err(Problem::TooFew {
            found: times.len(),
            needed: k,
        });
    }
    times.sort_unstable();
    if latest {
        times.reverse();
    }
    times.get(k - 1).copied().ok_or(Problem::TooFew {
        found: times.len(),
        needed: k,
    })
}

/// Checks that no witness cosigned with a timestamp below one it gave a smaller checkpoint:
/// a witness's time never decreases as the log grows. O(c log c).
pub fn monotonic(cosigned: &[Cosigned]) -> Result<(), Problem> {
    let mut by_witness: BTreeMap<&str, Vec<(u64, u64)>> = BTreeMap::new();
    for c in cosigned {
        by_witness
            .entry(c.witness.as_str())
            .or_default()
            .push((c.size, c.time));
    }
    for (witness, mut seen) in by_witness {
        seen.sort_unstable();
        if seen.windows(2).any(|pair| match pair {
            [(_, earlier), (_, later)] => later < earlier,
            _ => false,
        }) {
            return Err(Problem::Decreasing {
                witness: witness.to_owned(),
            });
        }
    }
    Ok(())
}

/// The duration of a wait (the cooldown of rules 24 and 25, the recovery delay of rule 38.1):
/// the witness time of the later checkpoint minus the logged-at time of the entry, never a
/// local reading. Zero when the checkpoint's time is not after it.
pub fn waited(logged_at: u64, checkpoint_time: u64) -> u64 {
    checkpoint_time.saturating_sub(logged_at)
}

/// Freshness (rule 39): whether a checkpoint's witness time is less than `window` seconds
/// before the local clock `now`. A local clock more than `skew` seconds behind the witness time
/// is clock skew, U7010, reported as `Err`.
pub fn fresh(checkpoint_time: u64, now: u64, window: u64, skew: u64) -> Result<bool, u64> {
    if checkpoint_time > now.saturating_add(skew) {
        return Err(checkpoint_time - now);
    }
    Ok(now.saturating_sub(checkpoint_time) < window)
}
