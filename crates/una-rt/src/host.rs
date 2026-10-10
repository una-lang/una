//! Host capabilities and traces (`spec/16 §3` and `§4` rule 5): the operations of `std.host`
//! with the interpreter's semantics, grants and path rules (`spec/14 §3`), the instances of
//! capabilities, and the events a trace records for every operation (`spec/11 §5`).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use una_interp::{Event, EventOutcome, FileFault, Grant, NotText, World};

use crate::render::index;
use crate::runtime::with_state;
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
    let mut stdout = io::stdout().lock();
    let written = writeln!(stdout, "{line}").and_then(|()| stdout.flush());
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
    with_state(|state| state.host_failed = true);
    word
}

/// Whether the last host operation failed, its value being then the failure; clears it.
pub fn host_failed() -> i64 {
    with_state(|state| std::mem::take(&mut state.host_failed))
        .map(i64::from)
        .unwrap_or(0)
}

/// The variants of `FileError` and `KeyError` (`spec/14 §3`), as words.
const NOT_FOUND: i64 = 1;
const DENIED: i64 = 3;
const IO: i64 = 5;
const UNKNOWN_KEY: i64 = 1;

/// `Console.print(line)`.
pub fn console_print(line: i64) -> i64 {
    print(&values::text_value(line));
    0
}

/// `Clock.now()`: milliseconds since the Unix epoch.
pub fn clock_now() -> i64 {
    values::int_word(&num_bigint::BigInt::from(Machine.now()))
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
fn confined(relative: &str, writing: bool) -> Option<PathBuf> {
    let grants = with_state(|state| state.grants.clone()).unwrap_or_default();
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
pub fn files_read(site: i64, path: i64) -> i64 {
    let Some(target) = confined(&values::text_value(path), false) else {
        return failed(DENIED);
    };
    match Machine.read_file(&target) {
        Ok(text) => values::text_word(text),
        Err(FileFault::NotText) => not_text(site, "the content of this file is not valid UTF-8"),
        Err(fault) => file_failure(fault),
    }
}

/// `Files.write(path, content)`.
pub fn files_write(path: i64, content: i64) -> i64 {
    let Some(target) = confined(&values::text_value(path), true) else {
        return failed(DENIED);
    };
    match Machine.write_file(&target, &values::text_value(content)) {
        Ok(()) => 0,
        Err(fault) => file_failure(fault),
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
        let count = state.installs.entry(name.clone()).or_insert(0);
        *count += 1;
        let count = *count;
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
        let trace = state.trace.as_mut()?;
        let slot = trace.len();
        trace.push(Event {
            seq: slot + 1,
            task: state.path.clone(),
            instance: id,
            capability,
            op: name,
            args: Vec::new(),
            outcome: EventOutcome::Aborted,
        });
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
/// is 1.
pub fn event_end(site: i64, slot: i64, value: i64, failed: i64) {
    let (Ok(site), Ok(slot)) = (usize::try_from(site), usize::try_from(slot)) else {
        return;
    };
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
    use una_http::client::{Error, Request, send};
    // The variants of `HttpError`, in declaration order, as words.
    let variant = |error: Error| {
        let index: i64 = match error {
            Error::Denied => 0,
            Error::BadUrl => 1,
            Error::BadRequest => 2,
            Error::Unreachable => 3,
            Error::TimedOut => 4,
            Error::TooLarge => 5,
            Error::Tls => 6,
            Error::Protocol => 7,
            // Not a failure: `http` aborts before (N-40).
            Error::HeaderValue => 2,
        };
        (index << 1) | 1
    };
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
    let (grants, settings) = with_state(|state| {
        let grants: Vec<una_http::grant::Grant> = state
            .grants
            .iter()
            .filter_map(|grant| match grant {
                Grant::Http(http) => Some(http.clone()),
                _ => None,
            })
            .collect();
        (grants, state.http)
    })
    .unwrap_or_default();
    let request = Request {
        method,
        url: &url,
        headers: &headers,
        body: body.as_bytes(),
        bearer: bearer.as_deref(),
    };
    match send(&request, &grants, &settings) {
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
        // The run, or the task, aborts with U6004 at the operation (N-40); a task's caller
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
