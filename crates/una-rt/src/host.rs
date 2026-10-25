//! Host capabilities and traces (`spec/16 §3` and `§4` rule 5): the operations of `std.host`
//! with the interpreter's semantics, grants and path rules (`spec/14 §3`), the instances of
//! capabilities, and the events a trace records for every operation (`spec/11 §5`).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use una_interp::{Event, EventOutcome, FileFault, Grant, NotText, World};

use crate::render::index;
use crate::runtime::with_state;
use crate::sched::{self, Pool};
use crate::values;

/// How an operation's argument, result or failure is shown: its type, and the level that
/// redacts it when its label is above `public`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    /// The index of its type.
    pub ty: usize,
    /// The level, when it is redacted.
    pub redacted: Option<String>,
}

/// An operation of the program, by the index the generated code passes to
/// [`event`]: what its events record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpSite {
    /// The capability.
    pub capability: String,
    /// The operation.
    pub op: String,
    /// Its parameters' names, as traces key the arguments.
    pub params: Vec<String>,
    /// Its arguments.
    pub args: Vec<Shown>,
    /// Its result.
    pub result: Shown,
    /// The type of its failures, when it can fail.
    pub error: Option<usize>,
}

impl OpSite {
    /// An operation of the embedded description's `ops` array.
    pub fn parse(value: &Value) -> Option<OpSite> {
        let strings = |key: &str| {
            value
                .get(key)?
                .as_array()?
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        };
        let shown = |value: &Value| {
            Some(Shown {
                ty: index(value, "type")?,
                redacted: value
                    .get("redacted")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        };
        Some(OpSite {
            capability: value.get("capability")?.as_str()?.to_owned(),
            op: value.get("op")?.as_str()?.to_owned(),
            params: strings("params")?,
            args: value
                .get("args")?
                .as_array()?
                .iter()
                .map(shown)
                .collect::<Option<Vec<_>>>()?,
            result: shown(value.get("result")?)?,
            error: index(value, "error"),
        })
    }
}

/// The machine a native executable runs on, as `una run` reaches it.
struct Machine;

impl World for Machine {
    fn print(&mut self, line: &str) {
        print(line);
    }

    fn now(&mut self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            })
    }

    fn env_var(&mut self, name: &str) -> Result<Option<String>, NotText> {
        if name.is_empty() || name.contains(['=', '\0']) {
            return Ok(None);
        }
        // The executable is the process, as `una`'s binary is for `una run`. A value that is
        // not valid UTF-8 is `NotText`, which aborts the run (S-16, ADR 0086).
        #[allow(clippy::disallowed_methods)]
        match std::env::var_os(name) {
            Some(value) => value.into_string().map(Some).map_err(|_| NotText),
            None => Ok(None),
        }
    }

    fn read_file(&mut self, path: &Path) -> Result<String, FileFault> {
        // Content that is not valid UTF-8 is `NotText`, which aborts the run, not `Io`, which
        // the program could branch on (S-16, ADR 0086).
        let bytes = std::fs::read(path).map_err(file_fault)?;
        String::from_utf8(bytes).map_err(|_| FileFault::NotText)
    }

    fn write_file(&mut self, path: &Path, content: &str) -> Result<(), FileFault> {
        // `spec/14 §3`, ADR 0038 D9 rule 3, as `una run` writes: a file that does not exist is
        // created with `create_new` (`O_CREAT | O_EXCL`), which fails on a symbolic link,
        // dangling or not, so a write never creates a file through a link. An existing file is
        // truncated and never created.
        let exists = std::fs::symlink_metadata(path).is_ok();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(exists)
            .create_new(!exists)
            .open(path)
            .map_err(file_fault)?;
        file.write_all(content.as_bytes()).map_err(file_fault)
    }

    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }

    fn is_link(&mut self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    }
}

fn file_fault(error: io::Error) -> FileFault {
    if error.kind() == io::ErrorKind::NotFound {
        FileFault::NotFound
    } else {
        FileFault::Io
    }
}

/// Writes `line` and a newline to stdout, keeping the first failed write to report at the end.
fn print(line: &str) {
    record_write(write_line(line));
}

