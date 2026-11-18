//! Naming conventions (`spec/01 §4`, U1005). type parameters follow the
//! upper-case convention, and owners and module path segments the lower-case one; each name is
//! checked where it is defined.

use std::collections::{BTreeMap, BTreeSet};

/// Why a lower-case name breaks the convention, and a name that keeps it.
pub(crate) fn lower(name: &str) -> Option<(String, String)> {
    let problem = if name.contains("__") {
        "must not contain `__`"
    } else if name.ends_with('_') {
        "must not end with `_`"
    } else {
        return None;
    };
    let mut suggestion = String::new();
    for c in name.chars() {
        if !(c == '_' && (suggestion.is_empty() || suggestion.ends_with('_'))) {
            suggestion.push(c);
        }
    }
    let suggestion = suggestion.trim_end_matches('_').to_owned();
    Some((problem.to_owned(), suggestion))
}

/// Whether an upper-case name is UpperCamelCase, and a name that is.
pub(crate) fn upper(name: &str) -> Option<(String, String)> {
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric());
    if valid {
        return None;
    }
    let mut suggestion = String::new();
    let mut capital = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if capital {
                suggestion.extend(c.to_uppercase());
            } else {
                suggestion.push(c);
            }
            capital = false;
        } else {
            capital = true;
        }
    }
    Some(("must be UpperCamelCase".to_owned(), suggestion))
}

/// Whether a requirement identifier follows `[A-Z][A-Z0-9]*(_[A-Z0-9]+)*_[0-9]+`.
pub(crate) fn requirement(name: &str) -> bool {
    let Some((head, number)) = name.rsplit_once('_') else {
        return false;
    };
    let mut parts = head.split('_');
    let first = parts.next().unwrap_or("");
    let upper_digit = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit();
    !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
        && first.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && first.chars().all(upper_digit)
        && parts.all(|part| !part.is_empty() && part.chars().all(upper_digit))
}

/// A requirement identifier that keeps the convention, derived from `name` (#743): upper case,
/// `_` between its words, and a number at the end. Trailing digits become the number
/// (`SOLD_OUT2` gives `SOLD_OUT_2`), and a name without one gets `_1` (`SOLD_OUT_1`).
pub(crate) fn requirement_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_uppercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let mut out = out.trim_end_matches('_').to_owned();
    if !out.starts_with(|c: char| c.is_ascii_uppercase()) {
        out.insert_str(0, if out.is_empty() { "REQ" } else { "REQ_" });
    }
    let digits = out.chars().rev().take_while(char::is_ascii_digit).count();
    let split = out.len() - digits;
    if digits > 0 && split > 0 && !out[..split].ends_with('_') {
        out.insert(split, '_');
    }
    if !requirement(&out) {
        out.push_str("_1");
    }
    out
}

/// The requirement identifiers of a package, read once to suggest free ones (#743, #1474).
/// For each base `B`, it keeps the numbers `n` of the identifiers written `B_n` (decimal, no
/// leading zero, as a suggestion is written), sorted, and for each number the last number of
/// the run of consecutive numbers from it. Building it is O(n log n) for n identifiers, and each
/// [`RequirementNames::unused`] is O(log n) set and binary searches plus the length of the name,
/// so the suggestions for all the identifiers of a package cost O(n log n) together.
pub(crate) struct RequirementNames<'a> {
    ids: &'a BTreeSet<&'a str>,
    /// Per base: the numbers taken, ascending, and where the run of each one ends.
    runs: BTreeMap<&'a str, (Vec<u64>, Vec<u64>)>,
}

impl<'a> RequirementNames<'a> {
    pub(crate) fn new(ids: &'a BTreeSet<&'a str>) -> Self {
        let mut runs: BTreeMap<&'a str, (Vec<u64>, Vec<u64>)> = BTreeMap::new();
        for id in ids {
            let Some((head, digits)) = id.rsplit_once('_') else {
                continue;
            };
            let canonical = !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && (digits.len() == 1 || !digits.starts_with('0'));
            if let (true, Ok(number)) = (canonical, digits.parse::<u64>()) {
                runs.entry(head).or_default().0.push(number);
            }
        }
        for (numbers, ends) in runs.values_mut() {
            // Distinct identifiers with canonical digits hold distinct numbers.
            numbers.sort_unstable();
            ends.resize(numbers.len(), 0);
            let mut end = 0;
            for i in (0..numbers.len()).rev() {
                let next = numbers.get(i + 1).copied();
                let number = numbers.get(i).copied().unwrap_or(0);
                if next.is_none() || number.checked_add(1) != next {
                    end = number;
                }
                if let Some(slot) = ends.get_mut(i) {
                    *slot = end;
                }
            }
        }
        RequirementNames { ids, runs }
    }

    /// `candidate`, or the first of it with its number raised that no identifier holds. When
    /// the numbers above it are all taken up to `u64::MAX`, the number space is exhausted, and
    /// the search goes on from `candidate` with `_1` appended (`X_18446744073709551615` gives
    /// `X_18446744073709551615_1`). The loop ends: each round makes the name longer, and only
    /// finitely many identifiers are taken.
    pub(crate) fn unused(&self, mut candidate: String) -> String {
        loop {
            if !self.ids.contains(candidate.as_str()) {
                return candidate;
            }
            let (head, number) = candidate
                .rsplit_once('_')
                .unwrap_or((candidate.as_str(), "1"));
            let start: u64 = number.parse().unwrap_or(1);
            match start
                .checked_add(1)
                .and_then(|first| self.first_free(head, first))
            {
                Some(free) => return format!("{head}_{free}"),
                None => candidate.push_str("_1"),
            }
        }
    }

