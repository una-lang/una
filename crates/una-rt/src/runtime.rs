//! The runtime in safe Rust: the command line, grants, the call depth, host operations and the
//! trace of a native executable (`spec/16 §3`). [`crate::ffi`] calls it from the C ABI.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;
use una_diag::{Code, Diagnostic, Location, Position, render_human};
use una_interp::{Event, Grant};

use crate::host::OpSite;
use crate::render::{Failure, Types};
use crate::values;

/// The exit code of a usage error (`spec/13 §5`, ADR 0036).
const USAGE: i32 = 2;

/// What `una build` embeds in the executable (`spec/16 §2`), as JSON: the `[run] grants`, every
/// `uses` of `main` with its operations and where it is written, where each call of the
/// program is written, for U6003, and the types and operations that traces and U6001 render.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// The grant specs of `[run] grants`.
    pub grants: Vec<String>,
    /// The `uses` clauses of `main`.
    pub uses: Vec<Need>,
    /// The calls of the program, by the index that the generated code passes on each call.
    pub sites: Vec<Location>,
    /// The types the program renders.
    pub types: Types,
    /// The operations of the program, by the index the generated code passes on each one.
    pub ops: Vec<OpSite>,
    /// What U6001 needs when `main` can fail.
    pub failure: Option<Failure>,
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
        let sites = match value.get("sites") {
            Some(sites) => sites
                .as_array()?
                .iter()
                .map(location)
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        let types = match value.get("types") {
            Some(types) => Types::parse(types)?,
            None => Types::default(),
        };
        let ops = match value.get("ops") {
            Some(ops) => ops
                .as_array()?
                .iter()
                .map(OpSite::parse)
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        let failure = match value.get("failure") {
            Some(failure) => Some(Failure {
                location: location(failure.get("location")?)?,
                root: crate::render::index(failure, "root")?,
            }),
            None => None,
        };
        Some(Meta {
            grants,
            uses,
            sites,
            types,
            ops,
            failure,
        })
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
    /// `--max-memory <MiB>` (ADR 0061).
    pub max_memory: usize,
}

/// The command line, without the program name, or the message of a usage error.
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        max_depth: una_interp::DEFAULT_MAX_DEPTH,
        max_memory: una_interp::DEFAULT_MAX_MEMORY,
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
            "--max-memory" => {
                let text = value()?;
                options.max_memory = text
                    .parse()
                    .map_err(|_| format!("`--max-memory {text}` is not a number"))?;
            }
            other => {
                return Err(format!(
                    "unexpected argument `{other}`; the options are --grant <spec>, --trace <file>, --max-depth <n> and --max-memory <MiB>"
                ));
            }
        }
    }
    Ok(options)
}

