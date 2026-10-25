//! The security timing oracles (#1145): real-clock tests that a timing channel stays closed,
//! each running native programs that differ only in `secret` input and requiring the same
//! observable outcome.
//!
//! Every test in this file is a security oracle, and every one must be `#[ignore]`d with the
//! reason below: the ordinary `cargo test` run shares the machine with other tests, where a real
//! clock measures the load. They run only in two named steps, one at a time, and nothing else
//! runs them:
//!
//! - the step "Timing channels (security oracles)" of `scripts/local-ci.sh`, on a native host
//!   (macOS, `--linux-arm64`), skipped visibly under emulation, where the real clock measures
//!   the emulator;
//! - the release gate `timing-channels` of `scripts/release-gates.sh` (`docs/release-gates.md`).
//!
//! Both run `cargo test --locked -p una-cli --test timing_channels -- --ignored
//! --test-threads=1`.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/native_build.rs"]
mod native_build;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use native_build::{root, una};

/// The extra time, in microseconds, past which the judge of the placement test holds its third
/// answer beyond the deadline: the first `public` block took that much longer than the second,
/// the same block run right after it in the same iteration.
const JUDGED_EXTRA_US: u128 = 2000;

/// The judge of the placement test: a server on `127.0.0.1` that notes when `/a`, `/b` and `/c`
/// arrive and answers `/a` and `/b` at once. It answers `/c` at once when the block between `/a`
/// and `/b` took at most [`JUDGED_EXTRA_US`] longer than the same block between `/b` and `/c`,
/// and otherwise holds it for 5 s, past the run's deadline, so the run's request fails with
/// `TimedOut`. Each run compares its two blocks with each other, on the same machine at the
/// same moment, so no threshold depends on the machine's speed.
fn judging_server() -> u16 {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let marks: Arc<Mutex<(Option<Instant>, Option<Instant>)>> = Arc::new(Mutex::new((None, None)));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let marks = Arc::clone(&marks);
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => request.push(byte[0]),
                        _ => break,
                    }
                }
                let now = Instant::now();
                let request = String::from_utf8_lossy(&request).into_owned();
                let path = request.split(' ').nth(1).unwrap_or("").to_owned();
                let hold = {
                    let mut marks = marks.lock().unwrap();
                    match path.as_str() {
                        "/a" => {
                            *marks = (Some(now), None);
                            false
                        }
                        "/b" => {
                            marks.1 = Some(now);
                            false
                        }
                        _ => match *marks {
                            (Some(a), Some(b)) => {
                                let first = b.duration_since(a).as_micros();
                                let second = now.duration_since(b).as_micros();
                                first > second + JUDGED_EXTRA_US
                            }
                            _ => true,
                        },
                    }
                };
                if hold {
                    std::thread::sleep(Duration::from_secs(5));
                }
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            });
        }
    });
    port
}

/// The package of the placement test, with `key` as its `secret` input, built.
fn placement_package(key: u8, port: u16) -> PathBuf {
    let get = |path: &str| {
        let url = format!("http://127.0.0.1:{port}/{path}");
        format!(r#"attempt @http.get("{url}", headers: [], key_name: None)"#)
    };
    let (a, b, c) = (get("a"), get("b"), get("c"));
    let program = format!(
        r#"module flip

import std.time (millisecond)

pub main() -> Unit
  uses @http: Http[get]
  diverges
=
  let key: secret Int = {key}
  let parts = 64 + 448 * key
  let earlier = together for i in 0..<parts
    i
  for ever within 1000 * millisecond
    let at_a = {a}
    let first = together for i in 0..<448
      i
    let at_b = {b}
    let second = together for i in 0..<448
      i
    let at_c = {c}
    stop
  unit
"#
    );
    let dir = std::env::temp_dir().join(format!("una-native-flip-{key}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita flip\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry flip\n  grants\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/flip.una"), program).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}

/// Whether one run of the placement test met its deadline: its third request, which the judge
/// holds when the first block took longer than the second, failed with `TimedOut`.
fn timed_out(dir: &Path, run: usize) -> bool {
    let trace = dir.join(format!("trace-{run}.json"));
    let native = Command::new(dir.join("target/una/flip"))
        .args(["--workers", "512", "--http-timeout", "30", "--trace"])
        .arg(&trace)
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    let events: Value = serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
    let _ = std::fs::remove_file(&trace);
    let gets: Vec<&Value> = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["op"] == "get")
        .collect();
    assert_eq!(gets.len(), 3, "{events}");
    // The first two requests are answered at once, well inside the deadline.
    for get in &gets[..2] {
        assert!(get["outcome"].get("variant").is_none(), "{events}");
    }
    gets[2]["outcome"]["variant"] == "TimedOut"
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn a_public_block_s_timing_never_depends_on_how_many_tasks_secret_code_started() {
    // #1145, `spec/27 §1` rules 2 and 4, ADR 0116 IF6 and amendment P8, on the real clock. The
    // same program runs with its `secret` key 0 and 1, and nothing else changed: the key decides
    // whether a block starts 64 or 512 tasks before the loop. In the loop, under a deadline, the
    // same `public` block of 448 tasks runs twice, with a request before, between and after.
    // The judge (`judging_server`) holds the last request past the deadline when the first block
    // took more than 2 ms longer than the second, so the trace records whether it did.
    //
    // How it failed before: one round-robin counter for the whole run placed tasks, and each
    // worker's thread started at its first job. With `--workers 512`, key 1 left the counter at
    // 512, so both blocks ran on workers whose threads had started. Key 0 left it at 64, so the
    // first block went to workers 64 to 511, and the fork of each of its 448 tasks started a
    // thread and waited for it, inside the deadline, while the second block found them started:
    // key 0 timed out in every run, key 1 in none. Both blocks now run on workers 0 to 447, and
    // every thread starts before `main`, so neither key times out. Before either key's earlier
    // block the stack pool is full (64 stacks) for both keys, so its warmth is no difference.
    let port = judging_server();
    let dirs = [placement_package(0, port), placement_package(1, port)];
    for run in 0..10 {
        for (key, dir) in dirs.iter().enumerate() {
            // A run that times out runs twice more and counts only if every try timed out, so
            // a pause of the machine cannot fail the test (#341); the cost that old placement
            // added is in every try.
            let mut tries = (0..3).map(|attempt| timed_out(dir, run * 3 + attempt));
            assert!(
                !tries.all(|timed_out| timed_out),
                "key {key}, run {run}: the first `public` block took longer than the second in \
                 three tries in a row, so how many tasks secret code started changed its timing"
            );
        }
    }
    for dir in dirs {
        let _ = std::fs::remove_dir_all(&dir);
    }
}
