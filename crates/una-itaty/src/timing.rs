//! The witness obligations of a registry (`spec/30 §14` rules 1 and 5, ADR 0122 rules 36.4
//! and 38.1): which checkpoints MUST carry the witness threshold of cosignatures before the log
//! grows past them, and which cosignatures a checkpoint may take, so that the deadline of every
//! pending recovery, dispute and co-signer change is one checkpoint that the service and every
//! client find alike.
//!
//! **What a request's deadline is.** A request's start is the k-th earliest cosignature on its
//! start checkpoint, the first that includes it ([`una_project::tlog::witness::logged_at`] over
//! that checkpoint's cosignatures); its target is seven days of witness time later. The
//! service fixes its deadline D as the first checkpoint it signs at or after the target, by its
//! clock, and keeps every checkpoint on its side of the target:
//! - a checkpoint signed before the target never takes a cosignature at or after it once that
//!   would give it the threshold of such cosignatures, so its witness time stays below the
//!   target and it is never the deadline;
//! - a checkpoint signed at or after the target never takes a cosignature before it, so every
//!   honest cosignature on D is at or after the target, and a place that strips some cannot
//!   bring its witness time below the target while the threshold remain;
//! - the start checkpoint never takes a cosignature earlier than the start once the start is
//!   fixed, so the target never moves.
//!
//! Every pinned multiple of 64 between the request and D is signed before the target, and
//! every one from D on after it, so the client's pinned search (`spec/30 §14` rule 5) finds D:
//! the first pinned checkpoint whose witness time reaches the target is the first multiple of
//! 64 at or after D, and the first size after the multiple below it whose witness time reaches
//! the target is D.
//!
//! **Obligations** ([`obligation`]). The log grows past its newest checkpoint only when that
//! checkpoint carries the threshold, if it is a pinned multiple of 64 (the cadence), a request's
//! start checkpoint, a request's deadline checkpoint, or the checkpoint just before a deadline
//! (the newest checkpoint once a target has passed, before D is signed). A witness cosigns only
//! the newest checkpoint and never one below a checkpoint it cosigned, so an obligation can be
//! met only before the log grows: while it is unmet, the service logs nothing, keeps serving the
//! checkpoint at `witness/next`, and takes cosignatures as they come, for as long as witnesses
//! are down. The checkpoint before a deadline is the one exception: once its target has passed
//! and it carries no cosignature before the target, no cosignature can complete it, and the log
//! grows, the gap the client then records as a missing deadline checkpoint.
//!
//! Costs: O(R) for R requests ever logged, per obligation check and per cosignature.

use una_project::client::CADENCE;
use una_project::tlog::owner::RECOVERY_DELAY;
use una_project::tlog::witness::Cosigned;

/// A request whose seven days are measured: a recovery, a dispute, a co-signer change, or a
/// cancellation of a recovery, whose own seven days bound the overruling of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timed {
    /// Its log index.
    pub index: u64,
    /// Its start, in witness time: the k-th earliest cosignature on its start checkpoint, once
    /// that checkpoint carries the threshold.
    pub start: Option<u64>,
    /// Its deadline checkpoint D, by size: the first checkpoint signed at or after its target.
    pub deadline: Option<u64>,
}

impl Timed {
    /// A request logged at `index`, not yet started.
    pub fn new(index: u64) -> Timed {
        Timed {
            index,
            start: None,
            deadline: None,
        }
    }

    /// Its target, seven days of witness time after its start, once the start is fixed.
    pub fn target(&self) -> Option<u64> {
        self.start.map(|start| start.saturating_add(RECOVERY_DELAY))
    }
}

/// Why a checkpoint MUST carry the witness threshold of cosignatures before the log grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Obligation {
    /// Its size is a multiple of 64, the pinned cadence.
    Cadence,
    /// It is the start checkpoint of the request logged at this index.
    Start(u64),
    /// It is the deadline checkpoint of the request logged at this index.
    Deadline(u64),
    /// It is the checkpoint just before the deadline of the request logged at this index.
    BeforeDeadline(u64),
}

impl Obligation {
    /// The obligation, as the end of a sentence.
    pub fn describe(self) -> String {
        match self {
            Obligation::Cadence => format!("its size is a multiple of {CADENCE}"),
            Obligation::Start(index) => {
                format!("it is the first checkpoint that includes the request at {index}")
            }
            Obligation::Deadline(index) => {
                format!("it is the deadline checkpoint of the request at {index}")
            }
            Obligation::BeforeDeadline(index) => format!(
                "it is the checkpoint just before the deadline of the request at {index}, whose target has passed"
            ),
        }
    }
}

