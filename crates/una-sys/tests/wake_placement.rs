//! Where the wake pipe lands under a soft limit of open files (#1757): the test runs itself
//! again as a child of `sh`, whose `ulimit -n` and redirections set the limit and the
//! descriptors the child starts with; the child installs the handlers and writes the read
//! end's descriptor, or the error, to a file its environment names.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

/// The name of this test, which the child runs alone.
const PROBE: &str =
    "the_wake_pipe_lands_as_high_as_the_limit_allows_and_never_on_a_standard_stream";

/// Runs this test again under `ulimit -n limit`, with the shell redirections `redirect`, and,
/// with `closed`, its standard streams closed before it installs the handlers, and returns
/// what the child wrote: `ok <read end>` or `err <usage> <message>`. The child closes them
/// itself: Rust's start-up opens `/dev/null` on a standard stream the shell closed, which a
/// native Una executable does not.
fn probe(limit: u64, redirect: &str, closed: bool) -> String {
    let out = std::env::temp_dir().join(format!(
        "una-wake-probe-{}-{limit}-{}",
        std::process::id(),
        redirect.len()
    ));
    let _ = std::fs::remove_file(&out);
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -n {limit} && exec \"$0\" --exact {PROBE} --test-threads 1 {redirect}"
        ))
        .arg(std::env::current_exe().unwrap())
        .env("UNA_SYS_WAKE_PROBE", &out)
        .env("UNA_SYS_WAKE_CLOSED", if closed { "1" } else { "" })
        .status()
        .unwrap();
    assert!(status.success(), "{limit} {redirect}: {status:?}");
    let seen = std::fs::read_to_string(&out).expect("the child wrote nothing");
    let _ = std::fs::remove_file(&out);
    seen
}

#[test]
#[allow(clippy::exit, unsafe_code)]
fn the_wake_pipe_lands_as_high_as_the_limit_allows_and_never_on_a_standard_stream() {
    #[allow(clippy::disallowed_methods)]
    if let Some(out) = std::env::var_os("UNA_SYS_WAKE_PROBE") {
        #[allow(clippy::disallowed_methods)]
        let closed = std::env::var_os("UNA_SYS_WAKE_CLOSED").is_some_and(|v| v == "1");
        if closed {
            for fd in 0..3 {
                // SAFETY: closing a standard stream of this child process touches no memory;
                // the child writes only to the file `out` from here on.
                unsafe { libc::close(fd) };
            }
        }
        let seen = match una_sys::signals::install() {
            Ok(()) => format!("ok {}", una_sys::signals::wake_fd().unwrap()),
            Err(error) => format!("err {} {}", error.usage, error.message),
        };
        std::fs::write(out, seen).unwrap();
        std::process::exit(0);
    }
    // The read end moves first, to `min(512, limit - 2)`, and the write end after it.
    for (limit, read) in [(64, 62), (256, 254), (513, 511), (514, 512), (1024, 512)] {
        assert_eq!(probe(limit, "", false), format!("ok {read}"), "at {limit}");
    }
    // Review M1: closed standard streams and the top two descriptors held; the ends move to
    // the lowest free ones from 3, never onto 0, 1 or 2.
    let seen = probe(64, "62</dev/null 63</dev/null", true);
    let read: i32 = seen.strip_prefix("ok ").expect(&seen).parse().unwrap();
    assert!((3..62).contains(&read), "{seen}");
    // Review L1: under a limit that is large enough, descriptors in use are not reported as
    // a small limit.
    let held: Vec<String> = (3..62).map(|fd| format!("{fd}</dev/null")).collect();
    let seen = probe(64, &held.join(" "), false);
    assert!(seen.starts_with("err true "), "{seen}");
    assert!(
        seen.contains("needs 4") && !seen.contains("too small"),
        "{seen}"
    );
}