/// The state of a run, on the thread that runs `main`.
pub(crate) struct State {
    pub(crate) trace: Option<Vec<Event>>,
    trace_path: Option<PathBuf>,
    pub(crate) failed_write: Option<io::Error>,
    depth: usize,
    max_depth: usize,
    sites: Vec<Location>,
    failure: Option<Failure>,
    /// The grants of the run, and the keys of its `keys:` grants.
    pub(crate) grants: Vec<Grant>,
    pub(crate) keys: BTreeMap<String, Vec<u8>>,
    pub(crate) types: Types,
    pub(crate) ops: Vec<OpSite>,
    /// Whether the last host operation failed.
    pub(crate) host_failed: bool,
    /// The number of instances of each handler that `with` installed so far.
    pub(crate) installs: BTreeMap<String, usize>,
    /// Where the stack of `main`'s thread starts, and how much of it the code may use: past
    /// it, a call aborts with U6003 before the stack overflows (ADR 0032, ADR 0061 rule 1).
    stack_base: usize,
    stack_budget: usize,
    /// The memory budget in MiB, and what the heap held when `main` started (ADR 0061).
    max_memory: usize,
    baseline: usize,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// `f` of the state of the run, when the thread runs `main`.
pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Runs the executable: parses the command line, checks `main`'s capabilities against the
/// grants, calls `entry`, the compiled `main`, on a thread with the stack that `--max-depth`
/// needs, and writes the trace. Returns the exit code.
pub fn start(args: &[String], meta: &str, entry: impl FnOnce() -> i64 + Send + 'static) -> i32 {
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
    let (grants, keys) = match grants(&meta, &options) {
        Ok(grants) => grants,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    let Some(stack) = una_interp::stack_bytes(options.max_depth) else {
        let _ = writeln!(
            stderr,
            "error: `--max-depth {}` needs more stack than the system can address",
            options.max_depth
        );
        return USAGE;
    };
    let ungranted = check_grants(&meta, &grants);
    if !ungranted.is_empty() {
        let _ = write!(
            stderr,
            "{}",
            render_human(&ungranted, &Default::default(), false)
        );
        return 1;
    }
    let state = State {
        trace: options.trace.as_ref().map(|_| Vec::new()),
        trace_path: options.trace.clone(),
        failed_write: None,
        depth: 0,
        max_depth: options.max_depth,
        sites: meta.sites,
        failure: meta.failure,
        grants,
        keys,
        types: meta.types,
        ops: meta.ops,
        host_failed: false,
        installs: BTreeMap::new(),
        stack_base: 0,
        stack_budget: stack.saturating_sub(STACK_RESERVE),
        max_memory: options.max_memory,
        baseline: 0,
    };
    let budget = options.max_memory.saturating_mul(1024 * 1024);
    let worker = std::thread::Builder::new()
        .name("una-main".to_owned())
        .stack_size(stack)
        .spawn(move || {
            let mut state = state;
            state.stack_base = stack_position(&state);
            state.baseline = crate::held();
            // ADR 0061 rule 3: past twice the budget, allocation fails and the process aborts.
            crate::ceiling(state.baseline.saturating_add(budget.saturating_mul(2)));
            STATE.with(|cell| *cell.borrow_mut() = Some(state));
            let status = entry();
            crate::ceiling(usize::MAX);
            // A debug build of the runtime checks the compiled code's reference counting: it
            // never used a value it had released, and released every value it made, which is
            // unreferenced once `main` returns (`spec/16 §4`).
            //
            // Only the detection of an internal error depends on the build (unlike N-006, where a
            // check of the program did): for a program whose counting is right, the output, exit
            // status and trace are the same in both builds, and a release executable never runs
            // the check. The tests link the debug build, so a counting bug fails them.
            let faults = if cfg!(debug_assertions) {
                (values::faults(), values::live(), values::copies())
            } else {
                (0, 0, 0)
            };
            let state = STATE.with(|cell| cell.borrow_mut().take());
            (status, state, faults)
        });
    let (status, state, (faults, leaked, copies)) = match worker.map(std::thread::JoinHandle::join)
    {
        Ok(Ok(result)) => result,
        _ => {
            let _ = writeln!(
                stderr,
                "error: cannot start `main` with the stack that `--max-depth {}` needs",
                options.max_depth
            );
            return USAGE;
        }
    };
    let mut code = if status == 0 { 0 } else { 1 };
    if let Some(state) = state {
        code = code.max(finish(state));
    }
    // A debug build also says how many lists and records were copied rather than updated in
    // place (#30), so tests can check that an update the code owns is in place. As with the
    // checks above, only a debug build says it, and it changes neither output nor exit status.
    if copies > 0 {
        let _ = writeln!(
            stderr,
            "note: una-rt (debug build): lists and records copied: {copies}"
        );
    }
    if faults > 0 {
        let _ = writeln!(
            stderr,
            "error: internal error: uses of released values: {faults}"
        );
        code = 3;
    }
    if leaked > 0 {
        let _ = writeln!(
            stderr,
            "error: internal error: values still referenced after `main` returned: {leaked}"
        );
        code = 3;
    }
    code
}

/// Reports a failed write of the output and writes the trace; returns the exit code they
/// need, 0 when both went well.
fn finish(state: State) -> i32 {
    let mut stderr = io::stderr();
    let mut code = 0;
    if let Some(error) = state.failed_write {
        let _ = writeln!(stderr, "error: cannot write the output: {error}");
        code = USAGE;
    }
    if let (Some(path), Some(events)) = (&state.trace_path, state.trace)
        && let Err(error) = std::fs::write(path, una_interp::trace_json(&events))
    {
        let _ = writeln!(
            stderr,
            "error: cannot write the trace to `{}`: {error}",
            path.display()
        );
        code = USAGE;
    }
    code
}

/// The keys of a run's `keys:` grants, by name; the first grant with a name wins, as in
/// `una run`.
type Keys = BTreeMap<String, Vec<u8>>;

/// The grants of the run: the embedded `[run] grants`, then the `--grant` flags, every
/// relative path resolved against the current directory (`spec/16 §3`), and their keys. Key
/// files are read and parsed here, so that an unreadable or invalid one is a usage error
/// before `main`.
fn grants(meta: &Meta, options: &Options) -> Result<(Vec<Grant>, Keys), String> {
    let mut grants = Vec::new();
    let mut keys = Keys::new();
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
            let loaded = una_interp::parse_keys(&text).map_err(|reason| {
                format!("the key file `{}` is invalid: {reason}", file.display())
            })?;
            for (name, key) in loaded {
                keys.entry(name).or_insert(key);
            }
        }
        grants.push(grant);
    }
    Ok((grants, keys))
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

/// The stack kept free below the budget of `main`'s thread, for the runtime's own work after the
/// last check: performing a host operation, rendering a value, writing the trace.
const STACK_RESERVE: usize = 8 * 1024 * 1024;

/// The address of `local`, a variable of the caller's frame: a measure of the stack in use.
fn stack_position<T>(local: &T) -> usize {
    std::ptr::from_ref(local).addr()
}

/// Enters the call at `site`: past the maximum depth, or when the stack of `main`'s thread runs
/// short before it, the run aborts with U6003 at the call, as in the interpreter
/// (`spec/11 §7`, ADR 0032), and so it does when the heap the run holds is over its budget
/// (ADR 0061). The output so far stays printed, the trace is written, and the executable exits
/// with code 1.
pub fn enter(site: usize) {
    let marker = site;
    let position = stack_position(&marker);
    let held = crate::held();
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let state = cell.as_mut()?;
        let used = state.stack_base.abs_diff(position);
        let message = if state.depth >= state.max_depth || used > state.stack_budget {
            format!(
                "this call exceeds the maximum call depth of {}",
                state.max_depth
            )
        } else if let Some(message) = over_budget(state, held) {
            message
        } else {
            state.depth += 1;
            return None;
        };
        let location = site_location(state, site);
        cell.take().map(|state| (state, location, message))
    });
    if let Some((state, location, message)) = abort {
        abort_run(state, location, message);
    }
}

