//! The solver protocol of `spec/12 §5`: the Z3 built into `una`, at the version of
//! `verification.lock`, one child process per query with its whole script (ADR 0124). The
//! child is started, fed and waited for by the [`Solver`] the caller supplies (`una-cli`'s,
//! which re-executes `una` in its solver mode); this module reads what it answered and applies
//! the proof condition of ADR 0124 rule 8: a proof needs a normal exit with status 0 and
//! `unsat`, and anything else is not one.

use std::fmt;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How the parent runs one query (ADR 0124 rules 4 to 7): it starts the solver child, writes
/// the script to its stdin, reads its stdout and the head of its stderr, kills it past the
/// wall-clock guard or on cancellation, and says how it ended.
pub trait Solver: Send + Sync + fmt::Debug {
    /// The Z3 version this `una` can run, or why it cannot (U5003).
    fn version(&self) -> Result<String, String>;
    /// How this `una` names itself in a U5003 message, as `this una (0.4.0)`.
    fn describe(&self) -> String;
    /// The SHA-256 of the solver library built into this `una`, which the proof cache's key
    /// covers (ADR 0124 rule 10); empty when none is.
    fn library_digest(&self) -> String;
    /// Runs `script` on a fresh child within `wallclock`, stopping early when `cancel` is set.
    ///
    /// # Errors
    ///
    /// U5003: the child could not be started, or is not this `una` (the handshake of rule 4).
    fn run(
        &self,
        script: &str,
        wallclock: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<Raw, String>;
}

/// What a child wrote and how it ended, before the proof condition is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raw {
    /// Everything the child wrote on stdout after its handshake lines.
    pub stdout: String,
    /// The first 4 KiB the child wrote on stderr.
    pub stderr: String,
    /// How it ended.
    pub ended: Ended,
    /// When an outer sandbox confines the child (macOS), the operations it denies, in the
    /// order of rule 7; `None` when the child confined itself.
    pub outer_denied: Option<Vec<String>>,
}

/// How a child ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// By its own exit, with this status.
    Exited(i32),
    /// By this signal.
    Signaled(i32),
    /// Killed by the parent at the wall-clock guard.
    Wallclock,
    /// Killed by the parent because the run no longer needs the answer.
    Cancelled,
    /// Killed by the parent past the memory limit (macOS, by the footprint).
    Memory,
}

/// Why no answer could be made of what the child wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The query is not valid SMT-LIB for the solver: a bug of the encoding (U8001).
    Query(String),
    /// The child broke its confinement: a security event (U5003).
    Solver(String),
}

/// `SIGSYS`, by which the filter kills a child that made a forbidden call.
#[cfg(target_os = "linux")]
const SIGSYS: i32 = 31;
#[cfg(not(target_os = "linux"))]
const SIGSYS: i32 = 12;
/// `SIGXCPU`, the end of a child past its processor-time limit.
const SIGXCPU: i32 = 24;
/// What Z3 5.1.0's front end writes on stdout when its processor-time limit (`RLIMIT_CPU`)
/// sends `SIGXCPU`: the SMT-LIB front end registers a handler for that signal
/// (`register_on_timeout_proc` in Z3's `util/timeout.cpp`) that prints this word on a line of
/// its own and ends the process with `_Exit(0)` (#2008). Since #2268 the child blocks the
/// signal so that handler never runs, and ends by `SIGXCPU` itself
/// (`una_sys::solver::processor_time`), whose fallback writes the same word; the word stays
/// read as `cpu` either way. Pinned for the pinned version, like the memout exit, and read as a top-level atom
/// anywhere in stdout: no answer of a query has it there, since a status word, a model and
/// values are the only top-level items.
const CPU_LIMIT_WORD: &str = "timeout";
/// The exit status of Z3 5.1.0's front end past `memory_max_size`, with the message below on
/// stderr and nothing on stdout (probe P4 of ADR 0124).
const MEMOUT_STATUS: i32 = 101;
const MEMOUT_MESSAGE: &str = "(error \"out of memory\")";
/// What Z3 5.1.0's front end answers `(get-model)` and `(get-value ...)` after `unsat`, with
/// exit status 1: `(error "line N column M: model is not available")`, pinned per solver
/// build and compared byte for byte but for the two numbers (ADR 0124 rule 8 as amended).
const MODEL_UNAVAILABLE_HEAD: &str = "(error \"line ";
const MODEL_UNAVAILABLE_TAIL: &str = ": model is not available\")";
/// The reply to the script's `(get-info :reason-unknown)` after a proof.
const NO_REASON: &str = "(:reason-unknown \"\")";

