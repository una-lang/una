//! Host capabilities and traces (`spec/16 §3` and `§4` rule 5): the operations of `std.host`
//! with the interpreter's semantics, grants and path rules (`spec/14 §3`), the instances of
//! capabilities, and the events a trace records for every operation (`spec/11 §5`).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use una_core::bigint::{BigInt, Sign};
use una_interp::{Confined, Event, EventOutcome, FileFault, Grant, NotText, World};

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
    /// The level its outcome is redacted at, when its outcome label is above `public` (S-45,
    /// #1552, `spec/11 §5`): the event's outcome is then `redacted`.
    pub outcome: Option<String>,
    /// The type of its failures, when it can fail.
    pub error: Option<usize>,
    /// The index of its position among the program's sites, where an abort at the operation is
    /// reported, as the interpreter reports it (`spec/16 §3` rule 2); none for an operation
    /// that the runtime performs on its own, such as those of a `serve` section's driver.
    pub site: Option<usize>,
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
            // Required, `null` when the outcome is in clear: a description without it fails
            // to load, never an outcome shown in clear by default.
            outcome: match value.get("outcome")? {
                Value::Null => None,
                level => Some(level.as_str()?.to_owned()),
            },
            error: index(value, "error"),
            site: index(value, "site"),
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

    /// One `lstat` per component of a confined path (#1343).
    fn step(&mut self, directory: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
        una_interp::disk_step(directory, name)
    }

    /// Only a component that does not exist is absence (#1380), which fails the operation
    /// without a path (#1459).
    fn absent(&mut self, path: &Path) -> Option<una_interp::Absence> {
        una_interp::disk_absent(path)
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
        // An operation that stopped its task in progress, its event left `aborted`: a `next` of
        // `HttpServer` once the run aborted (`crate::serve`).
        let stopped = std::mem::take(&mut state.op_stopped);
        if state.interrupted || stopped {
            2
        } else {
            i64::from(failed)
        }
    })
    .unwrap_or(0)
}

