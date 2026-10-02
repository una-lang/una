//! The runtime in safe Rust: the command line, grants, host operations and the trace of a
//! native executable (`spec/16 §3`). [`crate::ffi`] calls it from the C ABI.

use std::cell::RefCell;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;
use una_diag::{Code, Diagnostic, Location, Position, render_human};
use una_interp::{Event, EventOutcome, Grant};

/// The exit code of a usage error (`spec/13 §5`, ADR 0036).
const USAGE: i32 = 2;

/// What `una build` embeds in the executable (`spec/16 §2`), as JSON: the `[run] grants`, and
/// every `uses` of `main` with its operations and where it is written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// The grant specs of `[run] grants`.
    pub grants: Vec<String>,
    /// The `uses` clauses of `main`.
    pub uses: Vec<Need>,
}

/// One `uses` clause of `main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    /// The reference, without `@`.
    pub name: String,
    /// The capability.
    pub capability: String,
    /// The operations.
    pub ops: Vec<String>,
    /// Where the clause is written.
    pub location: Location,
}

impl Meta {
    /// The description `una build` embedded, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Meta> {
        let value: Value = serde_json::from_str(text).ok()?;
        let grants = value
            .get("grants")?
            .as_array()?
            .iter()
            .map(|g| g.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let uses = value
            .get("uses")?
            .as_array()?
            .iter()
            .map(|need| {
                Some(Need {
                    name: need.get("name")?.as_str()?.to_owned(),
                    capability: need.get("capability")?.as_str()?.to_owned(),
                    ops: need
                        .get("ops")?
                        .as_array()?
                        .iter()
                        .map(|op| op.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()?,
                    location: location(need.get("location")?)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Meta { grants, uses })
    }
}

fn location(value: &Value) -> Option<Location> {
    let position = |value: &Value| {
        Some(Position {
            line: u32::try_from(value.get("line")?.as_u64()?).ok()?,
            col: u32::try_from(value.get("col")?.as_u64()?).ok()?,
        })
    };
    Some(Location {
        file: value.get("file")?.as_str()?.to_owned(),
        start: position(value.get("start")?)?,
        end: position(value.get("end")?)?,
    })
}

/// The options of the executable's command line (`spec/16 §3`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The specs of `--grant` flags, in order.
    pub grants: Vec<String>,
    /// `--trace <file>`.
    pub trace: Option<PathBuf>,
    /// `--max-depth <n>`.
    pub max_depth: usize,
}

/// The command line, without the program name, or the message of a usage error.
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        max_depth: una_interp::DEFAULT_MAX_DEPTH,
        ..Options::default()
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| rest.next().cloned())
                .ok_or_else(|| format!("`{flag}` needs a value"))
        };
        match flag {
            "--grant" => options.grants.push(value()?),
            "--trace" => options.trace = Some(PathBuf::from(value()?)),
            "--max-depth" => {
                let text = value()?;
                options.max_depth = text
                    .parse()
                    .map_err(|_| format!("`--max-depth {text}` is not a number"))?;
            }
            other => {
                return Err(format!(
                    "unexpected argument `{other}`; the options are --grant <spec>, --trace <file> and --max-depth <n>"
                ));
            }
        }
    }
    Ok(options)
}