/// Whether a line is exactly the pinned "model is not available" error, with its position.
fn model_unavailable_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix(MODEL_UNAVAILABLE_HEAD) else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(MODEL_UNAVAILABLE_TAIL) else {
        return false;
    };
    let mut parts = rest.split(" column ");
    let (Some(line_number), Some(column), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !line_number.is_empty()
        && !column.is_empty()
        && line_number.bytes().all(|b| b.is_ascii_digit())
        && column.bytes().all(|b| b.is_ascii_digit())
}

/// Whether what the child wrote is a proof's exact output with exit status 1: `unsat` as the
/// first line, every later line of stdout the pinned error or the reply to `:reason-unknown`,
/// at least one of them the pinned error (the only cause of that status after a proof),
/// nothing on stderr, and no other status word anywhere.
fn exact_proof_output(stdout: &str, stderr: &str) -> bool {
    if !stderr.trim().is_empty() {
        return false;
    }
    let mut lines = stdout.lines();
    if lines.next() != Some("unsat") {
        return false;
    }
    let mut unavailable = false;
    lines.all(|line| {
        let error = model_unavailable_line(line);
        unavailable |= error;
        error || line == NO_REASON
    }) && unavailable
}

/// How many status words the output holds at the top level.
fn status_count(output: &str) -> usize {
    tokens(output)
        .iter()
        .filter(|item| matches!(item, Sexp::Atom(atom) if matches!(atom.as_str(), "sat" | "unsat" | "unknown")))
        .count()
}

/// The status of a query: `unsat` proves the obligation, `sat` refutes it (`spec/12 §5, §6`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Proven,
    Refuted,
    Unknown,
}

/// What the solver answered for one query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Answer {
    pub(crate) status: Status,
    /// Why the status is `unknown`.
    pub(crate) reason: Option<String>,
    /// The values of the requested terms, in the order they were requested, when refuted.
    pub(crate) values: Vec<String>,
    /// Why a refuted answer has no values: the solver's error after `sat`, which means the
    /// script asked for something it cannot answer (#668). Serialized even when absent, so a
    /// proof cache entry written before the field fails its tag and is solved again.
    #[serde(default)]
    pub(crate) values_error: Option<String>,
    /// What ended a child whose answer is `crash` or `memout`: the signal or the exit status
    /// (ADR 0124 rule 8). Never cached, since such an answer is not.
    #[serde(default)]
    pub(crate) detail: Option<String>,
    /// Whether an `(error ...)` line followed `sat` on stdout: the refutation of ADR 0101,
    /// which Z3 ends with status 1 (probe P4). Read at parsing, never cached.
    #[serde(skip)]
    pub(crate) error_after_sat: bool,
}

/// Why the solver could not answer: the query is not valid SMT-LIB for it, which is a bug of
/// the encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure(pub(crate) String);

