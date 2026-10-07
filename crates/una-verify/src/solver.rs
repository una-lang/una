//! The solver protocol of `spec/12 §5`: Z3 at the version of `verification.lock`, one
//! `z3 -in -smt2` process per query with its whole script.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

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
}

/// Why the solver could not answer: the query is not valid SMT-LIB for it, which is a bug of
/// the encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure(pub(crate) String);

/// The version the solver at `solver` reports, `5.1.0` for `Z3 version 5.1.0 - 64 bit`.
pub(crate) fn version(solver: &Path) -> Result<String, String> {
    let output = Command::new(solver)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .skip_while(|word| *word != "version")
        .nth(1)
        .map(str::to_owned)
        .ok_or_else(|| format!("`{}` printed no version", text.trim()))
}

/// Runs `script` and reads its answer; `values` is the number of terms whose values the script
/// requests after a `sat`. Past `wallclock` the process is killed and the answer is `unknown`
/// with reason `wallclock` (`spec/12 §5`).
pub(crate) fn run(
    solver: &Path,
    script: &str,
    values: usize,
    wallclock: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<Answer, Failure> {
    // The deadline runs from before the solver starts, and holds while the query is written:
    // a solver that does not read its input must not keep the run past its time (#286).
    let deadline = Instant::now() + wallclock;
    let mut child = Command::new(solver)
        .args(["-in", "-smt2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Failure(format!("cannot start the solver: {error}")))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let (waited, output) = std::thread::scope(|scope| {
        // Read and write on threads of their own, so that a full pipe in either direction
        // blocks neither the solver nor the wait. The writer drops the solver's input when it
        // is done; killing the solver ends a write it does not read with a broken pipe.
        let reader = stdout.map(|mut stdout| {
            scope.spawn(move || {
                let mut text = String::new();
                let _ = stdout.read_to_string(&mut text);
                text
            })
        });
        if let Some(mut stdin) = stdin {
            scope.spawn(move || {
                let _ = stdin.write_all(script.as_bytes());
            });
        }
        let waited = wait(|| !matches!(child.try_wait(), Ok(None)), deadline, cancel);
        if waited != Waited::Exited {
            let _ = child.kill();
            let _ = child.wait();
        }
        let output = reader
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        (waited, output)
    });
    let (timed_out, cancelled) = (waited == Waited::Wallclock, waited == Waited::Cancelled);
    if cancelled {
        return Ok(Answer {
            status: Status::Unknown,
            reason: Some("cancelled".to_owned()),
            values: Vec::new(),
        });
    }
    if timed_out {
        return Ok(Answer {
            status: Status::Unknown,
            reason: Some("wallclock".to_owned()),
            values: Vec::new(),
        });
    }
    parse(&output, values)
}

/// How waiting for the solver ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waited {
    /// It exited, with its answer, within its time.
    Exited,
    /// Its time ran out (`--solver-wallclock`).
    Wallclock,
    /// The run no longer needs its answer.
    Cancelled,
}

/// Waits for the solver, `exited` telling whether it has exited, until `deadline` or until
/// `cancel` is set. The deadline comes first: a solver's answer counts only when it came within
/// its time, so `--solver-wallclock 0` makes every query unknown, however fast the solver is
/// (`spec/12 §5`, #189).
fn wait(
    mut exited: impl FnMut() -> bool,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Waited {
    loop {
        if Instant::now() >= deadline {
            return Waited::Wallclock;
        }
        if exited() {
            return Waited::Exited;
        }
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Waited::Cancelled;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
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
    for item in &items {
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
    if status == Status::Refuted && found.len() != values {
        found.resize(values, String::new());
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_solver_past_its_wallclock_is_unknown_even_if_it_has_answered() {
        // #189: with `--solver-wallclock 0` every query is unknown with reason `wallclock`,
        // however fast the solver is: the deadline is checked before the solver's exit, so a
        // solver that has already answered when the first check comes is still past its time.
        let past = Instant::now();
        assert_eq!(wait(|| true, past, None), Waited::Wallclock);
        let cancel = AtomicBool::new(true);
        assert_eq!(wait(|| true, past, Some(&cancel)), Waited::Wallclock);
        // Within the deadline, an exit is the answer, and a cancellation stops the solver.
        let later = Instant::now() + Duration::from_secs(60);
        assert_eq!(wait(|| true, later, None), Waited::Exited);
        assert_eq!(wait(|| false, later, Some(&cancel)), Waited::Cancelled);
        let mut polls = 0;
        let exits_third = || {
            polls += 1;
            polls == 3
        };
        assert_eq!(wait(exits_third, later, None), Waited::Exited);
    }

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

    #[cfg(unix)]
    #[test]
    fn a_solver_that_reads_no_query_is_stopped_at_its_wallclock() {
        // #286: the deadline holds while the query is being written. A solver that never reads
        // its input leaves a query larger than a pipe unwritten; the write must not keep the
        // run past the wallclock.
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("una-solver-mute-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let solver = dir.join("mute");
        let staged = dir.join("mute.tmp");
        std::fs::write(&staged, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&staged, &solver).unwrap();
        let script = ";".repeat(4 * 1024 * 1024);
        let started = Instant::now();
        let answer = run(&solver, &script, 0, Duration::from_millis(300), None).unwrap();
        let took = started.elapsed();
        assert_eq!(answer.status, Status::Unknown);
        assert_eq!(answer.reason.as_deref(), Some("wallclock"));
        assert!(took < Duration::from_secs(5), "took {took:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
