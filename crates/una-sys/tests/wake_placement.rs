//! Where the wake pipe lands under a soft limit of open files (#1757): the test runs itself
//! again as a child of `sh`, whose `ulimit -n` sets the limit; the child holds the descriptors
//! its environment lists, installs the handlers and writes the read end's descriptor, or the
//! error, to a file its environment names.
//!
//! The child holds them itself, with `dup2`, rather than by redirections of `sh` (#1920): POSIX
//! leaves a redirection of a descriptor above 9 to the shell, and `dash`, the `sh` of Debian and
//! Ubuntu, reads `62</dev/null` as the argument `62` and a redirection of standard input, so a
//! run there held none of them and the check that a busy limit is reported never ran.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

/// The name of this test, which the child runs alone.
const PROBE: &str =
    "the_wake_pipe_lands_as_high_as_the_limit_allows_and_never_on_a_standard_stream";

/// Runs this test again under `ulimit -n limit`, holding the descriptors `held` open on
/// `/dev/null`, and, with `closed`, its standard streams closed before it installs the
/// handlers, and returns what the child wrote: `ok <read end>` or `err <usage> <message>`. The
/// child closes them itself: Rust's start-up opens `/dev/null` on a standard stream the shell
/// closed, which a native Una executable does not.
fn probe(limit: u64, held: &[libc::c_int], closed: bool) -> String {
    let out = std::env::temp_dir().join(format!(
        "una-wake-probe-{}-{limit}-{}-{closed}",
        std::process::id(),
        held.len()
    ));
    let _ = std::fs::remove_file(&out);
    let list: Vec<String> = held.iter().map(ToString::to_string).collect();
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -n {limit} && exec \"$0\" --exact {PROBE} --test-threads 1"
        ))
        .arg(std::env::current_exe().unwrap())
        .env("UNA_SYS_WAKE_PROBE", &out)
        .env("UNA_SYS_WAKE_CLOSED", if closed { "1" } else { "" })
        .env("UNA_SYS_WAKE_HELD", list.join(","))
        .status()
        .unwrap();
    assert!(status.success(), "{limit} {held:?}: {status:?}");
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
        #[allow(clippy::disallowed_methods)]
        let held: Vec<libc::c_int> = std::env::var("UNA_SYS_WAKE_HELD")
            .unwrap()
            .split(',')
            .filter(|fd| !fd.is_empty())
            .map(|fd| fd.parse().unwrap())
            .collect();
        if !held.is_empty() {
            let null =
                std::os::fd::IntoRawFd::into_raw_fd(std::fs::File::open("/dev/null").unwrap());
            for fd in &held {
                // SAFETY: `dup2` makes `fd` a duplicate of `null`, a descriptor this child just
                // opened; no memory is touched. A descriptor already open at `fd` is replaced,
                // as a redirection of the shell would replace it.
                assert!(
                    *fd == null || unsafe { libc::dup2(null, *fd) } == *fd,
                    "dup2 {fd}"
                );
            }
            if !held.contains(&null) {
                // SAFETY: `null` is the descriptor opened above, owned here and used no more.
                unsafe { libc::close(null) };
            }
        }
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
        assert_eq!(probe(limit, &[], false), format!("ok {read}"), "at {limit}");
    }
    // Review M1: closed standard streams and the top two descriptors held; the ends move to
    // the lowest free ones from 3, never onto 0, 1 or 2.
    let seen = probe(64, &[62, 63], true);
    let read: i32 = seen.strip_prefix("ok ").expect(&seen).parse().unwrap();
    assert!((3..62).contains(&read), "{seen}");
    // Review L1: under a limit that is large enough, descriptors in use are not reported as
    // a small limit.
    let held: Vec<libc::c_int> = (3..62).collect();
    let seen = probe(64, &held, false);
    assert!(seen.starts_with("err true "), "{seen}");
    assert!(
        seen.contains("needs 4") && !seen.contains("too small"),
        "{seen}"
    );
}