/// Writes `line` and a newline to stdout.
fn write_line(line: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{line}").and_then(|()| stdout.flush())
}

/// Keeps the first failed write of the run or the task, to report at the end.
fn record_write(written: io::Result<()>) {
    if let Err(error) = written {
        with_state(|state| {
            if state.failed_write.is_none() {
                state.failed_write = Some(error);
            }
        });
    }
}

/// Records that the host operation in progress failed, with a failure the caller then reads
/// as its value.
pub(crate) fn failed(word: i64) -> i64 {
    // A failure is a value of an error kind, a sum: a handle (even, not 0) or a variant without
    // fields, `(index << 1) | 1` with `index >= 0`. So it is never `-1`, the word of a stopped
    // task in the generated code (`una_codegen::lower::STOPPED`).
    debug_assert_ne!(word, -1, "a failure word equal to the stop of a task");
    with_state(|state| state.host_failed = true);
    word
}

/// Whether the last host operation failed, its value being then the failure, and clears it: 1
/// when it failed, 2 when it was interrupted while it waited, its task cancelled, which the
/// caller passes on as the stop of the task (ADR 0117 rule 4), and 0 otherwise.
pub fn host_failed() -> i64 {
    with_state(|state| {
        let failed = std::mem::take(&mut state.host_failed);
        if state.interrupted {
            2
        } else {
            i64::from(failed)
        }
    })
    .unwrap_or(0)
}

/// The word an interrupted operation returns: the stop of a task in the generated code
/// (`una_codegen::lower::STOPPED`), which its caller passes on by the failure path.
const INTERRUPTED: i64 = -1;

/// The variants of `FileError` and `KeyError` (`spec/14 §3`), as words.
const NOT_FOUND: i64 = 1;
const DENIED: i64 = 3;
const IO: i64 = 5;
const UNKNOWN_KEY: i64 = 1;

/// `Console.print(line)`. In a task, a line goes out on the worker when stdout can take it
/// without waiting, and on a helper of the file pool otherwise, while the task parks (ADR 0117
/// rule 3).
pub fn console_print(line: i64) -> i64 {
    let line = values::text_value(line);
    if sched::in_task() && !una_sys::io::writable(1) {
        record_write(sched::blocking(Pool::Files, move || write_line(&line)));
    } else {
        print(&line);
    }
    0
}

/// `Clock.now()`: milliseconds since the Unix epoch; under virtual time, the milliseconds of
/// the run's virtual clock, from 0 at its start (ADR 0116 rule 19, `spec/26 §8`).
pub fn clock_now() -> i64 {
    let clock = crate::runtime::clock();
    let now = if clock.is_virtual() {
        i64::try_from(clock.now()).unwrap_or(i64::MAX)
    } else {
        Machine.now()
    };
    values::int_word(&num_bigint::BigInt::from(now))
}

/// `Env.get(name)` at the call `site`: the variable, or `None`. A value that is not valid UTF-8
/// aborts the run, or the task, with U6005 at the operation (S-16, ADR 0086).
pub fn env_get(site: i64, name: i64) -> i64 {
    match Machine.env_var(&values::text_value(name)) {
        Ok(Some(value)) => values::some_owned(values::text_word(value)),
        Ok(None) => values::NONE,
        Err(NotText) => not_text(
            site,
            "the value of this environment variable is not valid UTF-8",
        ),
    }
}

/// Aborts the run, or the task, with U6005 at the call `site` (S-16, ADR 0086); a task's caller
/// stops it ([`crate::runtime::stopped`]).
fn not_text(site: i64, message: &str) -> i64 {
    crate::runtime::abort_at(
        usize::try_from(site).unwrap_or(usize::MAX),
        una_diag::Code::U6005,
        message.to_owned(),
    )
}

/// The path that `relative` names under the first `Files` grant that allows the access, as
/// `una run` confines it (`spec/14 §3`).
fn confined(grants: &[Grant], relative: &str, writing: bool) -> Option<PathBuf> {
    grants.iter().find_map(|grant| match grant {
        Grant::Files {
            prefix,
            read,
            write,
        } if (if writing { *write } else { *read }) => {
            una_interp::confine(&mut Machine, prefix, relative)
        }
        _ => None,
    })
}

