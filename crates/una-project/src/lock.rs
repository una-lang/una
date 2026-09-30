//! `verification.lock` (`spec/12 §9`, ADR 0021 rule 3).

use una_diag::Code;

use crate::ProjectError;
use crate::document::{integer, parse, string};

/// The solver version of the default lock: the version pinned by the repository's
/// `verification.lock`, which a test keeps in step.
pub const DEFAULT_SOLVER_VERSION: &str = "5.1.0";

/// The settings of `verification.lock`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// `encoding_version`: always `una-smt-1` in v0.1.
    pub encoding_version: String,
    /// `solver`: always `z3` in v0.1.
    pub solver: String,
    /// `solver_version`: the exact solver version.
    pub solver_version: String,
    /// `logic`: always `QF_UFLIA` in v0.1.
    pub logic: String,
    /// `step_budget`: the solver's resource limit per query.
    pub step_budget: u64,
    /// `random_seed`: the solver's seed.
    pub random_seed: u64,
}

impl Default for Lock {
    /// The lock of a single file (ADR 0021 rule 3): the values of the repository's lock.
    fn default() -> Lock {
        Lock {
            encoding_version: "una-smt-1".to_owned(),
            solver: "z3".to_owned(),
            solver_version: DEFAULT_SOLVER_VERSION.to_owned(),
            logic: "QF_UFLIA".to_owned(),
            step_budget: 5_000_000,
            random_seed: 0,
        }
    }
}

/// The keys with a fixed value in v0.1.
const FIXED: [(&str, &str); 3] = [
    ("encoding_version", "una-smt-1"),
    ("solver", "z3"),
    ("logic", "QF_UFLIA"),
];

/// Parses `verification.lock`. exactly the six keys of `spec/12 §9`, with the
/// values v0.1 implements; every problem is U7004.
pub fn parse_lock(text: &str) -> (Lock, Vec<ProjectError>) {
    let mut lock = Lock::default();
    let mut errors = Vec::new();
    let Some(document) = parse(text, Code::U7004, &mut errors) else {
        return (lock, errors);
    };
    let mut seen = Vec::new();
    for (key, value) in document.get_ref().iter() {
        let name = key.get_ref().as_ref();
        seen.push(name.to_owned());
        if let Some((_, expected)) = FIXED.iter().find(|(fixed, _)| *fixed == name) {
            if let Some(found) = string(name, value, Code::U7004, &mut errors)
                && found.value != *expected
            {
                errors.push(ProjectError::new(
                    Code::U7004,
                    found.range,
                    format!(
                        "`{name}` must be \"{expected}\" in v0.1, not \"{}\"",
                        found.value
                    ),
                ));
            }
            continue;
        }
        match name {
            "solver_version" => {
                if let Some(version) = string(name, value, Code::U7004, &mut errors) {
                    if version.value.trim().is_empty() {
                        errors.push(ProjectError::new(
                            Code::U7004,
                            version.range,
                            "`solver_version` must not be empty",
                        ));
                    } else {
                        lock.solver_version = version.value;
                    }
                }
            }
            "step_budget" | "random_seed" => {
                let minimum = i128::from(name == "step_budget");
                let Some(number) = integer(name, value, Code::U7004, &mut errors) else {
                    continue;
                };
                match u64::try_from(number) {
                    Ok(number) if i128::from(number) >= minimum => {
                        if name == "step_budget" {
                            lock.step_budget = number;
                        } else {
                            lock.random_seed = number;
                        }
                    }
                    _ => errors.push(ProjectError::new(
                        Code::U7004,
                        value.span(),
                        format!("`{name}` must be {minimum} or more"),
                    )),
                }
            }
            other => errors.push(ProjectError::new(
                Code::U7004,
                key.span(),
                format!("unknown key `{other}` in verification.lock"),
            )),
        }
    }
    for required in [
        "encoding_version",
        "solver",
        "solver_version",
        "logic",
        "step_budget",
        "random_seed",
    ] {
        if !seen.iter().any(|name| name == required) {
            errors.push(ProjectError::new(
                Code::U7004,
                0..0,
                format!("verification.lock has no `{required}`"),
            ));
        }
    }
    (lock, errors)
}