/// The answer of a query from what its child wrote and how it ended (ADR 0124 rule 8, as
/// amended on 2026-10-27). Proven needs the handshake (checked by the [`Solver`]), exactly one
/// status word, `unsat`, and then either exit status 0, or exit status 1 where every line
/// after `unsat` on stdout is the pinned solver's exact "model is not available" error (what
/// the script's `(get-model)` and `(get-value ...)` answer after a proof, which makes Z3 5.1.0
/// exit with 1) or the reply to `(get-info :reason-unknown)`, with nothing on stderr and no
/// other output of any kind; refuted needs `sat` with status 0, or status 1 when an
/// `(error ...)` followed `sat`; a limit or a crash is `unknown` with its reason, never a
/// proof, never cached, and never U8001; a `SIGSYS` is U5003; an `(error ...)` before the
/// status stays U8001, whatever the exit status.
pub(crate) fn answer(raw: &Raw, values: usize) -> Result<Answer, Refused> {
    let unknown = |reason: &str, detail: Option<String>| {
        Ok(Answer {
            status: Status::Unknown,
            reason: Some(reason.to_owned()),
            values: Vec::new(),
            values_error: None,
            detail,
            error_after_sat: false,
        })
    };
    let status = match raw.ended {
        Ended::Wallclock => return unknown("wallclock", None),
        Ended::Cancelled => return unknown("cancelled", None),
        Ended::Memory => {
            return unknown("memout", Some("killed past the memory limit".to_owned()));
        }
        Ended::Signaled(SIGSYS) => {
            return Err(Refused::Solver(
                "the solver child broke its confinement (SIGSYS); this is a security event, report it"
                    .to_owned(),
            ));
        }
        Ended::Signaled(SIGXCPU) => return unknown("cpu", None),
        Ended::Signaled(signal) => {
            return unknown(
                "crash",
                Some(format!("the solver child ended by signal {signal}")),
            );
        }
        Ended::Exited(MEMOUT_STATUS) => {
            return if raw.stdout.trim().is_empty() && raw.stderr.contains(MEMOUT_MESSAGE) {
                unknown("memout", None)
            } else {
                unknown(
                    "crash",
                    Some(format!(
                        "the solver child exited with status {MEMOUT_STATUS}"
                    )),
                )
            };
        }
        // The `timeout` word with an exit: what Z3's own handler of `SIGXCPU` wrote before
        // #2268, and the fallback of the child's processor-time thread
        // (`una_sys::solver::processor_time`) if raising the signal ever returned. Since #2268 a
        // child past its processor time ends by the signal (above). Either way `cpu`, whatever
        // came before on stdout, never a proof, a refutation or U8001.
        Ended::Exited(_) if past_processor_time(&raw.stdout) => return unknown("cpu", None),
        Ended::Exited(status) => status,
    };
    let parsed = match parse(&raw.stdout, values) {
        Ok(parsed) => parsed,
        // No status at all from a child that did not end normally is a crash, not a bad query.
        Err(failure) if status != 0 && !has_status(&raw.stdout) => {
            return unknown(
                "crash",
                Some(format!(
                    "the solver child exited with status {status}: {}",
                    failure.0
                )),
            );
        }
        Err(failure) => return Err(Refused::Query(failure.0)),
    };
    // Exactly one answer to `(check-sat)`: a second status word is no proof and no refutation.
    if parsed.status != Status::Unknown && status_count(&raw.stdout) != 1 {
        return unknown(
            "crash",
            Some("the solver child wrote more than one status".to_owned()),
        );
    }
    match (parsed.status, status) {
        (Status::Proven | Status::Refuted, 0) | (Status::Unknown, _) => Ok(parsed),
        (Status::Proven, 1) if exact_proof_output(&raw.stdout, &raw.stderr) => Ok(parsed),
        (Status::Refuted, 1) if parsed.error_after_sat => Ok(parsed),
        (_, status) => unknown(
            "crash",
            Some(format!(
                "the solver child exited with status {status} after its answer"
            )),
        ),
    }
}

/// Whether the output holds the word Z3's handler of `SIGXCPU` prints, as a top-level atom.
fn past_processor_time(output: &str) -> bool {
    tokens(output)
        .iter()
        .any(|item| matches!(item, Sexp::Atom(atom) if atom == CPU_LIMIT_WORD))
}

/// Whether a status word is among the output's items, wherever it is: Z3 exits with 1 after
/// any error, so an error before the status with status 1 is still a rejected query (U8001),
/// and only an output with no status at all is a crash.
fn has_status(output: &str) -> bool {
    tokens(output)
        .iter()
        .any(|item| matches!(item, Sexp::Atom(atom) if matches!(atom.as_str(), "sat" | "unsat" | "unknown")))
}

/// A solver's output as s-expressions.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}