/// The failure of a file operation. `NotText` never comes from a write, whose content is a
/// text, and a read aborts on it before it gets here.
fn file_failure(fault: FileFault) -> i64 {
    failed(match fault {
        FileFault::NotFound => NOT_FOUND,
        FileFault::Io | FileFault::NotText => IO,
    })
}

/// `Files.read(path)` at the call `site`. Content that is not valid UTF-8 aborts the run, or the
/// task, with U6005 at the operation (S-16, ADR 0086).
///
/// In a task, the confinement and the read run on a helper of the file pool while the task
/// parks, since either may block (ADR 0117 rule 3).
pub fn files_read(site: i64, path: i64) -> i64 {
    let relative = values::text_value(path);
    let grants = run_grants();
    let read = sched::blocking(Pool::Files, move || {
        let target = confined(&grants, &relative, false)?;
        Some(Machine.read_file(&target))
    });
    match read {
        None => failed(DENIED),
        Some(Ok(text)) => values::text_word(text),
        Some(Err(FileFault::NotText)) => {
            not_text(site, "the content of this file is not valid UTF-8")
        }
        Some(Err(fault)) => file_failure(fault),
    }
}

/// The grants of the run, which every task reads.
fn run_grants() -> Arc<Vec<Grant>> {
    with_state(|state| Arc::clone(&state.grants)).unwrap_or_default()
}

/// `Files.write(path, content)`.
///
/// In a task, it runs on a helper of the file pool while the task parks, as a read does.
pub fn files_write(path: i64, content: i64) -> i64 {
    let relative = values::text_value(path);
    let content = values::text_value(content);
    let grants = run_grants();
    let written = sched::blocking(Pool::Files, move || {
        let target = confined(&grants, &relative, true)?;
        Some(Machine.write_file(&target, &content))
    });
    match written {
        None => failed(DENIED),
        Some(Ok(())) => 0,
        Some(Err(fault)) => file_failure(fault),
    }
}

/// `Keys.hmac(key_name, data)`.
pub fn keys_hmac(key_name: i64, data: i64) -> i64 {
    let key = with_state(|state| state.keys.get(&values::text_value(key_name)).cloned()).flatten();
    match key {
        Some(key) => values::hashed_word(una_interp::hmac_sha256(
            &key,
            values::text_value(data).as_bytes(),
        )),
        None => failed(UNKNOWN_KEY),
    }
}

/// A new host instance of `capability`, with the id `host:<capability>`.
pub fn host_instance(capability: i64) -> i64 {
    values::instance(format!("host:{}", values::text_value(capability)))
}

/// A new instance of the handler named `name`, installed by `with`: its id is
/// `with:<name>#<n>`, the `n`th such instance of the run (`runtime::State::install_id`).
pub fn handler_instance(name: i64) -> i64 {
    let name = values::text_value(name);
    let id = with_state(|state| {
        // A streaming task, the root or a started one, counts run-wide (`spec/27 §7`).
        let count = if state.streaming {
            let mut counts = state
                .shared
                .installs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let count = counts.entry(name.clone()).or_insert(0);
            *count += 1;
            *count
        } else {
            let count = state.installs.entry(name.clone()).or_insert(0);
            *count += 1;
            *count
        };
        state.install_id(&name, count)
    })
    .unwrap_or_else(|| format!("with:{name}#1"));
    values::instance(id)
}

/// Starts the event of operation `site` on `instance`, when the run records a trace: its
/// position in the trace, or -1. Its outcome is `Aborted` until [`event_end`].
pub fn event(site: i64, instance: i64) -> i64 {
    let id = values::instance_id(instance);
    with_state(|state| {
        let op = usize::try_from(site).ok().and_then(|s| state.ops.get(s))?;
        let (capability, name) = (op.capability.clone(), op.op.clone());
        let task = state.path.clone();
        let trace = state.trace.as_mut()?;
        let slot = trace.len();
        trace.push(Event {
            seq: slot + 1,
            task,
            instance: id,
            capability,
            op: name,
            args: Vec::new(),
            outcome: EventOutcome::Aborted,
        });
        // Open until it ends: no event is written before it, so its slot stays (ADR 0117 rule
        // 10).
        state.open_events += 1;
        i64::try_from(slot).ok()
    })
    .flatten()
    .unwrap_or(-1)
}

