//! The name rules a registry enforces (ADR 0122 rules 44 and 45): the forms of owner and
//! package names, reserved names and their skeletons, skeletons unique among owners and among
//! one owner's packages and the unscoped names, and new owner names held for review at edit
//! distance 1 from one of the owners with the most dependents.
//!
//! The forms, the skeleton and the reserved names are `una_project::names`, the ones every
//! client checks. Costs: a skeleton is O(L) for a name of L bytes and its lookup O(L) by hash;
//! the distance check looks up the L + 1 deletion keys of the new name in a deletion index,
//! O(L²) bytes hashed, and ranks the few names found by their dependents, O(O) each for O owners.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use una_project::names::{is_name_word, reserved_like, skeleton};

/// The bytes of an owner name (rule 44).
pub const OWNER_BYTES: (usize, usize) = (2, 39);
/// The most bytes of a package name's last segment (rule 44).
pub const NAME_BYTES: usize = 64;
/// How many owners, by dependents, a new owner name may not be one edit from without review.
pub const POPULAR_OWNERS: usize = 1_000;

/// Why a name is refused, or held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The name is refused.
    Refused(String),
    /// The name is held for review by the registry (rule 45, distance).
    Held(String),
}

/// The deletion keys of a name: the name itself and each name with one character deleted.
fn deletions(name: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    out.insert(name.to_owned());
    for (i, c) in name.char_indices() {
        let mut shorter = String::with_capacity(name.len());
        shorter.push_str(name.get(..i).unwrap_or_default());
        shorter.push_str(name.get(i + c.len_utf8()..).unwrap_or_default());
        out.insert(shorter);
    }
    out
}

/// Whether `a` and `b` differ by one insertion, deletion, substitution or swap of adjacent
/// characters.
fn one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a == b {
        return false;
    }
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    match long.len() - short.len() {
        0 => {
            let diff: Vec<usize> = (0..a.len()).filter(|i| a.get(*i) != b.get(*i)).collect();
            match diff.as_slice() {
                [_] => true,
                [i, j] => {
                    *j == i + 1 && a.get(*i) == b.get(*j) && a.get(*j) == b.get(*i)
                }
                _ => false,
            }
        }
        1 => {
            let at = (0..short.len())
                .find(|i| short.get(*i) != long.get(*i))
                .unwrap_or(short.len());
            short.get(at..) == long.get(at + 1..)
        }
        _ => false,
    }
}

/// The names a registry holds, for the checks of new ones.
#[derive(Clone, Debug, Default)]
pub struct Names {
    /// Owner skeletons, to the owner.
    owner_skeletons: HashMap<String, String>,
    /// Deletion keys of the owners, to the owners.
    deletion_index: HashMap<String, BTreeSet<String>>,
    /// For each owner (`None` for the unscoped names of Itaúna), its package names' skeletons.
    package_skeletons: HashMap<Option<String>, HashMap<String, String>>,
    /// How many logged needs name a package of each owner.
    dependents: BTreeMap<String, u64>,
}

impl Names {
    /// No names.
    pub fn new() -> Names {
        Names::default()
    }

    /// Whether `owner` is a valid new owner name, not reserved and not confusable with one
    /// held; `Held` when it is one edit from a popular owner and `reviewed` is false.
    pub fn check_owner(&self, owner: &str, reviewed: bool) -> Result<(), Refusal> {
        if !is_name_word(owner, OWNER_BYTES.0, OWNER_BYTES.1) {
            return Err(Refusal::Refused(format!(
                "`{owner}` is not an owner name: lower-case letters, digits and `_`, from {} to {} bytes, starting with a letter, with no `__` and no trailing `_`",
                OWNER_BYTES.0, OWNER_BYTES.1
            )));
        }
        if let Some(reserved) = reserved_like(owner) {
            return Err(Refusal::Refused(format!(
                "`{owner}` is reserved, or has the skeleton of the reserved name `{reserved}`"
            )));
        }
        if let Some(existing) = self.owner_skeletons.get(&skeleton(owner)) {
            return Err(Refusal::Refused(if existing == owner {
                format!("the owner `{owner}` exists")
            } else {
                format!("`{owner}` has the skeleton of the owner `{existing}`")
            }));
        }
        if reviewed {
            return Ok(());
        }
        let mut near: BTreeSet<&String> = BTreeSet::new();
        for key in deletions(owner) {
            if let Some(owners) = self.deletion_index.get(&key) {
                near.extend(owners.iter().filter(|other| one_edit(owner, other)));
            }
        }
        for other in near {
            if self.popular(other) {
                return Err(Refusal::Held(format!(
                    "`{owner}` is one edit from the owner `{other}`, and is held for review"
                )));
            }
        }
        Ok(())
    }

    /// Whether `owner` is among the [`POPULAR_OWNERS`] owners with the most dependents. An
    /// owner tied with others at the boundary counts as popular. O(O).
    fn popular(&self, owner: &str) -> bool {
        let own = self.dependents.get(owner).copied().unwrap_or(0);
        let above = self.dependents.values().filter(|d| **d > own).count();
        above < POPULAR_OWNERS
    }

    /// Records a registered owner.
    pub fn add_owner(&mut self, owner: &str) {
        self.owner_skeletons
            .insert(skeleton(owner), owner.to_owned());
        for key in deletions(owner) {
            self.deletion_index
                .entry(key)
                .or_default()
                .insert(owner.to_owned());
        }
        self.dependents.entry(owner.to_owned()).or_insert(0);
    }