/// The obligations of the checkpoint of `size`, the newest, at `now` by the service's clock,
/// with `requests` the timed requests: what it MUST carry before the log grows. The checkpoint
/// of size 0 has none.
pub fn obligation<'r>(
    size: u64,
    now: u64,
    requests: impl Iterator<Item = &'r Timed>,
) -> Vec<Obligation> {
    let mut found = Vec::new();
    if size == 0 {
        return found;
    }
    if size.is_multiple_of(CADENCE) {
        found.push(Obligation::Cadence);
    }
    for request in requests {
        if request.index.saturating_add(1) == size {
            found.push(Obligation::Start(request.index));
        }
        if request.deadline == Some(size) {
            found.push(Obligation::Deadline(request.index));
        }
        let due = request.target().is_some_and(|target| now >= target);
        if request.deadline.is_none() && request.index < size && due {
            found.push(Obligation::BeforeDeadline(request.index));
        }
    }
    found
}

/// Whether the checkpoint of `size` may take a cosignature at `time` for `request`, given the
/// cosignatures it carries already that count (`lines`) and the threshold that counts them
/// (`k`): one sentence when it may not.
pub fn admits(
    request: &Timed,
    size: u64,
    time: u64,
    lines: &[Cosigned],
    k: usize,
) -> Result<(), String> {
    let index = request.index;
    if size <= index {
        return Ok(());
    }
    if size == index.saturating_add(1)
        && let Some(start) = request.start
        && time < start
    {
        return Err(format!(
            "the start of the request at {index} is fixed at {start}; a cosignature of its start checkpoint at {time} would move it"
        ));
    }
    let Some(target) = request.target() else {
        return Ok(());
    };
    if request.deadline.is_some_and(|deadline| size >= deadline) {
        if time < target {
            return Err(format!(
                "the checkpoint of size {size} was signed at or after the target {target} of the request at {index}; it takes no cosignature before it"
            ));
        }
        return Ok(());
    }
    let late = lines.iter().filter(|line| line.time >= target).count();
    if time >= target && late.saturating_add(1) >= k {
        return Err(format!(
            "the checkpoint of size {size} was signed before the target {target} of the request at {index}; one more cosignature at or after it would make it the deadline: cosign the newest checkpoint"
        ));
    }
    Ok(())
}

/// Whether the checkpoint just before a deadline, carrying `lines` that count, can still
/// complete the threshold `k` for the target `target`: it needs at least one cosignature
/// before the target, since it takes fewer than `k` at or after it.
pub fn completable(lines: &[Cosigned], target: u64, k: usize) -> bool {
    lines.len() >= k || lines.iter().any(|line| line.time < target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(witness: &str, size: u64, time: u64) -> Cosigned {
        Cosigned {
            witness: witness.to_owned(),
            size,
            time,
        }
    }

    #[test]
    fn obligations_follow_cadence_starts_and_deadlines() {
        let mut request = Timed::new(9);
        assert_eq!(obligation(0, 0, std::iter::empty()), Vec::new());
        assert_eq!(
            obligation(64, 0, std::iter::empty()),
            vec![Obligation::Cadence]
        );
        assert_eq!(
            obligation(10, 0, std::iter::once(&request)),
            vec![Obligation::Start(9)]
        );
        request.start = Some(100);
        let target = 100 + RECOVERY_DELAY;
        assert_eq!(
            obligation(12, target - 1, std::iter::once(&request)),
            Vec::new()
        );
        assert_eq!(
            obligation(12, target, std::iter::once(&request)),
            vec![Obligation::BeforeDeadline(9)]
        );
        request.deadline = Some(13);
        assert_eq!(
            obligation(13, target, std::iter::once(&request)),
            vec![Obligation::Deadline(9)]
        );
        assert_eq!(
            obligation(14, target, std::iter::once(&request)),
            Vec::new()
        );
    }

    #[test]
    fn cosignatures_keep_each_checkpoint_on_its_side_of_the_target() {
        let mut request = Timed::new(9);
        // Before the start is fixed, anything goes.
        assert_eq!(admits(&request, 10, 5, &[], 2), Ok(()));
        request.start = Some(100);
        let target = 100 + RECOVERY_DELAY;
        // The start never moves earlier.
        assert!(admits(&request, 10, 99, &[], 2).is_err());
        assert_eq!(admits(&request, 10, 100, &[], 2), Ok(()));
        // Before the deadline: fewer than k cosignatures at or after the target.
        assert_eq!(admits(&request, 12, target, &[], 2), Ok(()));
        let one_late = [line("w0", 12, target)];
        assert!(admits(&request, 12, target + 1, &one_late, 2).is_err());
        assert_eq!(admits(&request, 12, target - 1, &one_late, 2), Ok(()));
        // From the deadline on: none before the target.
        request.deadline = Some(13);
        assert!(admits(&request, 13, target - 1, &[], 2).is_err());
        assert_eq!(admits(&request, 14, target, &[], 2), Ok(()));
        // Checkpoints before the request are not its.
        assert_eq!(admits(&request, 9, 0, &[], 2), Ok(()));
    }

    #[test]
    fn a_checkpoint_before_a_deadline_needs_one_cosignature_before_the_target() {
        assert!(completable(&[line("w0", 12, 5)], 10, 2));
        assert!(!completable(&[line("w0", 12, 10)], 10, 2));
        assert!(!completable(&[], 10, 2));
        assert!(completable(
            &[line("w0", 12, 10), line("w1", 12, 11)],
            10,
            2
        ));
    }
}