/// Records argument `index` of the event at `slot`, of operation `site`.
pub fn event_arg(site: i64, slot: i64, index: i64, value: i64) {
    let (Ok(site), Ok(slot), Ok(index)) = (
        usize::try_from(site),
        usize::try_from(slot),
        usize::try_from(index),
    ) else {
        return;
    };
    with_state(|state| {
        let Some(op) = state.ops.get(site) else {
            return;
        };
        let name = op
            .params
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("arg{index}"));
        let shown = match op.args.get(index) {
            Some(arg) => state.types.redacted(arg.ty, value, arg.redacted.as_deref()),
            None => "?".to_owned(),
        };
        if let Some(event) = state.trace.as_mut().and_then(|trace| trace.get_mut(slot)) {
            event.args.push((name, shown));
        }
    });
}

/// Ends the event at `slot`, of operation `site`, with its value, or its failure when `failed`
/// is 1. A task that stopped at or in the operation (`failed` 2) leaves the event as it was,
/// `aborted`, unless the operation was interrupted while it waited, which ends it
/// `interrupted` (ADR 0117 rules 4 and 10).
pub fn event_end(site: i64, slot: i64, value: i64, failed: i64) {
    let interrupted =
        failed == 2 && with_state(|state| std::mem::take(&mut state.interrupted)).unwrap_or(false);
    let (Ok(site), Ok(slot)) = (usize::try_from(site), usize::try_from(slot)) else {
        return;
    };
    end_event(site, slot, value, (failed, interrupted));
    // The event is complete: a streaming task writes what it holds once no other is open.
    with_state(|state| {
        state.open_events = state.open_events.saturating_sub(1);
        state.flush();
    });
}

/// Fills in the outcome of the event at `slot`, of operation `site`, by how it ended.
fn end_event(site: usize, slot: usize, value: i64, (failed, interrupted): (i64, bool)) {
    if interrupted {
        with_state(|state| {
            if let Some(event) = state.trace.as_mut().and_then(|trace| trace.get_mut(slot)) {
                event.outcome = EventOutcome::Interrupted;
            }
        });
        return;
    }
    if failed == 2 {
        return;
    }
    with_state(|state| {
        let Some(op) = state.ops.get(site) else {
            return;
        };
        let outcome = if failed == 0 {
            EventOutcome::Ok(state.types.redacted(
                op.result.ty,
                value,
                op.result.redacted.as_deref(),
            ))
        } else {
            let ty = op.error.unwrap_or(usize::MAX);
            EventOutcome::Fail {
                variant: state.types.variant_name(ty, value),
                value: state.types.render(ty, value),
            }
        };
        if let Some(event) = state.trace.as_mut().and_then(|trace| trace.get_mut(slot)) {
            event.outcome = outcome;
        }
    });
}

/// The index of an `HttpError` variant in its declaration order (`spec/24 §1`), which the
/// formal track's model uses as the failure's code (#470); a test pins it to the prelude.
fn http_error_index(error: una_http::client::Error) -> i64 {
    use una_http::client::Error;
    match error {
        Error::Denied => 0,
        Error::BadUrl => 1,
        Error::BadRequest => 2,
        Error::Unreachable => 3,
        Error::TimedOut => 4,
        Error::TooLarge => 5,
        Error::Tls => 6,
        Error::Protocol => 7,
        // Edition 0.4 only, after the eight variants of `spec/24 §1` (ADR 0116 rule 12, ADR
        // 0117 rule 8).
        Error::Interrupted => 8,
        Error::RateLimited => 9,
        // Not a failure: `http` aborts before.
        Error::HeaderValue => 2,
    }
}