    /// Whether `name` (`owner/name`, or an unscoped name of Itaúna) is a valid new package name:
    /// its form, not reserved, and its last segment's skeleton not that of another of the
    /// owner's names or of an unscoped name.
    pub fn check_package(&self, owner: Option<&str>, name: &str) -> Result<(), Refusal> {
        if let Some(owner) = owner
            && !is_name_word(owner, OWNER_BYTES.0, OWNER_BYTES.1)
        {
            return Err(Refusal::Refused(format!("`{owner}` is not an owner name")));
        }
        if !is_name_word(name, 1, NAME_BYTES) {
            return Err(Refusal::Refused(format!(
                "`{name}` is not a package name: lower-case letters, digits and `_`, at most {NAME_BYTES} bytes, starting with a letter, with no `__` and no trailing `_`"
            )));
        }
        if let Some(reserved) = reserved_like(name) {
            return Err(Refusal::Refused(format!(
                "`{name}` is reserved, or has the skeleton of the reserved name `{reserved}`"
            )));
        }
        let own = skeleton(name);
        let scopes: Vec<Option<String>> = match owner {
            Some(owner) => vec![Some(owner.to_owned()), None],
            None => vec![None],
        };
        for scope in scopes {
            if let Some(existing) = self
                .package_skeletons
                .get(&scope)
                .and_then(|names| names.get(&own))
                && existing != name
            {
                return Err(Refusal::Refused(match scope {
                    Some(owner) => format!(
                        "`{name}` has the skeleton of `{owner}/{existing}`, another of its owner's packages"
                    ),
                    None => format!("`{name}` has the skeleton of the official package `{existing}`"),
                }));
            }
        }
        Ok(())
    }

    /// Records a package name.
    pub fn add_package(&mut self, owner: Option<&str>, name: &str) {
        self.package_skeletons
            .entry(owner.map(str::to_owned))
            .or_default()
            .insert(skeleton(name), name.to_owned());
    }

    /// Counts a logged need of a package of `owner`.
    pub fn add_dependent(&mut self, owner: &str) {
        *self.dependents.entry(owner.to_owned()).or_insert(0) += 1;
    }
}

/// Splits a package name into its owner, if any, and its last segment.
pub fn split(name: &str) -> (Option<&str>, &str) {
    match name.split_once('/') {
        Some((owner, last)) => (Some(owner), last),
        None => (None, name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_are_checked_by_form_reservation_and_skeleton() {
        let mut names = Names::new();
        assert!(names.check_owner("acme", false).is_ok());
        names.add_owner("acme");
        assert!(matches!(names.check_owner("acme", false), Err(Refusal::Refused(_))));
        assert!(matches!(names.check_owner("4cme", false), Err(Refusal::Refused(_))));
        assert!(matches!(names.check_owner("ac_me", false), Err(Refusal::Refused(_))));
        assert!(matches!(names.check_owner("std", false), Err(Refusal::Refused(_))));
        assert!(matches!(names.check_owner("s7d", false), Err(Refusal::Refused(_))));
        assert!(matches!(names.check_owner("a", false), Err(Refusal::Refused(_))));
        assert!(matches!(names.check_owner("Acme", false), Err(Refusal::Refused(_))));
        // One edit from a popular owner: held, unless reviewed.
        assert!(matches!(names.check_owner("acmes", false), Err(Refusal::Held(_))));
        assert!(matches!(names.check_owner("cme", false), Err(Refusal::Held(_))));
        assert!(matches!(names.check_owner("acne", false), Err(Refusal::Held(_))));
        assert!(matches!(names.check_owner("camE", false), Err(Refusal::Refused(_))));
        assert!(matches!(names.check_owner("came", false), Err(Refusal::Held(_))));
        assert!(names.check_owner("acmes", true).is_ok());
        assert!(names.check_owner("zenith", false).is_ok());
    }

    #[test]
    fn only_popular_owners_hold_their_neighbours() {
        let mut names = Names::new();
        names.add_owner("quiet");
        for i in 0..POPULAR_OWNERS {
            let owner = format!("busy{i}x");
            names.add_owner(&owner);
            names.add_dependent(&owner);
        }
        assert!(names.check_owner("quiets", false).is_ok());
        assert!(matches!(names.check_owner("busy7xz", false), Err(Refusal::Held(_))));
    }

    #[test]
    fn package_names_are_unique_by_skeleton_per_owner_and_against_official_ones() {
        let mut names = Names::new();
        names.add_package(None, "json");
        names.add_package(Some("acme"), "ledger");
        assert!(names.check_package(Some("acme"), "ledger").is_ok());
        assert!(names.check_package(Some("acme"), "1edger").is_err());
        assert!(names.check_package(Some("other"), "1edger").is_ok());
        assert!(names.check_package(Some("acme"), "js0n").is_err());
        assert!(names.check_package(None, "jsoon").is_err());
        assert!(names.check_package(Some("acme"), "module").is_err());
        assert!(names.check_package(Some("acme"), "std_").is_err());
        assert!(names.check_package(Some("acme"), "Bad").is_err());
    }

    #[test]
    fn one_edit_is_levenshtein_one_or_a_swap() {
        assert!(one_edit("acme", "acne"));
        assert!(one_edit("acme", "acmes"));
        assert!(one_edit("acme", "cme"));
        assert!(one_edit("acme", "came"));
        assert!(!one_edit("acme", "acme"));
        assert!(!one_edit("acme", "zcne"));
        assert!(!one_edit("acme", "ac"));
    }
}