    /// The least number from `first` up whose `head_n` is not taken, or `None` when every one up
    /// to `u64::MAX` is.
    fn first_free(&self, head: &str, first: u64) -> Option<u64> {
        let Some((numbers, ends)) = self.runs.get(head) else {
            return Some(first);
        };
        match numbers.binary_search(&first) {
            Err(_) => Some(first),
            Ok(index) => ends.get(index).and_then(|end| end.checked_add(1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_case_names() {
        assert_eq!(lower("total"), None);
        assert_eq!(lower("a_b1"), None);
        assert_eq!(
            lower("total_"),
            Some(("must not end with `_`".to_owned(), "total".to_owned()))
        );
        assert_eq!(
            lower("a__b"),
            Some(("must not contain `__`".to_owned(), "a_b".to_owned()))
        );
    }

    #[test]
    fn upper_case_names() {
        assert_eq!(upper("SeatCount"), None);
        assert_eq!(upper("HTTPServer"), None);
        assert_eq!(
            upper("Seat_Count"),
            Some(("must be UpperCamelCase".to_owned(), "SeatCount".to_owned()))
        );
    }

    #[test]
    fn requirement_identifiers() {
        for valid in ["EVENTS_3", "A_1", "AUDIT_LOG_3", "R2_D2_7"] {
            assert!(requirement(valid), "{valid}");
        }
        for invalid in [
            "EVENTS3",
            "EVENTS_",
            "Events_3",
            "EVENTS__3",
            "_12",
            "EVENTS_1A",
        ] {
            assert!(!requirement(invalid), "{invalid}");
        }
    }

    #[test]
    fn requirement_names_derived_from_the_one_written() {
        for (written, derived) in [
            ("SOLD_OUT", "SOLD_OUT_1"),
            ("SOLD_OUT2", "SOLD_OUT_2"),
            ("Events_3", "EVENTS_3"),
            ("EVENTS__3", "EVENTS_3"),
            ("EVENTS_1A", "EVENTS_1A_1"),
        ] {
            let name = requirement_name(written);
            assert_eq!(name, derived, "{written}");
            assert!(requirement(&name), "{name}");
        }
        let taken = BTreeSet::from(["SOLD_OUT_1", "SOLD_OUT_2"]);
        assert_eq!(
            RequirementNames::new(&taken).unused("SOLD_OUT_1".to_owned()),
            "SOLD_OUT_3"
        );
    }

    /// The suggestion the earlier probe gave: the first `head_n`, n above the candidate's
    /// number, that is not taken (#743). Kept here to compare against on ordinary input.
    fn probed(candidate: &str, taken: &BTreeSet<&str>) -> String {
        if !taken.contains(candidate) {
            return candidate.to_owned();
        }
        let (head, number) = candidate.rsplit_once('_').unwrap_or((candidate, "1"));
        let start: u64 = number.parse().unwrap_or(1);
        (start + 1..)
            .map(|n| format!("{head}_{n}"))
            .find(|name| !taken.contains(name.as_str()))
            .unwrap()
    }

    #[test]
    fn unused_requirements_match_the_probe_on_ordinary_names() {
        let taken = BTreeSet::from([
            "A_1",
            "A_2",
            "A_3",
            "A_5",
            "A_6",
            "A_9",
            "A_07",
            "A_10",
            "B_1",
            "B_X_1",
            "B_X_2",
            "C",
            "C_2",
            "D_1_1",
            "D_1_2",
            "REQ_1",
            "R2_D2_7",
            "R2_D2_8",
            "A_99999999999999999999",
        ]);
        let names = RequirementNames::new(&taken);
        for candidate in [
            "A_1",
            "A_3",
            "A_4",
            "A_5",
            "A_6",
            "A_7",
            "A_8",
            "A_9",
            "A_07",
            "A_0",
            "B_1",
            "B_X_1",
            "B_X_2",
            "C",
            "D_1_1",
            "REQ_1",
            "R2_D2_7",
            "Z_1",
            "A_99999999999999999999",
        ] {
            assert_eq!(
                names.unused(candidate.to_owned()),
                probed(candidate, &taken),
                "{candidate}"
            );
        }
    }

    #[test]
    fn unused_requirements_do_not_overflow_the_number() {
        // #1474: `start + 1` overflowed a u64 here (a panic in debug, a wrap to `X_0` in release).
        let max = "X_18446744073709551615";
        let taken = BTreeSet::from([max]);
        assert_eq!(
            RequirementNames::new(&taken).unused(max.to_owned()),
            "X_18446744073709551615_1"
        );
        // A run of taken numbers that reaches u64::MAX exhausts the number space as well.
        let taken = BTreeSet::from(["X_18446744073709551614", max, "X_18446744073709551615_1"]);
        let names = RequirementNames::new(&taken);
        assert_eq!(
            names.unused("X_18446744073709551614".to_owned()),
            "X_18446744073709551614_1"
        );
        assert_eq!(names.unused(max.to_owned()), "X_18446744073709551615_2");
        for name in ["X_18446744073709551614_1", "X_18446744073709551615_2"] {
            assert!(requirement(name), "{name}");
        }
    }

    #[test]
    fn unused_requirements_scale_on_a_run_of_taken_names() {
        // #1474: n identifiers whose derived names are a run of n taken names cost O(n^2) probes
        // with one probe per candidate (about 5 * 10^9 here); one search per identifier now.
        let n: u64 = 100_000;
        let owned: Vec<String> = (1..=n).map(|i| format!("X_{i}")).collect();
        let taken: BTreeSet<&str> = owned.iter().map(String::as_str).collect();
        let names = RequirementNames::new(&taken);
        let expected = format!("X_{}", n + 1);
        for candidate in &owned {
            assert_eq!(names.unused(candidate.clone()), expected);
        }
    }
}