impl Sexp {
    fn text(&self) -> String {
        match self {
            Sexp::Atom(atom) => atom.clone(),
            Sexp::List(items) => {
                let parts: Vec<String> = items.iter().map(Sexp::text).collect();
                format!("({})", parts.join(" "))
            }
        }
    }

    fn head(&self) -> Option<&str> {
        match self {
            Sexp::List(items) => match items.first() {
                Some(Sexp::Atom(atom)) => Some(atom),
                _ => None,
            },
            Sexp::Atom(_) => None,
        }
    }
}

fn tokens(text: &str) -> Vec<Sexp> {
    let mut stack: Vec<Vec<Sexp>> = vec![Vec::new()];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' => stack.push(Vec::new()),
            ')' => {
                if stack.len() > 1
                    && let Some(items) = stack.pop()
                    && let Some(top) = stack.last_mut()
                {
                    top.push(Sexp::List(items));
                }
            }
            '"' => {
                let mut atom = String::from('"');
                while let Some(next) = chars.next() {
                    atom.push(next);
                    if next == '"' {
                        if chars.peek() == Some(&'"') {
                            if let Some(quote) = chars.next() {
                                atom.push(quote);
                            }
                        } else {
                            break;
                        }
                    }
                }
                if let Some(top) = stack.last_mut() {
                    top.push(Sexp::Atom(atom));
                }
            }
            '|' => {
                let mut atom = String::from('|');
                for next in chars.by_ref() {
                    atom.push(next);
                    if next == '|' {
                        break;
                    }
                }
                if let Some(top) = stack.last_mut() {
                    top.push(Sexp::Atom(atom));
                }
            }
            ';' => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                let mut atom = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || next == '(' || next == ')' {
                        break;
                    }
                    atom.push(next);
                    chars.next();
                }
                if let Some(top) = stack.last_mut() {
                    top.push(Sexp::Atom(atom));
                }
            }
        }
    }
    while stack.len() > 1 {
        if let Some(items) = stack.pop()
            && let Some(top) = stack.last_mut()
        {
            top.push(Sexp::List(items));
        }
    }
    stack.pop().unwrap_or_default()
}