/// The interruption of a request on its helper (ADR 0116 rule 12, amendment P9, #1148). It
/// always waits, so the request's socket is non-blocking: every read, write and step of the TLS
/// handshake that would block comes back here, and the client looks at the cancellation of the
/// request's task between them (`una_http::client::send_covered`), so an interrupted exchange
/// stops at its next I/O step. A host stop interrupts it only in the body of a `for ever`
/// (`stop`), through the wake pipe, as `una_interp::stopping::OpInterrupt` does.
///
/// **No polling for a cancellation.** Outside a `for ever` body a wait is one `poll` of the
/// socket for the whole time left: the abandon of a cancelled task shuts the socket down, which
/// makes it ready and ends the wait at once, and the deadline ends it otherwise. Inside a `for
/// ever` body a host stop also ends it: the wake pipe tells it at once, and, as for every
/// interruptible wait of ADR 0116 rule 12, the wait looks at the run's stop state every
/// `stopping::SLICE` (100 ms) for a stop the pipe cannot tell, such as one after another run of
/// the process made the pipe readable. That slice is the host stop's, not a cancellation's.
/// O(1) per `poll`.
struct HelperWait {
    stop: Option<&'static una_interp::stopping::RunStop>,
    /// Whether the wake pipe was found readable without a stop of this run, as after another
    /// run of the process: the wait then looks at the run's requests between slices only.
    stale: std::cell::Cell<bool>,
}

impl una_http::client::Interrupt for HelperWait {
    fn waits(&self) -> bool {
        true
    }

    fn stopped(&self) -> bool {
        self.stop.is_some_and(|stop| stop.stopped())
    }

    fn wait(
        &self,
        socket: std::os::fd::RawFd,
        write: bool,
        timeout: std::time::Duration,
    ) -> una_http::client::Waited {
        use una_http::client::Waited;
        use una_sys::signals::Readiness;
        let end = std::time::Instant::now().checked_add(timeout);
        loop {
            if self.stopped() {
                return Waited::Stopped;
            }
            let left = end.map_or(timeout, |end| {
                end.saturating_duration_since(std::time::Instant::now())
            });
            if left.is_zero() {
                return Waited::TimedOut;
            }
            let pipe = self.stop.is_some() && !self.stale.get();
            let slice = if self.stop.is_some() {
                left.min(una_interp::stopping::SLICE)
            } else {
                left
            };
            match una_sys::signals::wait(Some(socket), write, pipe, slice) {
                Readiness::Ready => return Waited::Ready,
                Readiness::Failed => return Waited::Failed,
                Readiness::TimedOut => {}
                Readiness::Woken if self.stopped() => return Waited::Stopped,
                Readiness::Woken => self.stale.set(true),
            }
        }
    }
}