/// The state of a run: the grants, and the trace when `--trace` asks for one.
struct State {
    trace: Option<Vec<Event>>,
    failed_write: Option<io::Error>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Runs the executable: parses the command line, checks `main`'s capabilities against the
/// grants, calls `entry`, the compiled `main`, and writes the trace. Returns the exit code.
pub fn start(args: &[String], meta: &str, entry: &dyn Fn() -> i64) -> i32 {
    let mut stderr = io::stderr();
    let Some(meta) = Meta::parse(meta) else {
        let _ = writeln!(
            stderr,
            "error: the executable's embedded description is invalid; rebuild it with `una build`"
        );
        return 3;
    };
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    let grants = match grants(&meta, &options) {
        Ok(grants) => grants,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    if una_interp::stack_bytes(options.max_depth).is_none() {
        let _ = writeln!(
            stderr,
            "error: `--max-depth {}` needs more stack than the system can address",
            options.max_depth
        );
        return USAGE;
    }
    let ungranted = check_grants(&meta, &grants);
    if !ungranted.is_empty() {
        let _ = write!(
            stderr,
            "{}",
            render_human(&ungranted, &Default::default(), false)
        );
        return 1;
    }
    STATE.with(|state| {
        *state.borrow_mut() = Some(State {
            trace: options.trace.as_ref().map(|_| Vec::new()),
            failed_write: None,
        });
    });
    let status = entry();
    let state = STATE.with(|state| state.borrow_mut().take());
    let mut code = match status {
        0 => 0,
        _ => 1,
    };
    if let Some(state) = state {
        if let Some(error) = state.failed_write {
            let _ = writeln!(stderr, "error: cannot write the output: {error}");
            code = USAGE;
        }
        if let (Some(path), Some(events)) = (&options.trace, state.trace)
            && let Err(error) = std::fs::write(path, una_interp::trace_json(&events))
        {
            let _ = writeln!(
                stderr,
                "error: cannot write the trace to `{}`: {error}",
                path.display()
            );
            code = USAGE;
        }
    }
    code
}

/// The grants of the run: the embedded `[run] grants`, then the `--grant` flags, every
/// relative path resolved against the current directory (`spec/16 §3`). Key files are read
/// and parsed here, so that an unreadable or invalid one is a usage error before `main`.
fn grants(meta: &Meta, options: &Options) -> Result<Vec<Grant>, String> {
    let mut grants = Vec::new();
    for spec in meta.grants.iter().chain(&options.grants) {
        let Some(grant) = Grant::parse(spec, Path::new("")) else {
            return Err(format!(
                "`{spec}` is not a grant; grants are console, clock, env, keys:<file> and files:<path>:<access>"
            ));
        };
        if let Grant::Keys { file } = &grant {
            let text = std::fs::read_to_string(file).map_err(|error| {
                format!("cannot read the key file `{}`: {error}", file.display())
            })?;
            una_interp::parse_keys(&text).map_err(|reason| {
                format!("the key file `{}` is invalid: {reason}", file.display())
            })?;
        }
        grants.push(grant);
    }
    Ok(grants)
}

/// U3012 for each `uses` of `main` whose capability no grant provides, as `una run` reports
/// it (`spec/13 §5`): host grants cover every operation of their capability.
pub fn check_grants(meta: &Meta, grants: &[Grant]) -> Vec<Diagnostic> {
    meta.uses
        .iter()
        .filter(|need| !grants.iter().any(|g| g.capability() == need.capability))
        .map(|need| {
            let ops: Vec<String> = need.ops.iter().map(|op| format!("`{op}`")).collect();
            Diagnostic::error(
                Code::U3012,
                format!(
                    "`main` uses {} of `{}`, which neither `[run] grants` nor a `--grant` flag grants",
                    ops.join(", "),
                    need.capability
                ),
                need.location.clone(),
            )
        })
        .collect()
}

/// `Console.print`: writes the line and a newline to stdout, and records the event
/// (`spec/14 §3`, `spec/11 §5`).
pub fn console_print(line: &str) {
    let mut stdout = io::stdout().lock();
    let written = writeln!(stdout, "{line}").and_then(|()| stdout.flush());
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        if let Err(error) = written
            && state.failed_write.is_none()
        {
            state.failed_write = Some(error);
        }
        if let Some(trace) = state.trace.as_mut() {
            trace.push(Event {
                seq: trace.len() + 1,
                instance: "host:Console".to_owned(),
                capability: "Console".to_owned(),
                op: "print".to_owned(),
                args: vec![("line".to_owned(), una_core::quote(line))],
                outcome: EventOutcome::Ok("unit".to_owned()),
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn the_command_line_takes_grants_a_trace_and_a_depth() {
        let options = parse_args(&strings(&[
            "--grant",
            "console",
            "--grant=env",
            "--trace",
            "t.json",
            "--max-depth",
            "50",
        ]))
        .unwrap();
        assert_eq!(options.grants, ["console", "env"]);
        assert_eq!(options.trace, Some(PathBuf::from("t.json")));
        assert_eq!(options.max_depth, 50);
        assert!(parse_args(&strings(&["--json"])).is_err());
        assert!(parse_args(&strings(&["--grant"])).is_err());
        assert!(parse_args(&strings(&["--max-depth", "many"])).is_err());
    }

    #[test]
    fn main_needs_a_grant_for_each_capability_it_uses() {
        let meta = Meta::parse(
            r#"{"grants":[],"uses":[{"name":"console","capability":"Console","ops":["print"],"location":{"file":"src/hello.una","start":{"line":4,"col":3},"end":{"line":4,"col":33}}}]}"#,
        )
        .unwrap();
        let missing = check_grants(&meta, &[]);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].code, Code::U3012);
        assert_eq!(
            missing[0].message,
            "`main` uses `print` of `Console`, which neither `[run] grants` nor a `--grant` flag grants"
        );
        assert!(check_grants(&meta, &[Grant::Console]).is_empty());
    }
}
