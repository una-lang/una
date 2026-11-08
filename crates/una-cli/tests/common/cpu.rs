//! The processor time of a child that a scaling test runs (#1656), the measure that
//! `timing::assert_growth` judges for the tests whose claim is about the time a run of `una` or
//! of a built executable takes: a child's user and system time stay what its work costs when
//! other tests run beside it, where its elapsed time does not, so these tests need no slot of
//! their own (`.config/nextest.toml`, the names that hold `cpu_time`).
//!
//! The child runs under the POSIX utility `time -p` (`/usr/bin/time`, on macOS and in the Linux
//! image of `scripts/local-ci/Dockerfile`), which waits for it and prints its `real`, `user` and
//! `sys` seconds on its standard error after the child's own. That reading counts the child's
//! waited-for children too (a solver, a linker). It needs no `unsafe` code and no dependency:
//! the standard library has no interface to another process's processor time, and a test
//! measurement is no reason to widen the code that may hold `unsafe`.
//!
//! Resolution: `time -p` prints two decimals, so a reading is within 10 ms of the time the
//! kernel counted. A run that reads under [`LEAST`] seconds, as the smallest sizes of some tests
//! do, is measured again as that many runs in a row under one `time -p` (a POSIX `sh` loop, its
//! runs' output discarded), enough of them to read [`LEAST`] seconds or more, and the reading is
//! their mean: within 10 ms over the whole loop, at most 2.5% of it. The output returned is the
//! first run's. The loop adds the shell's fork per run, well under a millisecond, to the
//! smaller sizes only. Sizes, bounds and floors stay the tests' own.
//!
//! A test that measures work in its own process (a check, the interpreter) runs that work in a
//! child of its test binary ([`in_child`]): the test, run again alone with an argument that
//! names one measurement, makes it and exits, and the parent reads that child's processor time
//! as for a run of `una`. A clock or a counter of the test's own process could not stand in for
//! it: the elapsed time grows with the load, and the test process's own processor time counts
//! every test thread beside it under `cargo test`.

// Each test file uses its own subset of these helpers.
#![allow(dead_code)]

use std::process::{Command, Output};

/// Names, in a child of the test binary that [`in_child`] runs, the argument of the one
/// measurement it makes, as `<nonce> <file> <argument>`: the parent writes the nonce, new for each
/// child, into the file and removes the file once the child has run, so that a value that did not
/// come from a parent (one left in the environment of the run) fails the test rather than making
/// it return without measuring ([`child`]).
const CHILD: &str = "UNA_CPU_TIME_CHILD";

/// The children [`in_child`] has started in this process, a part of each nonce.
static CHILDREN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What a child prints once its measurement has passed, so that the parent knows its filter
/// named the test.
const RAN: &str = "una cpu-time child: ran";

/// The `time -p` utility.
const TIME: &str = "/usr/bin/time";

/// The least processor time, in seconds, that one reading covers: below it, the 10 ms step of
/// `time -p` would be more than 2.5% of the reading.
pub const LEAST: f64 = 0.4;

/// The output of `command`, run under `time -p` with its program, arguments, working directory
/// and environment changes, and the processor seconds (user and system) it took: for a run that
/// reads under [`LEAST`] seconds, the mean of enough runs in a row to read at least that. The
/// output's standard error is the child's own: the lines that `time -p` adds are taken off.
pub fn cpu_output(command: &Command) -> (Output, f64) {
    let (output, seconds) = timed(command, 1);
    if seconds >= LEAST || !output.status.success() {
        return (output, seconds);
    }
    // Counted up rather than divided and rounded, as in `timing::repeats`: a reading of 0.00 s
    // counts as 0.01 s, so at most 40 runs.
    let mut runs = 2;
    while (runs as f64) * seconds.max(0.01) < LEAST {
        runs += 1;
    }
    let (again, total) = timed(command, runs);
    assert!(again.status.success(), "{runs} runs in a row: {again:?}");
    (output, total / runs as f64)
}

/// For a test whose claim is about work done in its own process (a check, the interpreter): the
/// argument of the measurement this process makes when it is a child that [`in_child`] runs,
/// else `None`. The test then makes that one measurement, calls [`ran`] and returns. A value of
/// [`CHILD`] that its parent did not pass, without the nonce its file holds, fails the test.
pub fn child() -> Option<String> {
    // A test reads the variable its own parent set; no library code reads it.
    #[allow(clippy::disallowed_methods)]
    let value = std::env::var(CHILD).ok()?;
    let argument = verified(&value);
    assert!(
        argument.is_ok(),
        "{CHILD} is set, but not by a parent test: {argument:?}"
    );
    argument.ok()
}

/// The argument of a value of [`CHILD`], `<nonce> <file> <argument>`, when `file` holds `nonce`.
fn verified(value: &str) -> Result<String, String> {
    let mut parts = value.splitn(3, ' ');
    let (Some(nonce), Some(file), Some(argument)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(format!("not `<nonce> <file> <argument>`: {value:?}"));
    };
    match std::fs::read_to_string(file) {
        Ok(held) if !nonce.is_empty() && held == nonce => Ok(argument.to_owned()),
        Ok(held) => Err(format!("the nonce {nonce:?} is not the {held:?} of {file}")),
        Err(error) => Err(format!("no nonce in {file}: {error}")),
    }
}

/// Says, in a child, that its measurement has passed: its standard output is the parent's to read.
#[allow(clippy::print_stdout)]
pub fn ran() {
    println!("{RAN}");
}

/// The processor seconds of a child of this test binary that runs only `test`, which makes the
/// measurement `argument` names ([`child`]), under [`cpu_output`]. The child's work is in its
/// own process, so its user and system time count it whatever runs beside it, as for a run of
/// `una`; what the start of the test binary adds is a fixed cost, which the tests measure on a
/// tiny size the same way. Asserts that the child ran the test and passed.
pub fn in_child(test: &str, argument: &str) -> f64 {
    in_child_output(test, argument).1
}