/// An operation of `Http` (`spec/24`), by the client that the interpreter shares: the response
/// as a `Response` record, or an `HttpError`, which `una_rt_host_failed` then reports.
fn http(
    site: i64,
    method: una_http::grant::Method,
    url: i64,
    headers: i64,
    body: Option<i64>,
    key_name: i64,
) -> i64 {
    use una_http::client::{Cover, Error, Request, send_covered};
    // The variants of `HttpError`, in declaration order, as words.
    let variant = |error: Error| (http_error_index(error) << 1) | 1;
    let url = values::text_value(url);
    let headers: Vec<(String, String)> = values::list_items(headers)
        .into_iter()
        .map(|header| {
            (
                values::text_value(values::field(header, 0)),
                values::text_value(values::field(header, 1)),
            )
        })
        .collect();
    let body = body.map(values::text_value).unwrap_or_default();
    // `Some(name)` is the variant at position 0 of `Option`.
    let key_name =
        (values::tag(key_name) == 0).then(|| values::text_value(values::field(key_name, 0)));
    let bearer = match key_name {
        Some(name) => match with_state(|state| state.keys.get(&name).cloned()).flatten() {
            Some(key) => Some(key),
            None => return failed(variant(Error::Denied)),
        },
        None => None,
    };
    // The run's `Http` grants with the state of their limits, which every task shares (ADR 0117
    // rule 8): a state-less caller gets a governor of no grants, which allows nothing.
    let Some((governor, settings)) =
        with_state(|state| (Arc::clone(&state.http_limits), state.http))
    else {
        return failed(variant(Error::Denied));
    };
    // The deadline of a `for ever within` iteration (`spec/26 §7`): past it, the request fails
    // at once with `TimedOut`; before it, it waits no longer than the time left. In the body of a `for ever`, a host stop interrupts the request
    // too (ADR 0116 rule 12): whichever comes first cuts the wait, in the same poll.
    //
    // Under virtual time, a network wait takes no virtual time (ADR 0116 rule 19), so only a
    // deadline the virtual clock has already reached cuts the request, at once.
    let mut settings = settings;
    let mut cut = None;
    let covering = crate::deadline::left();
    // The covering deadline on the run's clock, which a wait at a limit reaches at most: under
    // virtual time, a wait for a refill moves the clock to it (ADR 0116 rule 19).
    let cover = covering.map(|(left, bound)| {
        let at = crate::runtime::clock()
            .now()
            .saturating_add(u64::try_from(left.as_millis()).unwrap_or(u64::MAX));
        (at, bound)
    });
    let cover_at = cover.map(|(at, _)| at);
    // The covering deadline itself, as an instant of the monotonic clock, under the real clock
    // (#1148): the helper's exchange never runs past it, however long the request waited for a
    // helper, whereas a duration would count from the helper's start. Under virtual time a
    // network wait takes no virtual time, so only the run's clock bounds it.
    let cover_instant = covering
        .filter(|_| !crate::runtime::clock().is_virtual())
        .and_then(|(left, _)| std::time::Instant::now().checked_add(left));
    let covered = Cover {
        at: cover_at,
        instant: cover_instant,
    };
    if let Some((left, bound)) = covering
        && left < settings.timeout
        && (left.is_zero() || !crate::runtime::clock().is_virtual())
    {
        settings.timeout = left;
        cut = Some(bound);
    }
    let stop = crate::runtime::op_interrupt();
    let sent = if cut.is_some() && settings.timeout.is_zero() {
        Err(Error::TimedOut)
    } else {
        // In a task, the request runs on a helper of the I/O pool while the task parks (ADR 0117
        // rule 3); only owned data, the run's governor of limits and its static stop state
        // cross.
        let abandon = una_http::limit::Abandon::new();
        let waiting = abandon.clone();
        let job = move || {
            let request = Request {
                method,
                url: &url,
                headers: &headers,
                body: body.as_bytes(),
                bearer: bearer.as_deref(),
            };
            // Non-blocking on every path (#1148), interrupted by a host stop in a `for ever`
            // body only (ADR 0116 rule 12).
            let interrupt = HelperWait {
                stop,
                stale: std::cell::Cell::new(false),
            };
            send_covered(
                &request, &governor, &settings, &interrupt, &waiting, covered,
            )
        };
        // A request is interruptible (ADR 0117 rule 4): a cancellation that comes while it
        // waits ends it `interrupted`, and the task stops. The cancellation reaches the client
        // through `abandon` (ADR 0116 P9, #1148): a request still waiting at a limit leaves its
        // queues, never sent (rule 8), and one whose exchange has begun stops at its next I/O
        // step, its socket shut down, so its helper is free again within that step. What it
        // sent is an effect in doubt.
        let entries = crate::runtime::interruptible_entries();
        let abandoned: crate::interrupt::Wake = Arc::new(move || abandon.abandon());
        match sched::blocking_interruptible(Pool::Io, &entries, Some(abandoned), job) {
            Some(sent) => sent,
            None => {
                with_state(|state| state.interrupted = true);
                return INTERRUPTED;
            }
        }
    };
    // Under the real clock, a request whose exchange reached the covering instant was cut by
    // it, even when its time-out was the shorter at the start (#1148).
    if cut.is_none()
        && sent == Err(Error::TimedOut)
        && let (Some((_, bound)), Some(at)) = (covering, cover_instant)
        && std::time::Instant::now() >= at
    {
        cut = Some(bound);
    }
    // Under virtual time, a wait at a limit that moved the clock to the covering deadline was cut
    // by it, as a request is under the real clock.
    if cut.is_none()
        && sent == Err(Error::TimedOut)
        && crate::runtime::clock().is_virtual()
        && let Some((at, bound)) = cover
        && crate::runtime::clock().now() >= at
    {
        cut = Some(bound);
    }
    // A `for ever within` iteration observes its deadline at the operation; a `within` at its
    // next check point, or when its expression returns (`spec/27 §5`).
    if cut == Some(crate::deadline::Bound::Ever) && sent == Err(Error::TimedOut) {
        crate::deadline::timed_out();
    }
    match sent {
        Ok(response) => {
            let list = values::list();
            for (name, value) in response.headers {
                let header = values::data(0);
                let (name, value) = (values::text_word(name), values::text_word(value));
                values::push(header, name);
                values::push(header, value);
                values::release(name);
                values::release(value);
                values::push(list, header);
                values::release(header);
            }
            let record = values::data(0);
            let status = values::int_word(&num_bigint::BigInt::from(response.status));
            let body = values::text_word(response.body);
            for field in [status, list, body] {
                values::push(record, field);
                values::release(field);
            }
            record
        }
        // The run, or the task, aborts with U6004 at the operation; a task's caller
        // stops it ([`crate::runtime::stopped`]).
        Err(Error::HeaderValue) => crate::runtime::abort_at(
            usize::try_from(site).unwrap_or(usize::MAX),
            una_diag::Code::U6004,
            "a header value contains a control character".to_owned(),
        ),
        Err(error) => failed(variant(error)),
    }
}