/// Checks the memory budget at an iteration of the `for` loop at `site`: iterations are not
/// calls, so this is the only check inside a loop that builds a value (ADR 0061 rule 2). Over
/// the budget, the run aborts with U6003 at the loop, as at a call.
pub fn budget(site: usize) {
    let held = crate::held();
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let state = cell.as_mut()?;
        let message = over_budget(state, held)?;
        let location = site_location(state, site);
        cell.take().map(|state| (state, location, message))
    });
    if let Some((state, location, message)) = abort {
        abort_run(state, location, message);
    }
}

/// The U6003 message when the heap the run holds, `held` bytes, is over its budget.
fn over_budget(state: &State, held: usize) -> Option<String> {
    let budget = state.max_memory.saturating_mul(1024 * 1024);
    (held.saturating_sub(state.baseline) > budget).then(|| {
        format!(
            "this call exceeds the memory limit of {} MiB",
            state.max_memory
        )
    })
}

fn site_location(state: &State, site: usize) -> Location {
    state.sites.get(site).cloned().unwrap_or(Location {
        file: String::new(),
        start: Position { line: 1, col: 1 },
        end: Position { line: 1, col: 1 },
    })
}

/// Ends the run with U6003 at `location`, as the interpreter's abort does: the output so far
/// stays printed, the trace is written, and the executable exits with code 1.
fn abort_run(state: State, location: Location, message: String) {
    crate::ceiling(usize::MAX);
    let _ = io::stdout().flush();
    let diagnostic = Diagnostic::error(Code::U6003, message, location);
    let _ = write!(
        io::stderr(),
        "{}",
        render_human(&[diagnostic], &Default::default(), false)
    );
    let code = finish(state).max(1);
    // Nothing unwinds through the compiled frames (spec/16 §4 rule 3).
    #[allow(clippy::exit)]
    std::process::exit(code);
}

/// Reports that `main` failed with `failure`, which the caller owns and the report releases:
/// U6001 with the failure's `public` fields (`spec/04 §8`), after the output so far.
pub fn main_failed(failure: i64) {
    let _ = io::stdout().flush();
    let diagnostic = STATE.with(|cell| {
        let cell = cell.borrow();
        let state = cell.as_ref()?;
        let described = state.failure.as_ref()?;
        Some(Diagnostic::error(
            Code::U6001,
            format!(
                "`main` failed with `{}`",
                state.types.shown(described.root, failure)
            ),
            described.location.clone(),
        ))
    });
    values::release(failure);
    if let Some(diagnostic) = diagnostic {
        let _ = write!(
            io::stderr(),
            "{}",
            render_human(&[diagnostic], &Default::default(), false)
        );
    }
}

/// Leaves a call.
pub fn leave() {
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            state.depth = state.depth.saturating_sub(1);
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
        assert_eq!(options.max_memory, una_interp::DEFAULT_MAX_MEMORY);
        let options = parse_args(&strings(&["--max-memory", "64"])).unwrap();
        assert_eq!(options.max_memory, 64);
        assert!(parse_args(&strings(&["--max-memory", "many"])).is_err());
        assert!(parse_args(&strings(&["--json"])).is_err());
        assert!(parse_args(&strings(&["--grant"])).is_err());
        assert!(parse_args(&strings(&["--max-depth", "many"])).is_err());
    }

    #[test]
    fn a_debug_runtime_reports_values_left_referenced_or_used_after_release() {
        let meta = r#"{"grants":[],"uses":[]}"#;
        let freed = start(&[], meta, || {
            values::release(values::text_word("freed".to_owned()));
            0
        });
        assert_eq!(freed, 0);
        let leaked = start(&[], meta, || {
            values::text_word("kept".to_owned());
            0
        });
        assert_eq!(leaked, 3);
        let reused = start(&[], meta, || {
            let text = values::text_word("twice".to_owned());
            values::release(text);
            values::release(text);
            0
        });
        assert_eq!(reused, 3);
    }

    #[test]
    fn main_needs_a_grant_for_each_capability_it_uses() {
        let meta = Meta::parse(
            r#"{"grants":[],"uses":[{"name":"console","capability":"Console","ops":["print"],"location":{"file":"src/hello.una","start":{"line":4,"col":3},"end":{"line":4,"col":33}}}]}"#,
        )
        .unwrap();
        assert!(meta.sites.is_empty());
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
