//! The witness obligations of a registry and the deadlines of owners' requests (`spec/30 §14`
//! rules 1 and 5, ADR 0122 rules 36.4 and 38.1 and Amendment 7d, `spec/30 §16` rule 10).
//!
//! **What a request's deadline is.** A request's start is the k-th earliest cosignature on its
//! start checkpoint, the first that includes it ([`una_project::tlog::witness::logged_at`] over
//! that checkpoint's cosignatures); its target is seven days of witness time later. Its
//! deadline checkpoint D is the first checkpoint that reaches the target with the cosignatures
//! it takes: the service fixes D as the cosignatures arrive, when the newest checkpoint above
//! the request carries the threshold of counted cosignatures at or after the target. Lines are
//! counted as the client counts them, with k the larger of the threshold of the root version
//! the checkpoint names and the last root's; a line is early before the target, late at or
//! after it.
//!
//! **The rules on a cosignature** ([`admits`]), for each timed request, refuse one that would:
//! 1. give a checkpoint that carries an early line the threshold of late lines
//!    ([`Crossing::EarlyThenLate`]); a checkpoint with no early line takes late lines freely;
//! 2. make a checkpoint other than the newest reach the target
//!    ([`Crossing::OlderReachesTarget`]): only the newest checkpoint may newly reach a target;
//! 3. put an early line on D or any later checkpoint ([`Crossing::EarlyOnDeadline`]);
//! 4. move a fixed start earlier ([`Crossing::BeforeStart`]).
//!
//! **Held-back lines** ([`holds`]). An early line on the newest checkpoint above a request
//! whose D is not fixed is held back, neither published nor served, while that checkpoint may
//! still become D. Held and published lines late for a request both count toward it reaching
//! that request's target ([`reaches`]), and toward the checkpoint's obligations. Once the log
//! grows past the checkpoint, its held lines are published, but those early for a request
//! whose D it is, which are dropped; rule 2 never applies to them. So D carries only late lines as the service publishes it, and a
//! place that strips lines from D cannot bring its witness time below the target with lines the
//! service published.
//!
//! So the client's pinned search finds D: every checkpoint between the request and D stays
//! below the target, and D is the first to reach it.
//!
//! **Obligations** ([`obligation`]). The log grows past its newest checkpoint only once that
//! checkpoint carries the threshold, its published and held lines counted, when it is a pinned
//! multiple of 64, a request's start checkpoint, a request's deadline checkpoint, or the newest
//! one once a target has passed and before its D is fixed. With the rules above, the newest
//! checkpoint can always complete while k honest witnesses are live: with no early line
//! published on it, late lines are taken freely. The uncompletable exception
//! ([`uncompletable`]) stays: an obligated checkpoint that no admissible cosignature can
//! complete lets the log grow, and clients find what needs it unavailable (U7013).
//!
//! Costs: O(A) per obligation check and per cosignature of the newest checkpoint, for the A
//! requests still active; O(R) for a cosignature of an older checkpoint, R every request timed.

use una_project::client::CADENCE;
use una_project::tlog::owner::RECOVERY_DELAY;
use una_project::tlog::witness::Cosigned;

/// A request whose seven days are measured: a recovery, a dispute, a co-signer change, or a
/// cancellation of a waiting request, whose own seven days bound an overruling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timed {
    /// Its log index.
    pub index: u64,
    /// The owner whose chain it is on.
    pub owner: String,
    /// Its start, in witness time: the k-th earliest cosignature on its start checkpoint, once
    /// that checkpoint carries the threshold.
    pub start: Option<u64>,
    /// Its deadline checkpoint D, by size: the first checkpoint that reaches its target.
    pub deadline: Option<u64>,
}

impl Timed {
    /// A request of `owner` logged at `index`, not yet started.
    pub fn new(index: u64, owner: &str) -> Timed {
        Timed {
            index,
            owner: owner.to_owned(),
            start: None,
            deadline: None,
        }
    }

    /// Its target, seven days of witness time after its start, once the start is fixed.
    pub fn target(&self) -> Option<u64> {
        self.start.map(|start| start.saturating_add(RECOVERY_DELAY))
    }

