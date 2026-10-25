//! Yanks and advisories with their thresholds, and the cooldown in witness time (ADR 0122
//! rules 24, 36.4 and 41).
//!
//! - **A yank** stands only when the owner's threshold of keys valid at its log position signs
//!   it; [`yank_text`] writes the one an owner signs with `una ita yank`.
//! - **An advisory** stands when one security key of the root metadata signs it, and lifts the
//!   cooldown for the fix it names by digest only with a second security key, or the owner's
//!   threshold, besides: a single security key can hold a release back, never hurry one in.
//! - **The cooldown** is a wait in witness time: the witness time of the resolution point's
//!   checkpoint minus the logged-at time of the release, never a local clock.
//!
//! [`releases`] turns a name's verified state into what resolution reads (rule 24), with the
//! releases a key signed after a stated compromise counted as yanked (rule 38). Costs: O(1) per
//! signature tried; O(R) for R releases.

use std::collections::BTreeMap;

use super::view::owner_of;
use crate::tlog::note::{self, Key};
use crate::tlog::state::{self, Advisory, ReleaseState, Signers, Yank};
use crate::tlog::witness;
use crate::version::Version;

/// The text of a yank of `version` of `name` at `source`, after `previous`, the head of the
/// name's chain: `ita-yank/1`, `source`, `name`, `version` and `previous`, each on its line.
pub fn yank_text(
    source: &str,
    name: &str,
    version: &Version,
    previous: Option<(u64, &str)>,
) -> String {
    let previous = match previous {
        Some((index, leaf)) => format!("{index} {leaf}"),
        None => "none".to_owned(),
    };
    format!("ita-yank/1\nsource {source}\nname {name}\nversion {version}\nprevious {previous}\n")
}

/// The yank a signed note carries, when it reads and the owner's `threshold` of `owner_keys`
/// (those valid at its log position) sign it.
pub fn check_yank(document: &str, owner_keys: &[Key], threshold: usize) -> Result<Yank, String> {
    let signed = note::parse_note(document)?;
    let yank = state::parse_yank(&signed.text)?;
    let signers = signed.signers(owner_keys);
    if threshold == 0 || signers < threshold {
        return Err(format!(
            "the yank of {} {} is signed by {signers} keys of `{}`, fewer than its threshold of {threshold}",
            yank.name,
            yank.version,
            owner_of(&yank.name)
        ));
    }
    Ok(yank)
}

/// The advisory a signed note carries and who signed it, when it reads and at least one of
/// `security_keys` signs it. `owner_keys` and `threshold` are the owner's, valid at its log
/// position: with a second security key, they decide whether it lifts the cooldown for its fix.
pub fn check_advisory(
    document: &str,
    security_keys: &[Key],
    owner_keys: &[Key],
    threshold: usize,
) -> Result<(Advisory, Signers), String> {
    let signed = note::parse_note(document)?;
    let advisory = state::parse_advisory(&signed.text)?;
    let signers = Signers::of(&signed, security_keys, owner_keys, threshold);
    if !signers.valid() {
        return Err(format!(
            "the advisory {} is signed by no security key of the root",
            advisory.id
        ));
    }
    Ok((advisory, signers))
}

/// Whether a release logged at `logged_at` has waited out `cooldown` seconds at a checkpoint of
/// witness time `checkpoint_time` (rules 24 and 36.4), or needs no wait: a fix named by digest
/// in an advisory that lifts the cooldown.
pub fn cooled(logged_at: u64, checkpoint_time: u64, cooldown: u64, threshold_fix: bool) -> bool {
    threshold_fix || witness::waited(logged_at, checkpoint_time) >= cooldown
}

/// The releases of a name's verified state as resolution reads them (rule 24): yanked when a
/// yank stands or when `compromised` says a key signed it after a stated compromise; advised
/// when an advisory names it; and whether a lifting advisory names it, by digest, as its fix.
/// O(R) for R releases.
pub fn releases(
    state: &BTreeMap<Version, ReleaseState>,
    compromised: &dyn Fn(&Version) -> bool,
) -> Vec<crate::resolve::Release> {
    state
        .iter()
        .map(|(version, release)| crate::resolve::Release {
            version: version.clone(),
            yanked: release.yanked || compromised(version),
            advised: !release.advised.is_empty(),
            threshold_fix: release.threshold_fix,
            logged_at: release.logged_at,
        })
        .collect()
}