/// [`in_child`], with the output of the child's first run ([`cpu_output`]).
pub fn in_child_output(test: &str, argument: &str) -> (Output, f64) {
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let count = CHILDREN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let nonce = format!("{}-{count}-{started}", std::process::id());
    let file = std::env::temp_dir().join(format!("una-cpu-time-child-{nonce}"));
    std::fs::write(&file, &nonce).expect("the nonce file");
    let mut command = Command::new(std::env::current_exe().expect("the test binary"));
    command
        .args(["--exact", test, "--nocapture", "--test-threads", "1"])
        .env(CHILD, format!("{nonce} {} {argument}", file.display()));
    let (output, seconds) = cpu_output(&command);
    let _ = std::fs::remove_file(&file);
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains(RAN),
        "{test} in a child, {argument}: {output:?}"
    );
    (output, seconds)
}

/// The output and processor seconds of `runs` runs of `command` under one `time -p`: the run
/// itself for one, else a POSIX `sh` loop that discards the runs' output and stops at a failure.
fn timed(command: &Command, runs: usize) -> (Output, f64) {
    let mut timed = Command::new(TIME);
    timed.arg("-p");
    if runs > 1 {
        timed.args([
            "sh",
            "-c",
            "n=$1; shift; i=0; while [ \"$i\" -lt \"$n\" ]; do \"$@\" >/dev/null 2>&1 || exit 1; i=$((i + 1)); done",
            "sh",
        ]);
        timed.arg(runs.to_string());
    }
    timed.arg(command.get_program()).args(command.get_args());
    if let Some(dir) = command.get_current_dir() {
        timed.current_dir(dir);
    }
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => timed.env(key, value),
            None => timed.env_remove(key),
        };
    }
    let mut output = timed.output().expect("/usr/bin/time -p runs");
    let (stderr, seconds) = split_report(&output.stderr);
    output.stderr = stderr;
    (output, seconds)
}

/// The child's own standard error and its user plus system seconds, from the standard error of
/// `time -p`: the child's text, then (GNU `time` only, for a child that failed) a line
/// `Command exited with ...` or `Command terminated by ...`, then `real`, `user` and `sys`.
fn split_report(stderr: &[u8]) -> (Vec<u8>, f64) {
    let text = String::from_utf8_lossy(stderr);
    let start = text
        .rfind("real ")
        .filter(|&at| at == 0 || text.as_bytes().get(at - 1) == Some(&b'\n'))
        .expect("the report of `time -p` on the standard error");
    let mut seconds = 0.0;
    let mut fields = 0;
    for line in text.get(start..).unwrap_or_default().lines() {
        let mut words = line.split_whitespace();
        if let (Some(name @ ("user" | "sys")), Some(value)) = (words.next(), words.next()) {
            let value: Option<f64> = value.parse().ok();
            assert!(value.is_some(), "`time -p` {name} in: {text}");
            seconds += value.unwrap_or_default();
            fields += 1;
        }
    }
    assert_eq!(fields, 2, "no user and sys of `time -p` in: {text}");
    let mut own = text.get(..start).unwrap_or_default();
    if let Some(at) = own
        .trim_end_matches('\n')
        .rfind('\n')
        .map(|at| at + 1)
        .or(Some(0))
    {
        let last = own.get(at..).unwrap_or_default();
        if last.starts_with("Command exited with ") || last.starts_with("Command terminated by ") {
            own = own.get(..at).unwrap_or_default();
        }
    }
    (own.as_bytes().to_vec(), seconds)
}

#[test]
fn a_child_value_without_its_parent_s_nonce_is_refused() {
    // A value left in the environment, an idle one included, is no measurement to make.
    for value in ["", "idle", "1 /nonexistent/una-cpu-time-child idle"] {
        assert!(verified(value).is_err(), "{value:?}");
    }
    let file = std::env::temp_dir().join(format!("una-cpu-time-child-test-{}", std::process::id()));
    std::fs::write(&file, "n-1-2").unwrap();
    let path = file.display();
    assert!(verified(&format!("n-1-3 {path} idle")).is_err());
    assert!(verified(&format!(" {path} idle")).is_err());
    assert_eq!(
        verified(&format!("n-1-2 {path} idle")),
        Ok("idle".to_owned())
    );
    // The argument keeps its own spaces: a size and a directory.
    assert_eq!(
        verified(&format!("n-1-2 {path} 10 /tmp/a b")),
        Ok("10 /tmp/a b".to_owned())
    );
    // Once the parent removed the file, the same value is refused.
    std::fs::remove_file(&file).unwrap();
    assert!(verified(&format!("n-1-2 {path} idle")).is_err());
}

#[test]
fn the_report_of_time_p_is_taken_off_the_child_s_own_standard_error() {
    // macOS (BSD) pads the fields; GNU `time` adds a line for a child that failed.
    let (own, seconds) =
        split_report(b"note: copied: 1\nreal         0.52\nuser         0.40\nsys          0.08\n");
    assert_eq!(own, b"note: copied: 1\n");
    assert!((seconds - 0.48).abs() < 1e-9, "{seconds}");
    let (own, seconds) = split_report(
        b"oops\nCommand exited with non-zero status 2\nreal 1.00\nuser 0.25\nsys 0.25\n",
    );
    assert_eq!(own, b"oops\n");
    assert!((seconds - 0.5).abs() < 1e-9, "{seconds}");
    let (own, _) = split_report(b"real 0.00\nuser 0.00\nsys 0.00\n");
    assert!(own.is_empty());
}