    /// Whether its D may still be fixed at a checkpoint: its target is known and D is not.
    pub fn waiting(&self) -> bool {
        self.deadline.is_none() && self.start.is_some()
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
    /// It is the newest checkpoint once the target of the request logged at this index has
    /// passed, its D not fixed: it may be D, or the checkpoint just before it.
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
                "the target of the request at {index} has passed, and this checkpoint may be its deadline or the one just before it"
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

/// Which rule a refused cosignature would break, for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crossing {
    /// Rule 1: a late line that would give a checkpoint carrying an early line the threshold
    /// of late lines.
    EarlyThenLate,
    /// Rule 2: a late line that would make a checkpoint other than the newest reach the target.
    OlderReachesTarget,
    /// Rule 3: an early line on D or a later checkpoint.
    EarlyOnDeadline,
    /// Rule 4: a line on the start checkpoint before the fixed start.
    BeforeStart,
}

impl Crossing {
    /// The rule, as one sentence about the request at `index`.
    pub fn describe(self, index: u64) -> String {
        match self {
            Crossing::EarlyThenLate => format!(
                "this checkpoint carries a cosignature before the target of the request at {index}, so it takes fewer than the threshold at or after it"
            ),
            Crossing::OlderReachesTarget => format!(
                "only the newest checkpoint may newly reach the target of the request at {index}: cosign the newest checkpoint"
            ),
            Crossing::EarlyOnDeadline => format!(
                "this checkpoint is the deadline of the request at {index}, or after it, and takes no cosignature before its target"
            ),
            Crossing::BeforeStart => format!(
                "the start of the request at {index} is fixed, and a cosignature of its start checkpoint before it would move it"
            ),
        }
    }
}

/// Whether the checkpoint of `size` (`newest` when it is the newest) may take a cosignature at
/// `time` for `request`, given the counted lines it carries as published (`lines`) and the
/// threshold that counts them (`k`, the larger of its named root's and the last root's).
pub fn admits(
    request: &Timed,
    size: u64,
    newest: bool,
    time: u64,
    lines: &[Cosigned],
    k: usize,
) -> Result<(), Crossing> {
    let index = request.index;
    if size <= index {
        return Ok(());
    }
    if size == index.saturating_add(1)
        && let Some(start) = request.start
        && time < start
    {
        return Err(Crossing::BeforeStart);
    }
    let Some(target) = request.target() else {
        return Ok(());
    };
    if request.deadline.is_some_and(|deadline| size >= deadline) {
        return if time < target {
            Err(Crossing::EarlyOnDeadline)
        } else {
            Ok(())
        };
    }
    if time < target {
        return Ok(());
    }
    let late = lines.iter().filter(|line| line.time >= target).count();
    let reaches = late.saturating_add(1) >= k;
    if reaches && lines.iter().any(|line| line.time < target) {
        return Err(Crossing::EarlyThenLate);
    }
    if reaches && !newest {
        return Err(Crossing::OlderReachesTarget);
    }
    Ok(())
}

/// Whether a cosignature at `time` on the checkpoint of `size` is held back for `request`: the
/// checkpoint is the newest above the request, the request's D is not fixed, so the checkpoint
/// may still become D, and the line is early.
pub fn holds(request: &Timed, size: u64, newest: bool, time: u64) -> bool {
    newest
        && size > request.index
        && request.deadline.is_none()
        && request.target().is_some_and(|target| time < target)
}

/// Whether the checkpoint of `size`, the newest, with its counted `lines`, published and held
/// back, reaches the target of `request` whose D is not fixed: its lines late for the target
/// are at least `k`. Then it is that request's D; its lines late for it, held because they are
/// early for another request, are published once the log grows past it.
pub fn reaches(request: &Timed, size: u64, lines: &[Cosigned], k: usize) -> bool {
    size > request.index
        && request.deadline.is_none()
        && request
            .target()
            .is_some_and(|target| lines.iter().filter(|line| line.time >= target).count() >= k)
}

/// Whether the newest checkpoint, with its published counted lines `published` and `held`
/// back ones, can still reach `k` counted cosignatures under [`admits`] once `target` has
/// passed and new lines are late: with no early line published, late lines are taken freely;
/// with one, fewer than `k` late lines are, beside its early ones.
pub fn completable(published: &[Cosigned], held: &[Cosigned], target: u64, k: usize) -> bool {
    if published.len().saturating_add(held.len()) >= k {
        return true;
    }
    if !published.iter().any(|line| line.time < target) {
        return true;
    }
    let early = published
        .iter()
        .chain(held)
        .filter(|line| line.time < target)
        .count();
    early.saturating_add(k.saturating_sub(1)) >= k
}

/// The request, by its index, whose passed target leaves the newest checkpoint of `size`
/// unable to reach `k` counted cosignatures, as [`completable`] decides: the uncompletable
/// exception, which lets the log grow past it; what needs it is then an omission to every
/// client (U7013), the fail-safe direction. `None` when every obligation can still be met.
pub fn uncompletable<'r>(
    size: u64,
    now: u64,
    requests: impl Iterator<Item = &'r Timed>,
    published: &[Cosigned],
    held: &[Cosigned],
    k: usize,
) -> Option<u64> {
    requests
        .filter(|request| {
            request.index < size && request.deadline.is_none_or(|deadline| size < deadline)
        })
        .find(|request| {
            request
                .target()
                .is_some_and(|target| now >= target && !completable(published, held, target, k))
        })
        .map(|request| request.index)
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

    fn started(index: u64, start: u64) -> Timed {
        let mut request = Timed::new(index, "acme");
        request.start = Some(start);
        request
    }

    #[test]
    fn obligations_follow_cadence_starts_and_deadlines() {
        let mut request = Timed::new(9, "acme");
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
    fn each_rule_refuses_at_its_boundary() {
        let request = started(9, 100);
        let target = 100 + RECOVERY_DELAY;
        // Rule 4: the start checkpoint takes nothing before the start, the start itself.
        assert_eq!(
            admits(&request, 10, true, 99, &[], 2),
            Err(Crossing::BeforeStart)
        );
        assert_eq!(admits(&request, 10, true, 100, &[], 2), Ok(()));
        // Rule 1: with an early line, the k-th late one is refused, the one before it taken.
        let early = [line("w0", 12, target - 1)];
        assert_eq!(
            admits(&request, 12, true, target, &early, 1),
            Err(Crossing::EarlyThenLate)
        );
        let mixed = [line("w0", 12, target - 1), line("w1", 12, target)];
        assert_eq!(
            admits(&request, 12, true, target + 1, &mixed, 2),
            Err(Crossing::EarlyThenLate)
        );
        assert_eq!(admits(&request, 12, true, target, &early, 2), Ok(()));
        // With no early line, late lines are taken freely on the newest checkpoint.
        let late = [line("w0", 12, target)];
        assert_eq!(admits(&request, 12, true, target, &late, 2), Ok(()));
        // Rule 2: an older checkpoint never newly reaches the target, one short is taken.
        assert_eq!(
            admits(&request, 12, false, target, &late, 2),
            Err(Crossing::OlderReachesTarget)
        );
        assert_eq!(admits(&request, 12, false, target, &[], 2), Ok(()));
        // Rule 3: from D on, nothing before the target; the target itself is taken.
        let mut fixed = request.clone();
        fixed.deadline = Some(13);
        assert_eq!(
            admits(&fixed, 14, true, target - 1, &[], 2),
            Err(Crossing::EarlyOnDeadline)
        );
        assert_eq!(admits(&fixed, 13, true, target, &[], 2), Ok(()));
        // Checkpoints before the request are not its.
        assert_eq!(admits(&request, 9, false, 0, &[], 2), Ok(()));
    }

    #[test]
    fn early_lines_on_a_checkpoint_that_may_become_the_deadline_are_held() {
        let request = started(9, 100);
        let target = 100 + RECOVERY_DELAY;
        assert!(holds(&request, 12, true, target - 1));
        assert!(!holds(&request, 12, true, target));
        assert!(!holds(&request, 12, false, target - 1));
        assert!(!holds(&request, 9, true, target - 1));
        let mut fixed = request.clone();
        fixed.deadline = Some(12);
        assert!(!holds(&fixed, 13, true, target - 1));
        // It reaches the target with k late lines.
        let late = [line("w0", 12, target), line("w1", 12, target + 5)];
        assert!(reaches(&request, 12, &late, 2));
        assert!(!reaches(&request, 12, &late[..1], 2));
    }

    #[test]
    fn the_newest_checkpoint_always_completes_while_witnesses_are_live() {
        let target = 10;
        // No early line published: late lines are free, k = 1 included.
        assert!(completable(&[], &[], target, 1));
        assert!(completable(&[], &[line("w0", 12, 5)], target, 3));
        // An early line published: one more early and k - 1 late.
        assert!(completable(&[line("w0", 12, 5)], &[], target, 2));
        let request = started(3, 0);
        assert_eq!(
            uncompletable(5, RECOVERY_DELAY, std::iter::once(&request), &[], &[], 2),
            None
        );
    }
}