/// Reads the answer: the status, the values requested after `sat`, the reason of `unknown`.
/// An `(error ...)` before the status means the query is not valid for the solver.
fn parse(output: &str, values: usize) -> Result<Answer, Failure> {
    let items = tokens(output);
    let mut status = None;
    let mut reason = None;
    let mut found = Vec::new();
    let mut values_error = None;
    let mut errors_after_status: Vec<String> = Vec::new();
    for item in &items {
        if status.is_some()
            && let Sexp::List(parts) = item
            && item.head() == Some("error")
        {
            errors_after_status.push(
                parts
                    .get(1)
                    .map_or_else(|| item.text(), Sexp::text)
                    .trim_matches('"')
                    .trim()
                    .to_owned(),
            );
        }
        match (status, item) {
            (None, Sexp::Atom(atom)) => {
                status = match atom.as_str() {
                    "sat" => Some(Status::Refuted),
                    "unsat" => Some(Status::Proven),
                    "unknown" => Some(Status::Unknown),
                    _ => None,
                };
            }
            (None, list) if list.head() == Some("error") => {
                return Err(Failure(format!(
                    "the solver rejected the query: {}",
                    list.text()
                )));
            }
            // After `sat`, an error answers the model or the values: the script is wrong, and
            // what the solver said is kept, so that no partial model shows (#668).
            (Some(Status::Refuted), list @ Sexp::List(parts))
                if list.head() == Some("error") && values_error.is_none() =>
            {
                values_error = Some(
                    parts
                        .get(1)
                        .map_or_else(|| list.text(), Sexp::text)
                        .trim_matches('"')
                        .trim()
                        .to_owned(),
                );
            }
            (Some(_), list) if list.head() == Some(":reason-unknown") => {
                if let Sexp::List(parts) = list {
                    reason = parts
                        .get(1)
                        .map(|r| r.text().trim_matches('"').to_owned())
                        .filter(|r| !r.is_empty());
                }
            }
            (Some(Status::Refuted), Sexp::List(pairs)) if found.is_empty() => {
                let is_values = pairs.iter().all(|pair| {
                    matches!(pair, Sexp::List(parts) if parts.len() == 2)
                        && pair.head() != Some("define-fun")
                });
                if is_values && !pairs.is_empty() {
                    found = pairs
                        .iter()
                        .filter_map(|pair| match pair {
                            Sexp::List(parts) => parts.get(1).map(Sexp::text),
                            Sexp::Atom(_) => None,
                        })
                        .collect();
                }
            }
            _ => {}
        }
    }
    let Some(status) = status else {
        return Err(Failure(format!(
            "the solver gave no status: {}",
            output.trim()
        )));
    };
    let error_after_sat = status == Status::Refuted && !errors_after_status.is_empty();
    // A refutation whose values do not answer every requested term, or that the solver
    // followed with an error, has no model to show: none is kept, never empty values (#668).
    // The other statuses keep no values at all.
    if status == Status::Refuted && (found.len() != values || values_error.is_some()) {
        if values_error.is_none() {
            values_error = Some(format!(
                "{} values for {values} requested terms",
                found.len()
            ));
        }
        found.clear();
    }
    Ok(Answer {
        status,
        reason: if status == Status::Unknown {
            Some(reason.unwrap_or_else(|| "unknown".to_owned()))
        } else {
            None
        },
        values: if status == Status::Refuted {
            found
        } else {
            Vec::new()
        },
        values_error: values_error.filter(|_| status == Status::Refuted),
        detail: None,
        error_after_sat,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_from_the_solver_output() {
        let refuted = "sat\n(\n  (define-fun |e| () Kind!val!0)\n)\n((|e| Kind!val!0) ((remaining!K |e|) (- 1)) (|path!1| true))\n(:reason-unknown \"\")\n";
        assert_eq!(
            parse(refuted, 3),
            Ok(Answer {
                status: Status::Refuted,
                reason: None,
                values: vec![
                    "Kind!val!0".to_owned(),
                    "(- 1)".to_owned(),
                    "true".to_owned()
                ],
                values_error: None,
                detail: None,
                error_after_sat: false,
            })
        );
        let proven =
            "unsat\n(error \"line 9 column 10: model is not available\")\n(:reason-unknown \"\")\n";
        assert_eq!(
            parse(proven, 2),
            Ok(Answer {
                status: Status::Proven,
                reason: None,
                values: Vec::new(),
                values_error: None,
                detail: None,
                error_after_sat: false,
            })
        );
        let rejected_values = "sat\n(\n  (define-fun |e| () Int\n    0)\n)\n(error \"line 12 column 14: unknown constant\")\n(:reason-unknown \"\")\n";
        assert_eq!(
            parse(rejected_values, 2),
            Ok(Answer {
                status: Status::Refuted,
                reason: None,
                values: Vec::new(),
                values_error: Some("line 12 column 14: unknown constant".to_owned()),
                detail: None,
                error_after_sat: true,
            })
        );
        // Fewer values than requested terms are no model either.
        assert_eq!(
            parse("sat\n((|e| 1))\n", 2),
            Ok(Answer {
                status: Status::Refuted,
                reason: None,
                values: Vec::new(),
                values_error: Some("1 values for 2 requested terms".to_owned()),
                detail: None,
                error_after_sat: false,
            })
        );
        let unknown = "unknown\n(error \"model is not available\")\n(:reason-unknown \"(resource limits reached)\")\n";
        assert_eq!(
            parse(unknown, 0).map(|a| a.reason),
            Ok(Some("(resource limits reached)".to_owned()))
        );
        let broken =
            "(error \"line 4 column 21: logic does not support nonlinear arithmetic\")\nsat\n";
        assert!(parse(broken, 0).is_err());
    }

    fn raw(stdout: &str, stderr: &str, ended: Ended) -> Raw {
        Raw {
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
            ended,
            outer_denied: None,
        }
    }

    #[test]
    fn a_proof_needs_a_normal_exit_with_status_zero_and_unsat() {
        // ADR 0124 rule 8: `unsat` from a child that exited with status 1, or was killed, is
        // not a proof; it is `unknown` with reason `crash`, never cached.
        let proven = answer(&raw("unsat\n", "", Ended::Exited(0)), 0).unwrap();
        assert_eq!(proven.status, Status::Proven);
        for ended in [Ended::Exited(1), Ended::Signaled(9), Ended::Exited(3)] {
            let not_proven = answer(&raw("unsat\n", "", ended), 0).unwrap();
            assert_eq!(not_proven.status, Status::Unknown, "{ended:?}");
            assert_eq!(not_proven.reason.as_deref(), Some("crash"), "{ended:?}");
            assert!(not_proven.detail.is_some(), "{ended:?}");
        }
    }

    #[test]
    fn a_proof_tolerates_status_one_only_for_model_not_available_errors() {
        // What Z3 5.1.0 writes for the script of a proven obligation: `unsat`, then the
        // errors of `(get-model)` and `(get-value ...)`, which it answers with status 1.
        let transcript = "unsat\n(error \"line 9 column 10: model is not available\")\n(error \"line 10 column 12: model is not available\")\n(:reason-unknown \"\")\n";
        let proven = answer(&raw(transcript, "", Ended::Exited(1)), 2).unwrap();
        assert_eq!(proven.status, Status::Proven);
        // Any other line after `unsat` with status 1 is not a proof: another error, a text
        // that is nearly the pinned one, anything on stderr, a second status.
        for (stdout, stderr) in [
            (
                "unsat\n(error \"line 9 column 10: model is not available\")\n(error \"line 11: unknown constant\")\n",
                "",
            ),
            (
                "unsat\n(error \"line 9 column 10: model is not available \")\n",
                "",
            ),
            ("unsat\n(error \"model is not available\")\n", ""),
            (
                "unsat\n(error \"line x column 10: model is not available\")\n",
                "",
            ),
            (
                "unsat\n(error \"line 9 column 10: model is not available\")\nsat\n",
                "",
            ),
            (
                "unsat\n(error \"line 9 column 10: model is not available\")\n",
                "warning\n",
            ),
            // #1360: the line `sandbox_init` writes when an outer sandbox refuses it is
            // output on stderr like any other; the child keeps it off stderr instead.
            (
                "unsat\n(error \"line 9 column 10: model is not available\")\n",
                "sandbox initialization failed: Operation not permitted\n",
            ),
            ("unsat\nunsat\n", ""),
        ] {
            let not_proven = answer(&raw(stdout, stderr, Ended::Exited(1)), 2).unwrap();
            assert_eq!(not_proven.reason.as_deref(), Some("crash"), "{stdout:?}");
        }
        // Two status words with status 0 are no proof either.
        let twice = answer(&raw("unsat\nunsat\n", "", Ended::Exited(0)), 0).unwrap();
        assert_eq!(twice.reason.as_deref(), Some("crash"));
        // An error before the status, with status 1 as Z3 exits, is still a rejected query.
        let rejected = "(error \"line 2 column 11: unknown constant y\")\nsat\n";
        assert!(matches!(
            answer(&raw(rejected, "", Ended::Exited(1)), 0),
            Err(Refused::Query(_))
        ));
    }

    #[test]
    fn a_refutation_tolerates_status_one_only_after_an_error_following_sat() {
        let refuted = answer(&raw("sat\n((|e| 1))\n", "", Ended::Exited(0)), 1).unwrap();
        assert_eq!(refuted.status, Status::Refuted);
        assert_eq!(refuted.values, vec!["1".to_owned()]);
        // ADR 0101: `sat`, then an error, exit status 1: still refuted, with no values.
        let after_error = answer(
            &raw(
                "sat\n(error \"line 2: unknown constant\")\n",
                "",
                Ended::Exited(1),
            ),
            1,
        )
        .unwrap();
        assert_eq!(after_error.status, Status::Refuted);
        assert!(after_error.values_error.is_some());
        // `sat` with status 1 and no error line is a crash, and so is `sat` with too few
        // values and status 1: a short model is not the error line that excuses the status.
        let crashed = answer(&raw("sat\n((|e| 1))\n", "", Ended::Exited(1)), 1).unwrap();
        assert_eq!(crashed.reason.as_deref(), Some("crash"));
        let short = answer(&raw("sat\n((|e| 1))\n", "", Ended::Exited(1)), 2).unwrap();
        assert_eq!(short.reason.as_deref(), Some("crash"));
    }

    #[test]
    fn limits_and_crashes_are_unknown_with_their_reason_and_a_sigsys_is_refused() {
        let cases = [
            (Ended::Wallclock, "wallclock"),
            (Ended::Cancelled, "cancelled"),
            (Ended::Memory, "memout"),
            (Ended::Signaled(SIGXCPU), "cpu"),
            (Ended::Signaled(11), "crash"),
        ];
        for (ended, reason) in cases {
            let answer = answer(&raw("", "", ended), 0).unwrap();
            assert_eq!(answer.status, Status::Unknown, "{ended:?}");
            assert_eq!(answer.reason.as_deref(), Some(reason), "{ended:?}");
        }
        // Z3 5.1.0 past `memory_max_size`: status 101, the message on stderr, nothing on
        // stdout (probe P4); status 101 with another stderr is a crash.
        let memout = answer(
            &raw("", "(error \"out of memory\")\n", Ended::Exited(101)),
            0,
        )
        .unwrap();
        assert_eq!(memout.reason.as_deref(), Some("memout"));
        let other = answer(&raw("", "segmentation fault\n", Ended::Exited(101)), 0).unwrap();
        assert_eq!(other.reason.as_deref(), Some("crash"));
        // A child killed by the filter is a security event, never a status.
        assert!(matches!(
            answer(&raw("", "", Ended::Signaled(SIGSYS)), 0),
            Err(Refused::Solver(message)) if message.contains("broke its confinement")
        ));
        // An error before the status is a bad query, whatever the exit status; no status at
        // all from a child that exited abnormally is a crash.
        assert!(matches!(
            answer(&raw("(error \"line 1: bad\")\n", "", Ended::Exited(0)), 0),
            Err(Refused::Query(_))
        ));
        let silent = answer(&raw("", "", Ended::Exited(2)), 0).unwrap();
        assert_eq!(silent.reason.as_deref(), Some("crash"));
    }

    #[test]
    fn the_end_of_z3s_processor_time_handler_is_cpu_and_never_a_proof_issue_2008() {
        // Z3 5.1.0 past `RLIMIT_CPU`: its `SIGXCPU` handler prints `timeout` and exits with
        // status 0, which the `limits` gate shows on the real child. Alone, after a status
        // word, or with another exit status, it is `unknown` with reason `cpu`: never U8001,
        // never a proof, never a refutation, never cached.
        for (stdout, status) in [
            ("timeout\n", 0),
            ("unsat\ntimeout\n", 0),
            ("sat\ntimeout\n", 0),
            ("unknown\ntimeout\n", 0),
            ("timeout\n", 1),
        ] {
            let answer = answer(&raw(stdout, "", Ended::Exited(status)), 0).unwrap();
            assert_eq!(answer.status, Status::Unknown, "{stdout:?} {status}");
            assert_eq!(answer.reason.as_deref(), Some("cpu"), "{stdout:?} {status}");
            assert_eq!(answer.detail, None, "{stdout:?} {status}");
        }
        // The word inside a list is not the handler's: a model may name anything.
        let model = answer(
            &raw(
                "sat\n((define-fun timeout () Int 1))\n((|e| 1))\n",
                "",
                Ended::Exited(0),
            ),
            1,
        )
        .unwrap();
        assert_eq!(model.status, Status::Refuted);
        // The reason a solver gives for `unknown` is a string, not the handler's word.
        let reason = answer(
            &raw(
                "unknown\n(:reason-unknown \"timeout\")\n",
                "",
                Ended::Exited(0),
            ),
            0,
        )
        .unwrap();
        assert_eq!(reason.reason.as_deref(), Some("timeout"));
    }
}