/// Runs `job`, an interruptible operation of `Http` or of `std.limit`, on a helper of the I/O
/// pool while the task parks ([`sched::blocking_interruptible`]), `abandoned` raised when a
/// cancellation of `entries` interrupts it. `None` when such a cancellation interrupted it: the
/// operation ends `interrupted`. A deadline's cut of the task's `tasks` statement is not one
/// (`spec/27 §2` rule 4, #1266): a job that has begun is waited for, and one still queued, which
/// the cut takes off the queue unsent, fails with `TimedOut`, the failure that the same
/// deadline gives a job that runs, so the outcome never depends on the pool's load. The cut
/// comes once the `within`'s timer has fired, at or after the deadline that covers the job.
fn on_helper<T: Send + 'static>(
    entries: &[crate::runtime::Cancel],
    abandoned: crate::interrupt::Wake,
    job: impl FnOnce() -> Result<T, una_http::client::Error> + Send + 'static,
) -> Option<Result<T, una_http::client::Error>> {
    match sched::blocking_interruptible(Pool::Io, entries, Some(abandoned), job) {
        Some(done) => Some(done),
        None if crate::interrupt::cut_only(entries) => Some(Err(una_http::client::Error::TimedOut)),
        None => None,
    }
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
    values::int_word(&una_core::bigint::BigInt::from(now))
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

/// What confinement decides for `relative` under the first `Files` grant of the access that
/// does not deny it, as `una run` confines it (`spec/14 §3`).
fn confined(world: &mut dyn World, grants: &[Grant], relative: &str, writing: bool) -> Confined {
    grants
        .iter()
        .filter_map(|grant| match grant {
            Grant::Files {
                prefix,
                read,
                write,
            } if (if writing { *write } else { *read }) => Some(prefix),
            _ => None,
        })
        .map(|prefix| una_interp::confine(world, prefix, relative))
        .find(|outcome| *outcome != Confined::Denied)
        .unwrap_or(Confined::Denied)
}

/// A `Files` operation on `relative` in `world`, within `grants`: `None` when it is `Denied`,
/// and otherwise what `operation` gives on the confined path. An intermediate directory that
/// confinement confirmed absent fails the operation here, and the world is given no path
/// (#1459).
fn on_confined<T>(
    world: &mut dyn World,
    grants: &[Grant],
    relative: &str,
    writing: bool,
    operation: impl FnOnce(&mut dyn World, &Path) -> Result<T, FileFault>,
) -> Option<Result<T, FileFault>> {
    match confined(world, grants, relative, writing) {
        Confined::Denied => None,
        Confined::Absent(absence) => Some(Err(absence.fault())),
        Confined::Path(target) => Some(operation(world, &target)),
    }
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
    files_read_in(Machine, site, path)
}

/// [`files_read`] in `world`, which is the machine for every run: the whole operation, so that
/// a test gives it a world that keeps what it is asked and reads the paths a read reaches the
/// world with (#2138). `world` goes to the helper with the job, as the machine does.
fn files_read_in(mut world: impl World + Send + 'static, site: i64, path: i64) -> i64 {
    let relative = values::text_value(path);
    let grants = run_grants();
    let read = sched::blocking(Pool::Files, move || {
        on_confined(&mut world, &grants, &relative, false, |world, target| {
            world.read_file(target)
        })
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
    files_write_in(Machine, path, content)
}

/// [`files_write`] in `world`, which is the machine for every run: the whole operation, for
/// the same test as [`files_read_in`].
fn files_write_in(mut world: impl World + Send + 'static, path: i64, content: i64) -> i64 {
    let relative = values::text_value(path);
    let content = values::text_value(content);
    let grants = run_grants();
    let written = sched::blocking(Pool::Files, move || {
        on_confined(&mut world, &grants, &relative, true, |world, target| {
            world.write_file(target, &content)
        })
    });
    match written {
        None => failed(DENIED),
        Some(Ok(())) => 0,
        Some(Err(fault)) => file_failure(fault),
    }
}

/// `Keys.hmac(key_name, data)`.
///
/// The operation itself copies no key out of the run's table (#1481): it takes a reference to
/// the table, a count, and hands the HMAC routine the key where the run holds it. The key's name
/// and the data are read in place in the heap too. The HMAC routine,
/// [`una_interp::hmac_sha256`], holds the padded key, its two pads and its hash state in fixed
/// arrays on its stack and overwrites them with zeros before it returns (#2120), without a
/// volatile write: its documentation says what that rests on and what it does not reach (what
/// the SHA-256 compression function keeps in its own frame, which includes a copy of the
/// key-equivalent hash state, and copies the compiler makes in registers). What still holds key
/// material, unwiped: the run's table for the whole run, the key file's text and its decoding
/// at start-up, and the bearer key of an `Http` request, which the operations of `Http` clone
/// out of the table, since the request crosses to a helper thread, and write into the request
/// they send. Time O(length of the key + length of the data).
pub fn keys_hmac(key_name: i64, data: i64) -> i64 {
    let Some(keys) = with_state(|state| Arc::clone(&state.keys)) else {
        return failed(UNKNOWN_KEY);
    };
    // The HMAC runs under the heap's borrow and does not call back into the heap.
    let mac = values::with_texts(key_name, data, |name, data| {
        keys.get(name)
            .map(|key| una_interp::hmac_sha256(key, data.as_bytes()))
    });
    match mac {
        Some(mac) => values::hashed_word(mac),
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
    instance_named(&values::text_value(name))
}

/// A new instance of the handler `name`, with the next id `with:<name>#<n>` of the task, or of
/// the run in a streaming task. Quiet code counts none of its installations: their ids read
/// `with:<name>#q` (S-41, `spec/26 §6` rule 4).
fn instance_named(name: &str) -> i64 {
    let id = with_state(|state| {
        if state.quiet > 0 {
            return una_interp::concurrency::quiet_install_id(name);
        }
        let name = name.to_owned();
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

/// A count of `limiter`'s arguments: the installation's obligations give `at_most >= 1`,
/// `per >= 1` and `waiting >= 0`; a count beyond 2^64 − 1 saturates, so a larger one never
/// wraps into a smaller limit.
fn limit_count(word: i64) -> u64 {
    let value: BigInt = values::int_value(word);
    u64::try_from(&value).unwrap_or(if value.sign() == Sign::Minus {
        0
    } else {
        u64::MAX
    })
}

/// A new instance of `limiter` of `std.limit` (ADR 0117 rule 9, `spec/27 §10`) from its
/// arguments `at_most`, `per` and `waiting`: one bucket over the run's clock, kept run-wide so
/// that every task that uses the binding shares it, a copy of the instance on another worker
/// included; the instance holds the bucket's index. Its id counts as a handler's,
/// `with:limiter#<n>`.
pub fn limiter_instance(at_most: i64, per: i64, waiting: i64) -> i64 {
    let limiter = Arc::new(una_http::limit::Limiter::new(
        limit_count(at_most),
        limit_count(per),
        limit_count(waiting),
        Arc::new(crate::runtime::clock().clone()),
    ));
    let index = with_state(|state| {
        let mut limiters = state
            .shared
            .limiters
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        limiters.push(limiter);
        limiters.len() - 1
    })
    .unwrap_or(0);
    let instance = instance_named(una_core::LIMITER);
    let index = values::int_word(&BigInt::from(index));
    values::push(instance, index);
    values::release(index);
    instance
}

/// `acquire` of a `limiter` (ADR 0117 rule 9, `spec/27 §10`): takes a token of the bucket of
/// `instance`, waiting for one as a request waits at a limit (`spec/27 §9` rule 4): within the
/// deadlines of the `within`s in progress and of the `for ever within` iteration, on a helper
/// of the I/O pool while the task parks, interrupted by a host stop in the body of a `for ever`,
/// and abandoned when the task is cancelled, which stops the task. Past a covering deadline, or
/// on a host stop, no token is taken and the call fails with `RateLimited`, the one failure
/// `Limited` has; so does a call that finds `waiting` calls already waiting. `unit` on success.
pub fn limit_acquire(_site: i64, instance: i64) -> i64 {
    use una_http::client::Error;
    // `RateLimited`, the first variant of `Limited`, as a word.
    const RATE_LIMITED: i64 = 1;
    let index =
        usize::try_from(values::int_value(values::field(instance, 0))).unwrap_or(usize::MAX);
    let limiter = with_state(|state| {
        state
            .shared
            .limiters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(index)
            .cloned()
    })
    .flatten();
    // An instance no run made allows nothing.
    let Some(limiter) = limiter else {
        return failed(RATE_LIMITED);
    };
    let covering = crate::deadline::left();
    let clock = crate::runtime::clock();
    // The covering deadline on the run's clock, which the wait reaches at most, in its first
    // whole millisecond (#1355); on the wall clock the same time, unless time is virtual, where
    // only the run's clock bounds it.
    let until = covering.map_or(u64::MAX, |covering| covering.at_millis());
    let wall = Instant::now();
    let deadline = match covering {
        Some(covering) if !clock.is_virtual() => wall.checked_add(covering.left),
        _ => wall.checked_add(Duration::from_secs(365 * 24 * 3600)),
    }
    .unwrap_or(wall);
    let stop = crate::runtime::op_interrupt();
    let abandon = una_http::limit::Abandon::new();
    let waiting = abandon.clone();
    let job = move || {
        let acquired = match stop {
            Some(stop) => limiter.acquire((deadline, until), &stop.interrupt(), &waiting),
            None => limiter.acquire(
                (deadline, until),
                &una_http::client::Uninterrupted,
                &waiting,
            ),
        };
        // Only a covering deadline ends the wait with a time-out, and the call returns once the
        // run's clock has reached it, on the helper, never on a worker (`spec/27 §5` rule 6,
        // #1355).
        if acquired == Err(Error::TimedOut)
            && let Some(covering) = covering
        {
            covering.reach();
        }
        acquired
    };
    let entries = crate::runtime::interruptible_entries();
    let abandoned: crate::interrupt::Wake = Arc::new(move || abandon.abandon());
    // A deadline's cut of the task's `tasks` statement that takes a call still queued for a
    // helper makes it fail with `RateLimited`, as its deadline makes a call that waits fail,
    // never `interrupted` (`spec/27 §2` rule 4, §10, #1266).
    let acquired = match on_helper(&entries, abandoned, job) {
        Some(acquired) => acquired,
        None => {
            with_state(|state| state.interrupted = true);
            return INTERRUPTED;
        }
    };
    match acquired {
        Ok(()) => 0,
        Err(Error::TimedOut) => {
            // A `for ever within` iteration observes its deadline at the operation.
            if covering.map(|covering| covering.bound) == Some(crate::deadline::Bound::Ever) {
                crate::deadline::timed_out();
            }
            failed(RATE_LIMITED)
        }
        Err(_) => failed(RATE_LIMITED),
    }
}

/// Starts the event of operation `site` on `instance`, when the run records a trace: its
/// position in the trace, or -1. Its outcome is `Aborted` until [`event_end`]. An operation
/// performed in quiet code has none, and takes no `seq`, so neither the number of events nor
/// the `seq` of a later one shows whether it ran (S-42, #1515, `spec/26 §6` rule 4); code
/// quiet by a timing floor alone keeps it (`spec/27 §5` rule 7). An instance that quiet code
/// installed, `with:<h>#q`, starts none either: it is in scope only in quiet code (#1544), as
/// in the interpreter.
pub fn event(site: i64, instance: i64) -> i64 {
    if with_state(|state| state.hushed > 0).unwrap_or(false) {
        return -1;
    }
    let id = values::instance_id(instance);
    if una_interp::concurrency::is_quiet_install(&id) {
        return -1;
    }
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

/// Whether the event at `slot`, of operation `op`, is one of the host's `HttpServer`, whose
/// values are written in the reduced form of ADR 0123 rule 1.
fn server_event(state: &crate::runtime::State, op: &OpSite, slot: usize) -> bool {
    op.capability == "HttpServer"
        && state
            .trace
            .as_ref()
            .and_then(|trace| trace.get(slot))
            .is_some_and(|event| event.instance == "host:HttpServer")
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
    // An argument whose text does not fit the memory budget aborts the run at the operation,
    // before it starts, and leaves no event (`spec/11 §7`, #1223): `Lowerer::op` stops the
    // task before it.
    let trace = crate::serve::trace();
    let over = with_state(|state| {
        // A task that an earlier argument stopped writes no more of them.
        if state.stopped.is_some() {
            return false;
        }
        let Some(op) = state.ops.get(site) else {
            return false;
        };
        let name = op
            .params
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("arg{index}"));
        // ADR 0123 rule 1 (`spec/31 §1` rule 7): the values of the events of the host's
        // `HttpServer` are redacted by their labels, as every event's are, and are written in
        // the reduced form whatever their labels, under the run's trace key, as in the
        // interpreter. Without the run's engine every argument is written as an `internal`
        // value is, redacted, failing closed.
        let server = server_event(state, op, slot);
        let shown = match (op.args.get(index), server.then_some(trace).flatten()) {
            // The toolchain's tests only: the reduced form without the labels' redaction.
            (Some(arg), Some((key, true))) => state.types.server_written(arg.ty, value, &key),
            (Some(arg), Some((key, false))) => {
                state
                    .types
                    .server_redacted(arg.ty, value, arg.redacted.as_deref(), &key)
            }
            (Some(arg), None) if server => {
                let level = arg.redacted.as_deref().unwrap_or("internal");
                state.types.redacted(arg.ty, value, Some(level))
            }
            (Some(arg), None) => state.types.redacted(arg.ty, value, arg.redacted.as_deref()),
            (None, _) => Some("?".to_owned()),
        };
        let Some(shown) = shown else {
            // The operation does not start, so it has no event and takes no `seq`, as in the
            // interpreter, which writes the arguments before it makes the event (`spec/16 §5`).
            // The event was the last one pushed and nothing was written since: it goes, and no
            // longer holds the trace open.
            debug_assert!(
                state
                    .trace
                    .as_ref()
                    .is_none_or(|trace| trace.len() == slot + 1),
                "an operation's event is the last one while its arguments are written"
            );
            if let Some(trace) = state.trace.as_mut()
                && trace.len() == slot + 1
            {
                trace.pop();
                state.open_events = state.open_events.saturating_sub(1);
                state.flush();
            }
            return true;
        };
        if let Some(event) = state.trace.as_mut().and_then(|trace| trace.get_mut(slot)) {
            event.args.push((name, shown));
        }
        false
    })
    .unwrap_or(false);
    if over {
        op_over_budget(site);
    }
}

/// Aborts the run, or stops its task, with U6003 at the position of operation `site`: a value
/// of its event does not fit the memory budget (`spec/11 §7`, #1223), as the interpreter
/// reports it.
fn op_over_budget(site: usize) {
    let at = with_state(|state| state.ops.get(site).and_then(|op| op.site)).flatten();
    crate::runtime::refuse(at.unwrap_or(usize::MAX), None);
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
    // An outcome whose text does not fit the memory budget aborts the run at the operation,
    // its event left `aborted` (`spec/11 §7`, #1223).
    let trace = crate::serve::trace();
    let over = with_state(|state| {
        let Some(op) = state.ops.get(site) else {
            return false;
        };
        // The outcome of an event of the host's `HttpServer`, in the reduced form of ADR 0123
        // rule 1, as its arguments are.
        let trace = trace.filter(|_| server_event(state, op, slot));
        let revealed = matches!(trace, Some((_, true)));
        // S-45 (#1552): an outcome labeled above `public` is written as `redacted`, from the
        // site's static level alone; no text is built from the value.
        let outcome = if let Some(level) = op.outcome.as_ref().filter(|_| !revealed) {
            Some(EventOutcome::Redacted(level.clone()))
        } else if failed == 0 {
            match trace {
                Some((key, true)) => state.types.server_written(op.result.ty, value, &key),
                Some((key, false)) => state.types.server_redacted(
                    op.result.ty,
                    value,
                    op.result.redacted.as_deref(),
                    &key,
                ),
                None => state
                    .types
                    .redacted(op.result.ty, value, op.result.redacted.as_deref()),
            }
            .map(EventOutcome::Ok)
        } else {
            let ty = op.error.unwrap_or(usize::MAX);
            let variant = state.types.variant_name(ty, value);
            state
                .types
                .render(ty, value)
                .map(|value| EventOutcome::Fail { variant, value })
        };
        let over = outcome.is_none();
        if let Some(event) = state.trace.as_mut().and_then(|trace| trace.get_mut(slot)) {
            event.outcome = outcome.unwrap_or(EventOutcome::Aborted);
        }
        over
    })
    .unwrap_or(false);
    if over {
        op_over_budget(site);
    }
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
        socket: std::os::fd::BorrowedFd<'_>,
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
            match una_sys::signals::wait(
                Some(std::os::fd::AsRawFd::as_raw_fd(&socket)),
                write,
                pipe,
                slice,
            ) {
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
    // Its first whole millisecond, never one before the deadline itself (#1355).
    let cover = covering.map(|covering| (covering.at_millis(), covering.bound));
    let cover_at = cover.map(|(at, _)| at);
    // The covering deadline itself, as an instant of the monotonic clock, under the real clock
    // (#1148): the helper's exchange never runs past it, however long the request waited for a
    // helper, whereas a duration would count from the helper's start. Under virtual time a
    // network wait takes no virtual time, so only the run's clock bounds it.
    let cover_instant = covering
        .filter(|_| !crate::runtime::clock().is_virtual())
        .and_then(|covering| std::time::Instant::now().checked_add(covering.left));
    let covered = Cover {
        at: cover_at,
        instant: cover_instant,
    };
    if let Some(covering) = covering
        && covering.left < settings.timeout
        && (covering.left.is_zero() || !crate::runtime::clock().is_virtual())
    {
        settings.timeout = covering.left;
        cut = Some(covering.bound);
    }
    let stop = crate::runtime::op_interrupt();
    // The deadline that cut the request, if one did: a request it cut returns once the run's
    // clock has reached it, never before, so the `within` it covers sees its deadline passed
    // (`spec/27 §5` rule 6, #1355). The reactor's waits end no earlier than their time-outs.
    let reach = covering.filter(|_| cut.is_some());
    let sent = if cut.is_some() && settings.timeout.is_zero() {
        Err(Error::TimedOut)
    } else if let Some(reactor) = crate::reactor::parks().filter(|_| !governor.limited()) {
        // In a task, the request runs on the task's coroutine, and each wait of its socket
        // parks the task on the reactor, so no thread waits for the server (ADR 0117 rule 3,
        // slice S7); resolution and connection go to a helper of the I/O pool. A cancellation
        // that comes while it waits ends it `interrupted`, its connection closed, and the task
        // stops (rule 4); a deadline's cut of its `tasks` statement does not, and the request
        // ends at that deadline, its own, with `TimedOut`, as on a helper (#1266). Only when no
        // grant carries limits: a wait at a limit parks its thread
        // (rule 8), which must never be a worker, so a run with limits sends on a helper.
        let entries = crate::runtime::interruptible_entries();
        let parked = crate::reactor::Parked::new(reactor, stop, &entries);
        let request = Request {
            method,
            url: &url,
            headers: &headers,
            body: body.as_bytes(),
            bearer: bearer.as_deref(),
        };
        // The run's governor, whose grants carry no limits here, so no hop waits in a queue.
        // The covering deadline bounds the exchange as an instant too (#1148). The parked wait
        // raises the request's abandon on a cancellation, as the helper path does (ADR 0116
        // P9).
        let sent = send_covered(
            &request,
            &governor,
            &settings,
            &parked,
            parked.abandon(),
            covered,
        );
        if parked.cancelled() {
            with_state(|state| state.interrupted = true);
            return INTERRUPTED;
        }
        sent
    } else {
        // Without the reactor, or with limits, in a task, the request runs on a helper of the
        // I/O pool while the task parks (ADR 0117 rule 3); only owned data, the run's governor
        // of limits and its static stop state cross.
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
            let sent = send_covered(
                &request, &governor, &settings, &interrupt, &waiting, covered,
            );
            // On the helper, never on a worker.
            if sent == Err(Error::TimedOut)
                && let Some(covering) = reach
            {
                covering.reach();
            }
            sent
        };
        // A request is interruptible (ADR 0117 rule 4): a cancellation that comes while it
        // waits ends it `interrupted`, and the task stops. The cancellation reaches the client
        // through `abandon` (ADR 0116 P9, #1148): a request still waiting at a limit leaves its
        // queues, never sent (rule 8), and one whose exchange has begun stops at its next I/O
        // step, its socket shut down, so its helper is free again within that step. What it
        // sent is an effect in doubt.
        let entries = crate::runtime::interruptible_entries();
        let abandoned: crate::interrupt::Wake = Arc::new(move || abandon.abandon());
        // A deadline's cut of the task's `tasks` statement that takes a request still queued
        // for a helper makes it fail with `TimedOut`, never sent, the outcome its deadline gives
        // a request that runs, on a helper or on the reactor, never `interrupted` (`spec/27 §2`
        // rule 4, #1266).
        match on_helper(&entries, abandoned, job) {
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
        && let (Some(covering), Some(at)) = (covering, cover_instant)
        && std::time::Instant::now() >= at
    {
        cut = Some(covering.bound);
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
            let status = values::int_word(&una_core::bigint::BigInt::from(response.status));
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

    /// `Keys.hmac` over the key in the run's table (#1481): the HMAC of RFC 4231 test case 2,
    /// an unknown key and a key name that is not a text, with the table left as it was, held by
    /// the run alone and its key unchanged.
    ///
    /// It pins the results of the operation, not the absence of a copy: a copy of the key made
    /// and dropped inside the operation changes nothing a test can read, so this test also
    /// passes on the code that cloned the key. That the key is not cloned is read in the source.
    #[test]
    fn keys_hmac_reads_the_key_in_the_table_of_the_run() {
        use crate::runtime::with_state;
        use crate::values;
        use std::collections::BTreeMap;
        use std::sync::Arc;
        std::thread::spawn(|| {
            crate::runtime::install_test_state_as("0".to_owned(), false, String::new());
            let keys = BTreeMap::from([("phone".to_owned(), b"Jefe".to_vec())]);
            with_state(|state| state.keys = Arc::new(keys));
            let held = || with_state(|state| Arc::strong_count(&state.keys));
            assert_eq!(held(), Some(1));
            let data = values::text_word("what do ya want for nothing?".to_owned());
            let mac = super::keys_hmac(values::text_word("phone".to_owned()), data);
            assert_eq!(super::host_failed(), 0);
            assert_eq!(
                una_interp::hex(&values::bytes(mac)),
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
            );
            // The same operation again gives an equal value, a new object.
            let again = super::keys_hmac(values::text_word("phone".to_owned()), data);
            assert_ne!(mac, again);
            assert_eq!(values::equal_at(0, mac, again), Some(true));
            // An unknown key, and a key name that is not a text (it reads as the empty text).
            for name in [values::text_word("other".to_owned()), mac] {
                assert_eq!(super::keys_hmac(name, data), super::UNKNOWN_KEY);
                assert_eq!(super::host_failed(), 1);
            }
            // The operation holds no reference to the table once it returns, and the key is the
            // one the run loaded.
            assert_eq!(held(), Some(1));
            let key = with_state(|state| state.keys.get("phone").cloned()).flatten();
            assert_eq!(key.as_deref(), Some(&b"Jefe"[..]));
            // The other test cases of RFC 4231 whose data is a text (#2120; cases 3 and 4 have
            // data that is not UTF-8, and are tested on the routine in `una-interp`): 1, 5,
            // whose expected value is the first 128 bits, 6 and 7, with a key longer than the
            // block of the hash.
            let cases: [(&str, Vec<u8>, &str, &str); 4] = [
                (
                    "one",
                    vec![0x0b; 20],
                    "Hi There",
                    "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
                ),
                (
                    "five",
                    vec![0x0c; 20],
                    "Test With Truncation",
                    "a3b6167473100ee06e0c796c2955552b",
                ),
                (
                    "six",
                    vec![0xaa; 131],
                    "Test Using Larger Than Block-Size Key - Hash Key First",
                    "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
                ),
                (
                    "seven",
                    vec![0xaa; 131],
                    "This is a test using a larger than block-size key and a larger than \
                     block-size data. The key needs to be hashed before being used by the HMAC \
                     algorithm.",
                    "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
                ),
            ];
            let keys: BTreeMap<String, Vec<u8>> = cases
                .iter()
                .map(|(name, key, _, _)| ((*name).to_owned(), key.clone()))
                .collect();
            with_state(|state| state.keys = Arc::new(keys.clone()));
            for (name, _, data, expected) in &cases {
                let mac = super::keys_hmac(
                    values::text_word((*name).to_owned()),
                    values::text_word((*data).to_owned()),
                );
                let mac = una_interp::hex(&values::bytes(mac));
                assert_eq!(mac.len(), 64);
                assert_eq!(&mac[..expected.len()], *expected, "test case {name}");
            }
            // The keys in the table are the ones the run loaded, bit for bit.
            assert_eq!(held(), Some(1));
            assert_eq!(with_state(|state| (*state.keys).clone()), Some(keys));
            drop(crate::runtime::leave_task());
        })
        .join()
        .unwrap_or_else(|_| panic!("the test thread panicked"));
    }

    /// The machine, keeping each path that a file operation is given.
    #[cfg(unix)]
    #[derive(Default)]
    struct Operations(Vec<(&'static str, std::path::PathBuf)>);

    #[cfg(unix)]
    impl super::World for Operations {
        fn print(&mut self, _line: &str) {}
        fn now(&mut self) -> i64 {
            0
        }
        fn env_var(&mut self, _name: &str) -> Result<Option<String>, super::NotText> {
            Ok(None)
        }
        fn read_file(&mut self, path: &std::path::Path) -> Result<String, super::FileFault> {
            self.0.push(("read", path.to_path_buf()));
            super::Machine.read_file(path)
        }
        fn write_file(
            &mut self,
            path: &std::path::Path,
            content: &str,
        ) -> Result<(), super::FileFault> {
            self.0.push(("write", path.to_path_buf()));
            super::Machine.write_file(path, content)
        }
        fn canonical(&mut self, path: &std::path::Path) -> Option<std::path::PathBuf> {
            super::Machine.canonical(path)
        }
        fn is_link(&mut self, path: &std::path::Path) -> bool {
            super::Machine.is_link(path)
        }
        fn step(
            &mut self,
            directory: &std::path::Path,
            name: &std::ffi::OsStr,
        ) -> Option<std::path::PathBuf> {
            super::Machine.step(directory, name)
        }
        fn absent(&mut self, path: &std::path::Path) -> Option<una_interp::Absence> {
            super::Machine.absent(path)
        }
    }

    #[cfg(unix)]
    #[test]
    #[cfg_attr(miri, ignore = "uses the file system, which Miri's isolation refuses")]
    #[allow(clippy::unwrap_used)]
    fn an_absent_directory_fails_the_operation_without_a_path_1459() {
        // `spec/14 §3` rule 2, #1459: a read or a write below a directory that confinement
        // confirmed absent fails with `NotFound` (`Io` below a component that is not a
        // directory), and the machine's file operations are given no path; a final component
        // that does not exist is created in the canonical form of its directory.
        use super::{FileFault, Grant, on_confined};
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("una-rt-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("box/sub")).unwrap();
        std::fs::write(dir.join("box/note.txt"), "inside\n").unwrap();
        symlink("sub", dir.join("box/in_dir")).unwrap();
        symlink("box", dir.join("boxlink")).unwrap();
        let canonical = std::fs::canonicalize(dir.join("box")).unwrap();
        let grant = |prefix: &str| Grant::Files {
            prefix: dir.join(prefix),
            read: true,
            write: true,
        };
        let mut world = Operations::default();
        let mut operate = |grants: &[Grant], relative: &str, writing: bool| {
            on_confined(&mut world, grants, relative, writing, |world, target| {
                if writing {
                    world.write_file(target, "entry")
                } else {
                    world.read_file(target).map(|_| ())
                }
            })
        };
        let absent = [
            ("box", "missing/new.txt", FileFault::NotFound),
            ("box", "missing/deeper/new.txt", FileFault::NotFound),
            ("box", "in_dir/missing/new.txt", FileFault::NotFound),
            ("boxlink", "in_dir/missing/a/new.txt", FileFault::NotFound),
            ("box", "note.txt/x/new.txt", FileFault::Io),
            // #2138: a NUL after the missing directory is never reached.
            ("box", "missing/a\0b", FileFault::NotFound),
            ("box", "missing/a\0b/new.txt", FileFault::NotFound),
            ("nothing", "new.txt", FileFault::NotFound),
            ("nothing/deeper", "sub/new.txt", FileFault::NotFound),
        ];
        for (prefix, relative, fault) in absent {
            for writing in [false, true] {
                assert_eq!(
                    operate(&[grant(prefix)], relative, writing),
                    Some(Err(fault)),
                    "{prefix} {relative} {writing}"
                );
            }
        }
        // The first grant that does not deny decides, an absent directory included.
        assert_eq!(
            operate(&[grant("nothing"), grant("box")], "note.txt", false),
            Some(Err(FileFault::NotFound))
        );
        assert_eq!(operate(&[grant("box")], "../note.txt", false), None);
        assert_eq!(world.0, Vec::new(), "no file operation was given a path");
        assert!(!dir.join("box/missing").exists());
        assert!(!dir.join("nothing").exists());
        // A final component that does not exist: the operation gets the canonical directory
        // joined with it, a write creates the file there, and a read fails with `NotFound`.
        let mut world = Operations::default();
        let mut operate = |prefix: &str, relative: &str, writing: bool| {
            on_confined(
                &mut world,
                &[grant(prefix)],
                relative,
                writing,
                |world, target| {
                    if writing {
                        world.write_file(target, "entry")
                    } else {
                        world.read_file(target).map(|_| ())
                    }
                },
            )
        };
        assert_eq!(
            operate("boxlink", "in_dir/gone.txt", false),
            Some(Err(FileFault::NotFound))
        );
        assert_eq!(operate("boxlink", "in_dir/new.txt", true), Some(Ok(())));
        assert_eq!(operate("box", "in_dir/new.txt", false), Some(Ok(())));
        assert_eq!(
            world.0,
            vec![
                ("read", canonical.join("sub/gone.txt")),
                ("write", canonical.join("sub/new.txt")),
                ("read", canonical.join("sub/new.txt")),
            ]
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("box/sub/new.txt")).unwrap(),
            "entry"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The environment variable that marks the process
    /// [`a_grant_on_the_current_directory_reaches_its_files_2144`] starts for its scenario: the
    /// scenario does nothing in a process that was not started for it.
    #[cfg(unix)]
    const CURRENT_DIRECTORY_SCENARIO: &str = "UNA_RT_HOST_CURRENT_DIRECTORY_SCENARIO";

    #[cfg(unix)]
    #[test]
    #[cfg_attr(miri, ignore = "uses the file system, which Miri's isolation refuses")]
    #[allow(clippy::unwrap_used)]
    fn a_grant_on_the_current_directory_reaches_its_files_2144() {
        // The scenario resolves grants against the current directory of its process, so it
        // runs in a process of its own, started in a directory this test makes: the test
        // holds wherever its binary is started (Cargo starts it in the package's directory,
        // `scripts/sanitize.sh` in the repository's), and no other test's directory changes.
        let dir = std::env::temp_dir().join(format!("una-rt-here-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("src/host.rs"), "//! A file.\n").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "host::tests::a_grant_on_the_current_directory_reaches_its_files_2144_scenario",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .current_dir(&dir)
            .env(CURRENT_DIRECTORY_SCENARIO, "1")
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{output:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "run by `a_grant_on_the_current_directory_reaches_its_files_2144`, in a process of its own"]
    #[allow(clippy::unwrap_used, clippy::panic, clippy::disallowed_methods)]
    fn a_grant_on_the_current_directory_reaches_its_files_2144_scenario() {
        // #2144: an executable resolves its grants against the current directory, and the
        // prefix of `files:.:<modes>` was the empty path, which does not resolve, so every
        // operation under it ended with `NotFound` and the machine was given no path. The
        // prefix is `.`: the operation is given the canonical current directory joined with
        // the final component, and a missing directory is still answered with no path.
        // The current directory is the one the parent test made, with `Cargo.toml` and
        // `src/host.rs` in it, which this scenario only reads.
        use super::{FileFault, Grant, on_confined};
        if std::env::var_os(CURRENT_DIRECTORY_SCENARIO).is_none() {
            return;
        }
        let here = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        assert!(here.join("src/host.rs").is_file(), "{here:?}");
        let grant = |spec: &str| {
            Grant::parse(spec, std::path::Path::new("")).unwrap_or_else(|| panic!("{spec}"))
        };
        let read = |spec: &str, relative: &str| {
            let mut world = Operations::default();
            let outcome = on_confined(&mut world, &[grant(spec)], relative, false, |world, at| {
                world.read_file(at).map(|_| ())
            });
            (outcome, world.0)
        };
        for spec in ["files:.:read", "files:./:read", "files:src/..:read"] {
            assert_eq!(
                read(spec, "Cargo.toml"),
                (Some(Ok(())), vec![("read", here.join("Cargo.toml"))]),
                "{spec}"
            );
            assert_eq!(
                read(spec, "src/host.rs"),
                (Some(Ok(())), vec![("read", here.join("src/host.rs"))]),
                "{spec}"
            );
            assert_eq!(read(spec, "../Cargo.toml"), (None, Vec::new()), "{spec}");
        }
        for spec in ["files:.:read", "files:src:read", "files:./src:read"] {
            let directory = if spec == "files:.:read" {
                here.clone()
            } else {
                here.join("src")
            };
            // A file that does not exist in a directory that resolves is the operation's.
            assert_eq!(
                read(spec, "no-such-file-2144.txt"),
                (
                    Some(Err(FileFault::NotFound)),
                    vec![("read", directory.join("no-such-file-2144.txt"))]
                ),
                "{spec}"
            );
            // A missing directory is answered by confinement (#1459).
            assert_eq!(
                read(spec, "no-such-directory-2144/new.txt"),
                (Some(Err(FileFault::NotFound)), Vec::new()),
                "{spec}"
            );
        }
        assert_eq!(
            read("files:src:read", "host.rs"),
            (Some(Ok(())), vec![("read", here.join("src/host.rs"))])
        );
        // A write is confined by the same route, and is not performed.
        let mut world = Operations::default();
        let written = on_confined(
            &mut world,
            &[grant("files:.:write")],
            "no-such-file-2144.txt",
            true,
            |_, at| Ok::<_, FileFault>(at.to_path_buf()),
        );
        assert_eq!(written, Some(Ok(here.join("no-such-file-2144.txt"))));
        assert_eq!(world.0, Vec::new());
    }

    /// The machine, keeping every path that any of its methods is asked about, the lookups of
    /// confinement included: owned, so that it goes where [`super::files_read_in`] sends its
    /// world.
    #[cfg(unix)]
    #[derive(Clone, Default)]
    struct Asked(std::sync::Arc<std::sync::Mutex<Vec<(&'static str, std::path::PathBuf)>>>);

    #[cfg(unix)]
    impl Asked {
        fn keep(&self, method: &'static str, path: &std::path::Path) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((method, path.to_path_buf()));
        }

        /// What was asked since the last call.
        fn take(&self) -> Vec<(&'static str, std::path::PathBuf)> {
            std::mem::take(
                &mut self
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        }
    }

    #[cfg(unix)]
    impl super::World for Asked {
        fn print(&mut self, _line: &str) {}
        fn now(&mut self) -> i64 {
            0
        }
        fn env_var(&mut self, _name: &str) -> Result<Option<String>, super::NotText> {
            Ok(None)
        }
        fn read_file(&mut self, path: &std::path::Path) -> Result<String, super::FileFault> {
            self.keep("read", path);
            super::Machine.read_file(path)
        }
        fn write_file(
            &mut self,
            path: &std::path::Path,
            content: &str,
        ) -> Result<(), super::FileFault> {
            self.keep("write", path);
            super::Machine.write_file(path, content)
        }
        fn canonical(&mut self, path: &std::path::Path) -> Option<std::path::PathBuf> {
            self.keep("canonical", path);
            super::Machine.canonical(path)
        }
        fn is_link(&mut self, path: &std::path::Path) -> bool {
            self.keep("is_link", path);
            super::Machine.is_link(path)
        }
        fn step(
            &mut self,
            directory: &std::path::Path,
            name: &std::ffi::OsStr,
        ) -> Option<std::path::PathBuf> {
            self.keep("step", &directory.join(name));
            super::Machine.step(directory, name)
        }
        fn absent(&mut self, path: &std::path::Path) -> Option<una_interp::Absence> {
            self.keep("absent", path);
            super::Machine.absent(path)
        }
    }

    #[cfg(unix)]
    #[test]
    #[cfg_attr(miri, ignore = "uses the file system, which Miri's isolation refuses")]
    #[allow(clippy::unwrap_used)]
    fn files_read_and_write_give_the_world_no_path_below_an_absent_directory_2138() {
        // `spec/14 §3` rule 2, #1459, end to end in the runtime (#2138): the operations that
        // generated code calls, `files_read` and `files_write`, with the grants of the run's
        // state and paths that are texts of the run's heap. Each case runs twice: through
        // `files_read_in` and `files_write_in`, the whole of each operation, in a world that is
        // the machine and keeps every path any method is asked about; and through `files_read`
        // and `files_write` themselves, which must answer the same word.
        //
        // What this does not observe, and is read in the source: that `files_read` and
        // `files_write` pass the machine and nothing else to those two functions, and that the
        // machine's methods make no operating system call other than on the path they are
        // given. The calls are outside a task, so the job runs inline; in a task the same job,
        // with the world moved into it, runs on a helper.
        use super::{DENIED, Grant, IO, NOT_FOUND};
        use crate::runtime::with_state;
        use crate::values;
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;
        use std::path::{Component, Path, PathBuf};
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("una-rt-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("box/sub")).unwrap();
        std::fs::write(dir.join("box/note.txt"), "inside\n").unwrap();
        symlink("sub", dir.join("box/in_dir")).unwrap();
        symlink("box", dir.join("boxlink")).unwrap();
        let canonical = std::fs::canonicalize(dir.join("box")).unwrap();
        let prefix = dir.join("boxlink");
        // The outcome of an operation: its word when it failed, `Ok` otherwise.
        fn outcome(word: i64) -> Result<i64, i64> {
            match super::host_failed() {
                0 => Ok(word),
                _ => Err(word),
            }
        }
        // A component after one named `missing`, or after the file `note.txt`: a path below a
        // directory that is not there.
        fn below_absent(path: &Path) -> bool {
            let mut parts = path.components().skip_while(|part| {
                !matches!(part, Component::Normal(name)
                    if *name == OsStr::new("missing") || *name == OsStr::new("note.txt"))
            });
            // `note.txt/x` is what confinement finds not to be a directory: one component
            // below the file is looked up, none is opened.
            let at = parts.next();
            let file = at == Some(Component::Normal(OsStr::new("note.txt")));
            parts.nth(usize::from(file)).is_some()
        }
        let thread_prefix = prefix.clone();
        let thread_canonical = canonical.clone();
        std::thread::spawn(move || {
            let (prefix, canonical) = (thread_prefix, thread_canonical);
            crate::runtime::install_test_state_as("0".to_owned(), false, String::new());
            with_state(|state| {
                state.grants = Arc::new(vec![Grant::Files {
                    prefix: prefix.clone(),
                    read: true,
                    write: true,
                }]);
            });
            let asked = Asked::default();
            let text = |text: &str| values::text_word(text.to_owned());
            let read_in = |path: &str| outcome(super::files_read_in(asked.clone(), 0, text(path)));
            let write_in = |path: &str| {
                outcome(super::files_write_in(
                    asked.clone(),
                    text(path),
                    text("entry"),
                ))
            };
            let read = |path: &str| outcome(super::files_read(0, text(path)));
            let write = |path: &str| outcome(super::files_write(text(path), text("entry")));
            // Below a directory that does not exist, and below a file: the failure, and no
            // read or write in the world; no method of the world is asked about the path as
            // written, about anything below the absent directory, or about a path with a NUL.
            for (path, word) in [
                ("missing/new.txt", NOT_FOUND),
                ("missing/deeper/new.txt", NOT_FOUND),
                ("in_dir/missing/new.txt", NOT_FOUND),
                ("sub/../missing/a/b/new.txt", NOT_FOUND),
                ("missing/a\0b", NOT_FOUND),
                ("missing/a\0b/new.txt", NOT_FOUND),
                ("note.txt/x/new.txt", IO),
            ] {
                for writing in [false, true] {
                    let (seen, real) = if writing {
                        (write_in(path), write(path))
                    } else {
                        (read_in(path), read(path))
                    };
                    assert_eq!(seen, Err(word), "{path:?} {writing}");
                    assert_eq!(real, Err(word), "{path:?} {writing}");
                    let asked = asked.take();
                    assert!(!asked.is_empty(), "{path:?}: confinement looked nothing up");
                    for (method, at) in &asked {
                        assert!(
                            !matches!(*method, "read" | "write"),
                            "{path:?}: {method} {at:?}"
                        );
                        assert_ne!(*at, prefix.join(path), "{path:?}: {method}");
                        assert!(!below_absent(at), "{path:?}: {method} {at:?}");
                        assert!(
                            !at.as_os_str().as_bytes().contains(&0),
                            "{path:?}: {method} {at:?}"
                        );
                    }
                }
            }
            assert!(!canonical.join("missing").exists());
            // Denied: no read or write either.
            assert_eq!(read_in("../box/note.txt"), Err(DENIED));
            assert_eq!(read("../box/note.txt"), Err(DENIED));
            assert_eq!(write_in("/outside/new.txt"), Err(DENIED));
            assert!(
                asked
                    .take()
                    .iter()
                    .all(|(method, _)| !matches!(*method, "read" | "write"))
            );
            // A NUL in a directory that confinement must resolve is not absence: `Denied`.
            assert_eq!(read_in("a\0b/new.txt"), Err(DENIED));
            assert_eq!(read("a\0b/new.txt"), Err(DENIED));
            assert_eq!(write_in("sub/a\0b/new.txt"), Err(DENIED));
            assert_eq!(write("sub/a\0b/new.txt"), Err(DENIED));
            assert!(
                asked
                    .take()
                    .iter()
                    .all(|(method, _)| !matches!(*method, "read" | "write"))
            );
            // A final component in a directory that resolves is the operation's: the world
            // is given the canonical directory joined with it, and nothing else. One that
            // does not exist is `NotFound` for a read and created by a write; one with a NUL
            // is the operation's own `Io`.
            let operations = |asked: &Asked| -> Vec<(&'static str, PathBuf)> {
                asked
                    .take()
                    .into_iter()
                    .filter(|(method, _)| matches!(*method, "read" | "write"))
                    .collect()
            };
            assert_eq!(read_in("in_dir/gone.txt"), Err(NOT_FOUND));
            assert_eq!(read("in_dir/gone.txt"), Err(NOT_FOUND));
            assert_eq!(write_in("in_dir/new.txt"), Ok(0));
            assert_eq!(
                read_in("in_dir/new.txt").map(values::text_value),
                Ok("entry".to_owned())
            );
            assert_eq!(write("in_dir/real.txt"), Ok(0));
            assert_eq!(
                read("in_dir/real.txt").map(values::text_value),
                Ok("entry".to_owned())
            );
            assert_eq!(read_in("in_dir/a\0b"), Err(IO));
            assert_eq!(read("in_dir/a\0b"), Err(IO));
            assert_eq!(write_in("a\0b"), Err(IO));
            assert_eq!(write("a\0b"), Err(IO));
            assert_eq!(
                operations(&asked),
                vec![
                    ("read", canonical.join("sub/gone.txt")),
                    ("write", canonical.join("sub/new.txt")),
                    ("read", canonical.join("sub/new.txt")),
                    ("read", canonical.join("sub/a\0b")),
                    ("write", canonical.join("a\0b")),
                ]
            );
            drop(crate::runtime::leave_task());
        })
        .join()
        .unwrap_or_else(|_| panic!("the test thread panicked"));
        assert_eq!(
            std::fs::read_to_string(dir.join("box/sub/new.txt")).unwrap(),
            "entry"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("box/sub/real.txt")).unwrap(),
            "entry"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_operation_site_without_its_outcome_label_fails_to_load() {
        // S-45 (#1552, `spec/11 §5`), review L3 of #1562: the native description's `outcome`
        // key is required. `null` is an outcome in clear and a string is the level it is
        // redacted at; a site that lacks the key, or gives a value that is neither `null` nor a
        // string, fails to load, so the executable never shows an outcome in clear by default.
        let site = |outcome: Option<serde_json::Value>| {
            let mut value = serde_json::json!({
                "capability": "Console",
                "op": "print",
                "params": [],
                "args": [],
                "result": {"type": 0},
            });
            if let (Some(outcome), Some(fields)) = (outcome, value.as_object_mut()) {
                fields.insert("outcome".to_owned(), outcome);
            }
            super::OpSite::parse(&value)
        };
        assert_eq!(site(None).map(|op| op.outcome), None);
        assert_eq!(site(Some(serde_json::json!(1))).map(|op| op.outcome), None);
        assert_eq!(
            site(Some(serde_json::Value::Null)).map(|op| op.outcome),
            Some(None)
        );
        assert_eq!(
            site(Some(serde_json::json!("internal"))).map(|op| op.outcome),
            Some(Some("internal".to_owned()))
        );
    }

    /// Starts, on `sched`, a task whose operation runs on a helper of the I/O pool through
    /// [`super::on_helper`] until `gate` opens, its cancellation entry that of `scope`'s tasks;
    /// the receiver gives what [`super::on_helper`] gave.
    fn on_a_helper(
        sched: &crate::sched::Scheduler,
        scope: &crate::started::StartScope,
        gate: std::sync::mpsc::Receiver<()>,
    ) -> std::sync::mpsc::Receiver<Option<Result<i32, Error>>> {
        let (outcome, seen) = std::sync::mpsc::channel();
        let entries = vec![(
            std::sync::Arc::clone(&scope.cancel),
            crate::tasks::RACE_ENTRY | 1,
        )];
        sched
            .spawn(
                1,
                256 * 1024,
                move || {
                    let abandoned: crate::interrupt::Wake = std::sync::Arc::new(|| {});
                    let done = super::on_helper(&entries, abandoned, move || {
                        let _ = gate.recv();
                        Ok(7)
                    });
                    let _ = outcome.send(done);
                },
                || {},
            )
            .unwrap();
        seen
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_cut_fails_a_queued_operation_timed_out_and_a_failure_interrupts_it() {
        // #1266, `spec/27 §2` rule 4: behind the only helper, a deadline's cut of the `tasks`
        // statement takes a queued operation off the queue unsent, and it fails with
        // `TimedOut` (`RateLimited` for `acquire`), as the job that runs does at the same
        // deadline, never `interrupted`; a failure of the block interrupts it (`None`). Before
        // the fix the cut ended the queued one `interrupted` too, which depended on the load
        // of the pool. `Http` and `limit_acquire` both map their helper's wait through it.
        let sched = crate::sched::Scheduler::new(una_interp::Concurrency {
            workers: 1,
            task_depth: 10,
            io_threads: 1,
            file_threads: 1,
        });
        let wait = std::time::Duration::from_millis(100);
        let long = std::time::Duration::from_secs(10);
        for by_deadline in [true, false] {
            let scope = crate::started::StartScope::new("s1".to_owned(), 2);
            let (release, gate) = std::sync::mpsc::channel();
            let running = on_a_helper(&sched, &scope, gate);
            std::thread::sleep(wait);
            let (never, queued_gate) = std::sync::mpsc::channel::<()>();
            let queued = on_a_helper(&sched, &scope, queued_gate);
            std::thread::sleep(wait);
            crate::started::cancel_tasks(&scope, by_deadline);
            let queued = queued.recv_timeout(long).unwrap();
            if by_deadline {
                assert_eq!(queued, Some(Err(Error::TimedOut)));
                // The running job is waited for, and gives its own result.
                assert!(
                    running.recv_timeout(wait).is_err(),
                    "woken before its helper"
                );
                release.send(()).unwrap();
                assert_eq!(running.recv_timeout(long).unwrap(), Some(Ok(7)));
            } else {
                assert_eq!(queued, None);
                assert_eq!(running.recv_timeout(long).unwrap(), None);
                let _ = release.send(());
                std::thread::sleep(wait);
            }
            drop(never);
        }
    }

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