/// `Http.get(url, headers, key_name)`, at the call `site`.
pub fn http_get(site: i64, url: i64, headers: i64, key_name: i64) -> i64 {
    http(
        site,
        una_http::grant::Method::Get,
        url,
        headers,
        None,
        key_name,
    )
}

/// `Http.post(url, headers, body, key_name)`, at the call `site`.
pub fn http_post(site: i64, url: i64, headers: i64, body: i64, key_name: i64) -> i64 {
    http(
        site,
        una_http::grant::Method::Post,
        url,
        headers,
        Some(body),
        key_name,
    )
}

/// `Http.put(url, headers, body, key_name)`, at the call `site`.
pub fn http_put(site: i64, url: i64, headers: i64, body: i64, key_name: i64) -> i64 {
    http(
        site,
        una_http::grant::Method::Put,
        url,
        headers,
        Some(body),
        key_name,
    )
}

/// `Http.delete(url, headers, key_name)`, at the call `site`.
pub fn http_delete(site: i64, url: i64, headers: i64, key_name: i64) -> i64 {
    http(
        site,
        una_http::grant::Method::Delete,
        url,
        headers,
        None,
        key_name,
    )
}

#[cfg(test)]
mod tests {
    use super::http_error_index;
    use una_http::client::Error;

    #[test]
    fn http_error_codes_follow_the_declaration_order() {
        // #470: the formal track's model reads a request's failure as the index of its
        // `HttpError` variant. The runtime's indices must be the prelude's declaration order,
        // which `spec/24 §1` states, so that the two cannot drift apart silently.
        let prelude = include_str!("../../una-check/src/std/prelude_0_2.una");
        let line = prelude
            .lines()
            .find(|line| line.starts_with("pub kind HttpError ="))
            .expect("the prelude declares HttpError");
        let declared: Vec<&str> = line["pub kind HttpError =".len()..]
            .split('|')
            .map(str::trim)
            .collect();
        let cases = [
            (Error::Denied, "Denied"),
            (Error::BadUrl, "BadUrl"),
            (Error::BadRequest, "BadRequest"),
            (Error::Unreachable, "Unreachable"),
            (Error::TimedOut, "TimedOut"),
            (Error::TooLarge, "TooLarge"),
            (Error::Tls, "Tls"),
            (Error::Protocol, "Protocol"),
        ];
        // Edition 0.4 adds `Interrupted`, then `RateLimited` (ADR 0116 rule 12, ADR 0117 rule
        // 8).
        assert_eq!(http_error_index(Error::Interrupted), 8);
        assert_eq!(http_error_index(Error::RateLimited), 9);
        assert_eq!(declared.len(), cases.len(), "{declared:?}");
        for (error, name) in cases {
            let index = usize::try_from(http_error_index(error)).expect("an index");
            assert_eq!(declared[index], name, "{name} is at {index}");
        }
    }
}
