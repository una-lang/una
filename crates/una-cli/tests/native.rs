//! The differential test of the native backend (`spec/16 §5`, ADR 0055 NB-4): every runnable
//! program of the repository's suites runs in the interpreter (`una run --json`) and as a
//! native executable (`una build`), and their output, exit status and trace must be identical.
//! A program that starts tasks is compared under the equivalence of `spec/19 §8` rule 4 instead
//! ([`task_differences`]).
//!
//! The suites are the examples, the conformance cases that run, and the adversarial programs of
//! the reviews that have `pub main`. A program is runnable when the interpreter runs it.
//! The backend is delivered in slices (#26): a runnable program that `una build` cannot compile
//! yet must be listed in `tests/native/unsupported.txt`, and a listed program that compiles fails
//! the test until it leaves the list, so the list stays exact.
//!
//! Each program is a test of its own (#1652), `interpreter::<name>` and `linkers::<name>`,
//! generated from the list that `una-test-cases` builds from the corpus;
//! [`the_generated_programs_are_the_runnable_programs_on_disk`] holds that list to the one on
//! disk, and the tests of the lists' exactness hold each list to the programs.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/case_lists.rs"]
mod case_lists;
#[path = "common/cpu.rs"]
mod cpu;
#[path = "common/hermetic.rs"]
mod hermetic;
#[path = "common/native_build.rs"]
mod native_build;
#[path = "common/timing.rs"]
mod timing;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

use native_build::{root, runtime, una};

/// A fresh copy of `source`, a package directory or a single file, in a temporary directory:
/// builds and proof caches never touch the repository.
fn copy(source: &Path, scratch: &Path) -> PathBuf {
    let target = scratch.join(source.file_name().unwrap());
    let status = Command::new("cp")
        .arg("-R")
        .arg(source)
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success());
    target
}

/// The runnable programs whose `una run` is rejected, by repository path with a reason
/// (`tests/native/rejected.txt`, #1045): a rejected case drops out of the comparison, so one
/// that is not listed is a failure, and so is a listed one that runs. The list must be exact.
fn rejected() -> BTreeMap<String, String> {
    let list = root().join("tests/native/rejected.txt");
    std::fs::read_to_string(list)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let parts = line.split_once(':');
            assert!(
                parts.is_some(),
                "tests/native/rejected.txt: `{line}` has no `: reason`"
            );
            let (name, reason) = parts.unwrap();
            (name.trim().to_owned(), reason.trim().to_owned())
        })
        .collect()
}

/// The programs listed as not compiled yet, by repository path.
fn unsupported() -> BTreeSet<String> {
    let list = root().join("tests/native/unsupported.txt");
    std::fs::read_to_string(list)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect()
}

/// The programs of the generated list, as [`case_lists::native_programs`] gives them.
fn generated() -> Vec<case_lists::Program> {
    macro_rules! programs {
        ($($name:ident => $path:literal [$($grant:literal),*],)*) => {
            vec![$(case_lists::Program {
                path: $path.to_owned(),
                grants: vec![$($grant.to_owned()),*],
            }),*]
        };
    }
    una_test_cases::native_programs!(programs)
}

/// A fresh scratch directory of one program's test, `kind` naming the differential and `test`
/// the generated test, unique among the programs.
fn scratch(kind: &str, test: &str) -> PathBuf {
    let scratch = std::env::temp_dir().join(format!("una-{kind}-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    scratch
}

/// The copy of the program at `name` under `dir`: the directory it runs in and the target it
/// names, `.` for a package.
fn program_copy(name: &str, dir: &Path) -> (PathBuf, String) {
    std::fs::create_dir_all(dir).unwrap();
    let copied = copy(&root().join(name), dir);
    if copied.is_file() {
        (
            dir.to_path_buf(),
            copied.file_name().unwrap().to_string_lossy().into_owned(),
        )
    } else {
        (copied.clone(), ".".to_owned())
    }
}

/// One runnable program runs natively as in the interpreter: `test` is its generated test's
/// name, `name` its path relative to the repository root, and `grants` the grants it runs with.
fn runs_natively_as_in_the_interpreter(test: &str, name: &str, grants: &[&str]) {
    runtime();
    let listed = unsupported();
    let rejected_list = rejected();
    let scratch = scratch("native", test);
    // A copied single file keeps edition 0.1 whatever lies above the temporary directory
    // (#1560).
    hermetic::seal(&scratch);
    let mut failures = Vec::new();
    let mut compiled = false;
    let dir = scratch.join("0");
    let (work, target) = program_copy(name, &dir);
    let mut run_args = vec!["--json", "run", target.as_str()];
    for grant in grants {
        run_args.push("--grant");
        run_args.push(grant);
    }
    let run = una(&work, &run_args);
    // A case that `una run` rejects, or whose document does not parse, is compared with
    // nothing, so it is a failure unless tests/native/rejected.txt lists it with a reason
    // (#1045); a listed case that runs must leave the list.
    let document = serde_json::from_slice::<Value>(&run.stdout).ok();
    let is_rejected = document
        .as_ref()
        .is_none_or(|document| document["status"] == "rejected");
    'compare: {
        if is_rejected {
            if !rejected_list.contains_key(name) {
                failures.push(format!(
                    "{name}: una run rejected it, so nothing is compared, and it is not listed in tests/native/rejected.txt: {}",
                    String::from_utf8_lossy(&run.stderr).trim()
                ));
            }
            // A rejected program is neither compiled nor not compiled.
            if listed.contains(name) {
                failures.push(format!(
                    "{name}: listed in tests/native/unsupported.txt, but not a runnable program"
                ));
            }
            break 'compare;
        }
        // INVARIANT: a document that is not rejected parsed.
        let document = document.unwrap_or(Value::Null);
        if let Some(reason) = rejected_list.get(name) {
            failures.push(format!(
                "{name}: runs now; remove it from tests/native/rejected.txt (listed as: {reason})"
            ));
        }
        let exe = dir.join("native-exe");
        let built = una(
            &work,
            &[
                "--json",
                "build",
                target.as_str(),
                "--out",
                exe.to_str().unwrap(),
            ],
        );
        let stderr = String::from_utf8_lossy(&built.stderr);
        if !built.status.success() {
            if stderr.contains("the native backend does not compile") {
                if !listed.contains(name) {
                    failures.push(format!(
                        "{name}: runnable but not compiled, and not listed in tests/native/unsupported.txt: {}",
                        stderr.trim()
                    ));
                }
            } else {
                failures.push(format!("{name}: una build failed: {}", stderr.trim()));
                // A program whose build fails otherwise is neither compiled nor not compiled.
                if listed.contains(name) {
                    failures.push(format!(
                        "{name}: listed in tests/native/unsupported.txt, but not a runnable program"
                    ));
                }
            }
            break 'compare;
        }
        compiled = true;
        if listed.contains(name) {
            failures.push(format!(
                "{name}: compiles now; remove it from tests/native/unsupported.txt"
            ));
        }
        let trace = dir.join("trace.json");
        let mut native = Command::new(&exe);
        native.current_dir(&work).arg("--trace").arg(&trace);
        for grant in grants {
            native.arg("--grant").arg(grant);
        }
        let native = native.output().unwrap();
        let expected_stdout: String = document["output"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| format!("{}\n", line.as_str().unwrap()))
            .collect();
        if native.status.code() != run.status.code() {
            failures.push(format!(
                "{name}: exit status differs: interpreter {:?}, native {:?}",
                run.status.code(),
                native.status.code()
            ));
        }
        let native_trace: Value = std::fs::read(&trace)
            .ok()
            .and_then(|t| serde_json::from_slice(&t).ok())
            .unwrap_or(Value::Null);
        if starts_tasks(&document["trace"]) || starts_tasks(&native_trace) {
            let stdout = String::from_utf8_lossy(&native.stdout);
            failures.extend(
                task_differences(&stdout, &document["trace"], &native_trace)
                    .into_iter()
                    .map(|difference| format!("{name}: {difference}")),
            );
            break 'compare;
        }
        if String::from_utf8_lossy(&native.stdout) != expected_stdout {
            failures.push(format!(
                "{name}: stdout differs: interpreter {expected_stdout:?}, native {:?}",
                String::from_utf8_lossy(&native.stdout)
            ));
        }
        if timeless(native_trace.clone()) != timeless(document["trace"].clone()) {
            failures.push(format!(
                "{name}: trace differs: interpreter {}, native {native_trace}",
                document["trace"]
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    if name == "examples/hello" {
        assert!(compiled, "examples/hello must compile: {failures:#?}");
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The internal linker against the system `cc` (ADR 0124 rule 33, "Per program"): a runnable
/// program that compiles is built by both, from the same `una`, and the two executables run
/// with the same grants give the same output, failure, exit status and trace. A program that
/// starts tasks is compared by its exit status alone (see below). A program that neither
/// builds must be one that `tests/native/unsupported.txt` or `tests/native/rejected.txt`
/// lists, which [`runs_natively_as_in_the_interpreter`] holds exact: so a failure of both
/// linkers on a program that compiles never passes unseen, as the count of compared programs
/// guarded before #1652 ([`the_linkers_compare_more_than_a_hundred_programs`]).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn runs_alike_linked_by_una_link_and_by_cc(test: &str, name: &str, grants: &[&str]) {
    runtime();
    let scratch = scratch("linkers", test);
    // A copied single file keeps edition 0.1 whatever lies above the temporary directory
    // (#1560).
    hermetic::seal(&scratch);
    let mut failures = Vec::new();
    let dir = scratch.join("0");
    let (work, target) = program_copy(name, &dir);
    let mut outcomes = Vec::new();
    for linker in ["cc", "internal"] {
        let exe = dir.join(format!("exe-{linker}"));
        let built = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .env("NO_COLOR", "1")
            .env("UNA_LINKER", linker)
            .args(["build", target.as_str(), "--out", exe.to_str().unwrap()])
            .current_dir(&work)
            .output()
            .unwrap();
        if !built.status.success() {
            outcomes.push(None);
            continue;
        }
        let trace = dir.join(format!("trace-{linker}.json"));
        let mut run = Command::new(&exe);
        run.current_dir(&work).arg("--trace").arg(&trace);
        for grant in grants {
            run.arg("--grant").arg(grant);
        }
        let output = run.output().unwrap();
        let trace: Value = std::fs::read(&trace)
            .ok()
            .and_then(|t| serde_json::from_slice(&t).ok())
            .unwrap_or(Value::Null);
        outcomes.push(Some((output, trace)));
    }
    match (&outcomes[0], &outcomes[1]) {
        (None, None) => {
            if !unsupported().contains(name) && !rejected().contains_key(name) {
                failures.push(format!(
                    "{name}: neither cc nor the internal linker links it, and neither tests/native/unsupported.txt nor tests/native/rejected.txt lists it"
                ));
            }
        }
        (Some(_), None) => {
            failures.push(format!(
                "{name}: cc links it and the internal linker does not"
            ));
        }
        (None, Some(_)) => {
            failures.push(format!(
                "{name}: the internal linker links it and cc does not"
            ));
        }
        (Some((cc, cc_trace)), Some((internal, internal_trace))) => {
            if cc.status.code() != internal.status.code() {
                failures.push(format!(
                    "{name}: exit status differs: cc {:?}, internal {:?}",
                    cc.status.code(),
                    internal.status.code()
                ));
            }
            // Two native runs that start tasks may interleave and cancel differently by
            // design (`spec/19 §8`): each is held to the interpreter's run by
            // `runs_natively_as_in_the_interpreter`, and only their exit status is compared
            // here.
            if !(starts_tasks(cc_trace) || starts_tasks(internal_trace)) {
                if cc.stdout != internal.stdout {
                    failures.push(format!(
                        "{name}: stdout differs: cc {:?}, internal {:?}",
                        String::from_utf8_lossy(&cc.stdout),
                        String::from_utf8_lossy(&internal.stdout)
                    ));
                }
                if cc.stderr != internal.stderr {
                    failures.push(format!(
                        "{name}: stderr differs: cc {:?}, internal {:?}",
                        String::from_utf8_lossy(&cc.stderr),
                        String::from_utf8_lossy(&internal.stderr)
                    ));
                }
                if timeless(cc_trace.clone()) != timeless(internal_trace.clone()) {
                    failures.push(format!(
                        "{name}: trace differs: cc {cc_trace}, internal {internal_trace}"
                    ));
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// One test per runnable program in each differential: `interpreter::<name>` and, on
/// aarch64-apple-darwin, `linkers::<name>`.
macro_rules! program_tests {
    ($($name:ident => $path:literal [$($grant:literal),*],)*) => {
        /// Every runnable program runs natively as in the interpreter.
        mod interpreter {
            $(
                #[test]
                fn $name() {
                    super::runs_natively_as_in_the_interpreter(stringify!($name), $path, &[$($grant),*]);
                }
            )*
        }
        /// Every runnable program runs alike linked by una-link and by cc.
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        mod linkers {
            $(
                #[test]
                fn $name() {
                    super::runs_alike_linked_by_una_link_and_by_cc(stringify!($name), $path, &[$($grant),*]);
                }
            )*
        }
    };
}

una_test_cases::native_programs!(program_tests);

/// The generated tests are the runnable programs on disk, path for path and grant for grant: a
/// program added, removed or changed without a rebuild of `una-test-cases` fails here, never
/// goes untested.
#[test]
fn the_generated_programs_are_the_runnable_programs_on_disk() {
    assert!(root().join("examples").is_dir(), "examples is missing");
    let on_disk = case_lists::native_programs(&root());
    assert!(
        on_disk
            .iter()
            .any(|program| program.path == "examples/hello"),
        "no runnable program found on disk"
    );
    pretty_assertions::assert_eq!(generated(), on_disk);
}

/// Every program that `tests/native/unsupported.txt` or `tests/native/rejected.txt` lists is a
/// runnable program; whether each listed one is still not compiled or still rejected is its own
/// test's.
#[test]
fn the_native_lists_name_runnable_programs_only() {
    let names: BTreeSet<String> = generated()
        .into_iter()
        .map(|program| program.path)
        .collect();
    let mut failures = Vec::new();
    for name in unsupported() {
        if !names.contains(&name) {
            failures.push(format!(
                "{name}: listed in tests/native/unsupported.txt, but not a runnable program"
            ));
        }
    }
    for name in rejected().keys() {
        if !names.contains(name) {
            failures.push(format!(
                "{name}: listed in tests/native/rejected.txt, but not a runnable program"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// More than a hundred programs are expected to build and so are compared by the two linkers:
/// each such program's test fails when neither linker builds it.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn the_linkers_compare_more_than_a_hundred_programs() {
    let listed = unsupported();
    let rejected = rejected();
    let compared = generated()
        .into_iter()
        .filter(|program| !listed.contains(&program.path) && !rejected.contains_key(&program.path))
        .count();
    assert!(compared > 100, "only {compared} programs are compared");
}

/// The internal linker through `una build` (ADR 0124 rules 27 and 29, step 3): the same package
/// built from two directories gives the same bytes; a runtime file the linker refuses is a usage
/// error and leaves no executable and no private directory; `--debuggable` adds the
/// entitlement and nothing else of the program changes.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn una_build_links_by_itself_deterministically_and_fails_closed() {
    runtime();
    let scratch = std::env::temp_dir().join(format!("una-link-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let mut built = Vec::new();
    for place in ["a", "a-much-longer-second-place/nested"] {
        let dir = scratch.join(place);
        std::fs::create_dir_all(&dir).unwrap();
        let package = copy(&root().join("examples/hello"), &dir);
        let out = una(&package, &["build", ".", "--out", "out/hello"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        built.push(std::fs::read(package.join("out/hello")).unwrap());
    }
    assert_eq!(built[0], built[1], "two directories gave two executables");
    // The signature carries the hardened runtime, and only `--debuggable` the entitlement.
    let package = scratch.join("a/hello");
    let out = una(
        &package,
        &["build", ".", "--out", "out/debuggable", "--debuggable"],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let shown = |name: &str| {
        let out = Command::new("codesign")
            .args(["-d", "-vvv", "--entitlements", "-"])
            .arg(package.join("out").join(name))
            .output()
            .unwrap();
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    assert!(shown("hello").contains("flags=0x30002(adhoc,runtime,linker-signed)"));
    assert!(!shown("hello").contains("get-task-allow"));
    assert!(shown("debuggable").contains("com.apple.security.get-task-allow"));
    for name in ["hello", "debuggable"] {
        let run = Command::new(package.join("out").join(name))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, Una\n");
    }
    // A runtime file the linker refuses, in this development build: a usage error, exit code
    // 2, nothing written (a runtime built into `una` would be U8001).
    let broken = scratch.join("broken.a");
    std::fs::write(&broken, b"!<arch>\nnot an archive member").unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .env("UNA_RT_LIB", &broken)
        .args(["build", "."])
        .current_dir(&package)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("the internal linker refuses"),
        "{refused:?}"
    );
    assert!(!package.join("target/una/hello").exists());
    let left: Vec<_> = std::fs::read_dir(package.join("target/una"))
        .unwrap()
        .collect();
    assert!(left.is_empty(), "the refused link left {left:?}");
    let _ = std::fs::remove_dir_all(&scratch);
}

/// Whether a trace has events of tasks (`spec/19 §7` rule 1).
fn starts_tasks(trace: &Value) -> bool {
    trace
        .as_array()
        .is_some_and(|events| events.iter().any(|event| event.get("task").is_some()))
}

/// How a native run that starts tasks departs from the equivalence of `spec/19 §8` rule 4,
/// the exit status being compared already:
/// - the output is what the native trace says each task printed, each task's lines in its own
///   order, those of different tasks interleaved in any order;
/// - the trace equals the interpreter's on an ordered run (`spec/19 §6` rule 7). A run that is
///   not ordered also holds events of tasks that started after a failed one, so its trace holds
///   the interpreter's, in order, and the extra events are those of tasks.
fn task_differences(stdout: &str, interpreted: &Value, native: &Value) -> Vec<String> {
    let mut differences = Vec::new();
    let empty = Vec::new();
    let events = native.as_array().unwrap_or(&empty);
    let printed: Vec<(&str, &str)> = events
        .iter()
        .filter(|event| event["capability"] == "Console" && event["op"] == "print")
        .filter_map(|event| {
            let task = event.get("task").and_then(Value::as_str).unwrap_or("");
            Some((task, event["args"]["line"].as_str()?))
        })
        .collect();
    let lines: Vec<String> = stdout.lines().map(una_core::quote).collect();
    let mut sorted_lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut sorted_printed: Vec<&str> = printed.iter().map(|(_, line)| *line).collect();
    sorted_lines.sort_unstable();
    sorted_printed.sort_unstable();
    // A redacted line is not in the trace: only the order of the others is checked then.
    let redacted = sorted_printed
        .iter()
        .any(|line| line.starts_with("<redacted"));
    if !redacted && sorted_lines != sorted_printed {
        differences.push(format!(
            "the output is not what the native trace printed: output {lines:?}, trace {printed:?}"
        ));
    }
    let tasks: BTreeSet<&str> = printed.iter().map(|(task, _)| *task).collect();
    for task in tasks {
        let own: Vec<&str> = printed
            .iter()
            .filter(|(t, _)| *t == task)
            .map(|(_, line)| *line)
            .collect();
        if !subsequence(&own, &lines.iter().map(String::as_str).collect::<Vec<_>>()) {
            differences.push(format!(
                "the output {lines:?} does not keep the order of the lines of task {task:?}: {own:?}"
            ));
        }
    }
    // Started tasks write their events in the order of real time (`spec/27 §8`): each task's
    // events are compared in their own order, and the tasks are the same.
    if has_started(interpreted) || has_started(native) {
        let (interpreted, native) = (by_task(interpreted), by_task(native));
        if interpreted != native {
            differences.push(format!(
                "the events of each task differ: interpreter {interpreted:?}, native {native:?}"
            ));
        }
        return differences;
    }
    let unnumbered = |trace: &Value| -> Vec<Value> {
        let mut trace = timeless(trace.clone());
        let mut events = trace.as_array_mut().map(std::mem::take).unwrap_or_default();
        for event in &mut events {
            if let Some(event) = event.as_object_mut() {
                event.remove("seq");
            }
        }
        events
    };
    if timeless(native.clone()) != timeless(interpreted.clone()) {
        let (interpreted, native) = (unnumbered(interpreted), unnumbered(native));
        let extra_of_root = native.len()
            - native.iter().filter(|e| e.get("task").is_some()).count()
            != interpreted.len()
                - interpreted
                    .iter()
                    .filter(|e| e.get("task").is_some())
                    .count();
        if !subsequence(&interpreted, &native) || extra_of_root {
            differences.push(format!(
                "the trace is neither the interpreter's nor one of a run that is not ordered: interpreter {interpreted:?}, native {native:?}"
            ));
        }
    }
    differences
}

/// Whether a trace has events of started tasks, whose paths have a part `s<c>`, or `qs<c>` in
/// quiet code (`spec/27 §7`).
fn has_started(trace: &Value) -> bool {
    trace.as_array().is_some_and(|events| {
        events.iter().any(|event| {
            event["task"].as_str().is_some_and(|path| {
                path.split('.').any(|part| {
                    let part = part.strip_prefix('q').unwrap_or(part);
                    part.strip_prefix('s')
                        .is_some_and(|count| count.parse::<usize>().is_ok())
                })
            })
        })
    })
}

/// The events of a trace by task, each task's in its own order, without `seq`, and with the
/// readings of `Clock.now` set aside.
fn by_task(trace: &Value) -> std::collections::BTreeMap<String, Vec<Value>> {
    let mut tasks: std::collections::BTreeMap<String, Vec<Value>> =
        std::collections::BTreeMap::new();
    let trace = timeless(trace.clone());
    for event in trace.as_array().into_iter().flatten() {
        let task = event["task"].as_str().unwrap_or("").to_owned();
        let mut event = event.clone();
        if let Some(fields) = event.as_object_mut() {
            fields.remove("seq");
        }
        tasks.entry(task).or_default().push(event);
    }
    tasks
}

/// Whether `part` is a subsequence of `whole`.
fn subsequence<T: PartialEq>(part: &[T], whole: &[T]) -> bool {
    let mut rest = whole.iter();
    part.iter().all(|item| rest.any(|other| other == item))
}

/// `trace` with the result of each `Clock.now` replaced: two runs read the clock at different
/// times, and everything else must agree.
fn timeless(mut trace: Value) -> Value {
    if let Some(events) = trace.as_array_mut() {
        for event in events {
            if event["capability"] == "Clock" && event["op"] == "now" {
                event["outcome"]["value"] = Value::from("<now>");
            }
        }
    }
    trace
}

/// A package in a fresh temporary directory, with `una.toml` and the files `(path, text)`.
fn package(name: &str, manifest_extra: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-native-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n{manifest_extra}"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("examples/hello/verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    for (path, text) in files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    dir
}

const HELLO: &str = "module hello\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"Hello, Una\")\n";

#[test]
fn an_empty_grants_list_grants_nothing_to_build() {
    // #925: `grants = []` is the final set, so `una build`, as `una check`, reports U3012 at
    // each host use of `main`, where it once built an executable that could never run.
    let dir = package(
        "empty-grants",
        "\n[run]\nentry = \"hello\"\ngrants = []\n",
        &[("src/hello.una", HELLO)],
    );
    for command in ["check", "build"] {
        let output = una(&dir, &[command]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{command}: {stderr}");
        assert!(stderr.contains("U3012"), "{command}: {stderr}");
        assert!(stderr.contains("src/hello.una:4:3"), "{command}: {stderr}");
    }
    assert!(!dir.join("target/una/hello").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn una_build_writes_the_executable_and_prints_una_build_1() {
    runtime();
    let dir = package(
        "hello",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", HELLO)],
    );
    let output = una(&dir, &["--json", "build"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        document,
        serde_json::json!({
            "schema": "una.build/1",
            "tier": 1,
            "status": "built",
            "target": una_codegen::host_target(),
            "output": "target/una/hello",
            "diagnostics": [],
        })
    );
    let run = Command::new(dir.join("target/una/hello")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, Una\n");
    assert_eq!(run.status.code(), Some(0));
    // Human output names the executable.
    let output = una(&dir, &["build", "--out", "bin/greet"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "built bin/greet\n");
    assert!(dir.join("bin/greet").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn una_build_never_writes_through_a_link_leading_outside_the_package_1160() {
    // #1160: a repository can commit `target/una/<name>.o`, `target/una/<name>` or `target`
    // itself as a link to somewhere outside it. `una build` refuses each before it writes,
    // and before the linker starts, and the file the link leads to stays as it was.
    let dir = package(
        "planted",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", HELLO)],
    );
    let outside = dir.with_extension("outside");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    let victim = outside.join("victim");
    std::fs::write(&victim, "untouched").unwrap();
    let refused = |expected: &str| {
        let output = una(&dir, &["build"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains(expected), "{stderr}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched");
    };
    // The object's name: build outputs are not sources, so any link there is refused.
    std::fs::create_dir_all(dir.join("target/una")).unwrap();
    std::os::unix::fs::symlink(&victim, dir.join("target/una/planted.o")).unwrap();
    refused("is a symbolic link");
    assert!(std::fs::symlink_metadata(dir.join("target/una/planted")).is_err());
    // The executable's name, which the linker would write.
    std::fs::remove_file(dir.join("target/una/planted.o")).unwrap();
    std::os::unix::fs::symlink(&victim, dir.join("target/una/planted")).unwrap();
    refused("is a symbolic link");
    assert!(std::fs::symlink_metadata(dir.join("target/una/planted.o")).is_err());
    // `target` itself: nothing is created outside.
    std::fs::remove_dir_all(dir.join("target")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("target")).unwrap();
    refused("outside the package");
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    // A relative `--out` through that link is refused too: it is rooted at the current
    // directory, here the package.
    let output = una(&dir, &["build", "--out", "target/una/planted"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("outside the package"), "{stderr}");
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    // A relative `--out` with `..` is the user's own typed path, created as given.
    let typed = format!(
        "../{}/typed/planted",
        outside.file_name().unwrap().to_string_lossy()
    );
    let output = una(&dir, &["build", "--out", &typed]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(outside.join("typed/planted").is_file());
    std::fs::remove_dir_all(outside.join("typed")).unwrap();
    // Without links, the build succeeds and leaves only the executable: the linker wrote in a
    // private directory, which is gone.
    std::fs::remove_file(dir.join("target")).unwrap();
    let output = una(&dir, &["build"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let left: Vec<String> = std::fs::read_dir(dir.join("target/una"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["planted"]);
    let run = Command::new(dir.join("target/una/planted"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, Una\n");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn una_build_finds_the_runtime_through_a_symlinked_una() {
    // HOST-5 (#185): `una` installed as a symlink, as a package manager links it into `bin/`,
    // finds `libuna_rt.a` beside the binary the link resolves to, not beside the link. macOS
    // reports the path of the link as the executable's path.
    runtime();
    let dir = package(
        "linked",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", HELLO)],
    );
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let link = bin.join("una");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_una"), &link).unwrap();
    let output = Command::new(&link)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env_remove("UNA_RT_LIB")
        .args(["build"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(dir.join("target/una/linked"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, Una\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The runtime that the test builds' `una` links, beside the binary.
fn built_runtime() -> PathBuf {
    runtime();
    Path::new(env!("CARGO_BIN_EXE_una"))
        .parent()
        .unwrap()
        .join("libuna_rt.a")
}

/// `una build --out <out>` in `dir`, with the runtime built into `una` stood in by `embedded`
/// with the digest `sha256` (the test hook of `runtime_archive`, ADR 0124 step 2), and
/// `UNA_RT_LIB` naming `rt_lib`.
fn build_with_embedded(
    dir: &Path,
    out: &Path,
    embedded: &Path,
    sha256: &str,
    rt_lib: &Path,
) -> Output {
    build_with_embedded_by(
        Path::new(env!("CARGO_BIN_EXE_una")),
        dir,
        out,
        embedded,
        sha256,
        rt_lib,
    )
}

/// [`build_with_embedded`], run by the `una` at `una`.
fn build_with_embedded_by(
    una: &Path,
    dir: &Path,
    out: &Path,
    embedded: &Path,
    sha256: &str,
    rt_lib: &Path,
) -> Output {
    Command::new(una)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env("UNA_TEST_EMBEDDED_RUNTIME", embedded)
        .env("UNA_TEST_EMBEDDED_RUNTIME_SHA256", sha256)
        .env("UNA_RT_LIB", rt_lib)
        .env("NO_COLOR", "1")
        .args(["build", "--out"])
        .arg(out)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

/// A `una` placed at `install/bin/una`, a hard link to (or else a copy of) the test build's, with
/// a file that is no runtime at each place a development build looks for one (ADR 0059 rule 2
/// and its amendment): `bin/lib/libuna_rt.a`, `bin/libuna_rt.a` and `lib/libuna_rt.a`.
fn una_beside_junk_runtimes(install: &Path) -> PathBuf {
    let bin = install.join("bin");
    std::fs::create_dir_all(bin.join("lib")).unwrap();
    std::fs::create_dir_all(install.join("lib")).unwrap();
    let una = bin.join("una");
    if std::fs::hard_link(env!("CARGO_BIN_EXE_una"), &una).is_err() {
        std::fs::copy(env!("CARGO_BIN_EXE_una"), &una).unwrap();
    }
    for junk in [
        bin.join("lib/libuna_rt.a"),
        bin.join("libuna_rt.a"),
        install.join("lib/libuna_rt.a"),
    ] {
        std::fs::write(junk, "not a runtime\n").unwrap();
    }
    una
}

#[test]
fn an_embedded_runtime_is_linked_and_una_rt_lib_has_no_effect() {
    // ADR 0124 rules 14 and 15 (step 2): a `una` with its runtime built in links that runtime,
    // checked against its digest, and reads no variable and no file for it: `UNA_RT_LIB`
    // naming a file that is no runtime at all, and such a file at each place a development
    // build looks beside `una`, change nothing, so a runtime older than `una` can no longer
    // fail the link (#203). The same `una` without the stand-in reads those files and fails, so
    // they are where it would look (#1359, L5). The runtime goes into the link's private
    // directory and leaves with it. The executable is the one a development build links from
    // the same runtime file.
    let built = built_runtime();
    let dir = package(
        "embedded",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", HELLO)],
    );
    let bytes = std::fs::read(&built).unwrap();
    let embedded = dir.join("embedded-runtime.a");
    std::fs::write(&embedded, &bytes).unwrap();
    let junk = dir.join("not-a-runtime.a");
    std::fs::write(&junk, "not a runtime\n").unwrap();
    let una = una_beside_junk_runtimes(&dir.join("install"));
    let out = dir.join("out");
    let output = build_with_embedded_by(
        &una,
        &dir,
        &out.join("hello"),
        &embedded,
        &sha256_hex(&bytes),
        &junk,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(out.join("hello")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, Una\n");
    // The private directory, with the runtime written there, is gone.
    let left: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["hello"]);
    // The same `una` as a development build, with no stand-in and no `UNA_RT_LIB`, reads the
    // junk beside it, and its link fails: the files above are where it looks.
    let development = Command::new(&una)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env_remove("UNA_TEST_EMBEDDED_RUNTIME")
        .env_remove("UNA_RT_LIB")
        .args(["build", "--out"])
        .arg(dir.join("dev-beside/hello"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(development.status.code(), Some(2), "{development:?}");
    // A development build reads `UNA_RT_LIB`: the junk file fails its link.
    let development = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env_remove("UNA_TEST_EMBEDDED_RUNTIME")
        .env("UNA_RT_LIB", &junk)
        .args(["build", "--out"])
        .arg(dir.join("dev/hello"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(development.status.code(), Some(2), "{development:?}");
    // And, from the same runtime file, it links the same executable.
    let development = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env_remove("UNA_TEST_EMBEDDED_RUNTIME")
        .env("UNA_RT_LIB", &embedded)
        .args(["build", "--out"])
        .arg(dir.join("dev/hello"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(development.status.code(), Some(0), "{development:?}");
    assert!(
        std::fs::read(out.join("hello")).unwrap() == std::fs::read(dir.join("dev/hello")).unwrap(),
        "the embedded and the file runtime link different executables"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_report_names_the_digest_of_the_embedded_runtime() {
    // ADR 0124 rule 28 (#1359, L5): the report of a `una` with its runtime built in names that
    // runtime's recorded digest, `native.runtime_sha256` in JSON and a `runtime sha256:` line
    // in text, the digest its links check; a development build's report names none.
    // scripts/check-release.sh compares the release `una`'s with the file it embedded.
    let built = built_runtime();
    let dir = package(
        "report-runtime",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", HELLO)],
    );
    let bytes = std::fs::read(&built).unwrap();
    let embedded = dir.join("embedded-runtime.a");
    std::fs::write(&embedded, &bytes).unwrap();
    let digest = sha256_hex(&bytes);
    let report = |json: bool, stand_in: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
        command
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .env_remove("UNA_TEST_EMBEDDED_RUNTIME")
            .env_remove("UNA_TEST_EMBEDDED_RUNTIME_SHA256")
            .env("NO_COLOR", "1")
            .current_dir(&dir);
        if stand_in {
            command
                .env("UNA_TEST_EMBEDDED_RUNTIME", &embedded)
                .env("UNA_TEST_EMBEDDED_RUNTIME_SHA256", &digest);
        }
        if json {
            command.arg("--json");
        }
        let output = command.arg("report").output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let json = report(true, true);
    assert!(
        json.contains(&format!("\"runtime_sha256\":\"{digest}\"")),
        "{json}"
    );
    let text = report(false, true);
    assert!(
        text.contains(&format!("  runtime sha256: {digest}\n")),
        "{text}"
    );
    assert!(!report(true, false).contains("runtime_sha256"));
    assert!(!report(false, false).contains("runtime sha256"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_or_damaged_embedded_runtime_fails_closed_with_u8001() {
    // ADR 0124 step 2: `una build` checks the runtime built into `una` against the digest its
    // build recorded before every link. A runtime that is missing, or whose bytes differ by
    // one bit, is U8001 (exit code 3): nothing is linked, no executable is written, and the
    // private directory is removed. `UNA_RT_LIB` naming a good runtime is no way around it.
    let built = built_runtime();
    let dir = package(
        "damaged",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", HELLO)],
    );
    let bytes = std::fs::read(&built).unwrap();
    let recorded = sha256_hex(&bytes);
    let mut flipped = bytes;
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0x01;
    let damaged = dir.join("damaged-runtime.a");
    std::fs::write(&damaged, &flipped).unwrap();
    let cases = [
        (damaged, "does not match the digest recorded at its build"),
        (dir.join("no-such-runtime.a"), "is missing"),
    ];
    for (embedded, expected) in cases {
        let out = dir.join("out");
        let output = build_with_embedded(&dir, &out.join("hello"), &embedded, &recorded, &built);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(3), "{stderr}");
        assert!(
            stderr.starts_with(
                "internal compiler error (U8001): the native runtime built into `una`"
            ),
            "{stderr}"
        );
        assert!(stderr.contains(expected), "{stderr}");
        assert!(stderr.contains("reinstall"), "{stderr}");
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0, "{out:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_executable_does_not_depend_on_where_una_and_its_runtime_are_installed() {
    // #382 (S-13): a built executable carries no path of the machine that built it. The link
    // used to keep the absolute path of `libuna_rt.a` in the macOS debug map (an `OSO` entry
    // per object), so an executable named its builder's install directory, home included, and
    // two installs made different bytes. The same program, linked with the same runtime from
    // two install directories, gives the same executable, which names neither.
    runtime();
    let dir = package(
        "placed",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", HELLO)],
    );
    let built = Path::new(env!("CARGO_BIN_EXE_una"))
        .parent()
        .unwrap()
        .join("libuna_rt.a");
    let mut executables = Vec::new();
    for install in ["first-install/lib", "a-second-install-elsewhere/lib"] {
        let lib = dir.join(install);
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::copy(&built, lib.join("libuna_rt.a")).unwrap();
        let out = dir.join(install).join("out");
        let output = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .env("UNA_RT_LIB", lib.join("libuna_rt.a"))
            .args(["build", "--out"])
            .arg(out.join("hello"))
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let run = Command::new(out.join("hello")).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, Una\n");
        executables.push(std::fs::read(out.join("hello")).unwrap());
    }
    assert!(
        executables[0] == executables[1],
        "the executables differ by where the runtime was installed"
    );
    let text = String::from_utf8_lossy(&executables[0]);
    for path in [
        dir.display().to_string(),
        built.parent().unwrap().display().to_string(),
    ] {
        assert!(!text.contains(&path), "the executable names {path}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn una_build_rejects_a_package_that_does_not_check() {
    let dir = package(
        "broken",
        "\n[run]\nentry = \"broken\"\ngrants = [\"console\"]\n",
        &[(
            "src/broken.una",
            "module broken\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(1)\n",
        )],
    );
    let output = una(&dir, &["--json", "build"]);
    assert_eq!(output.status.code(), Some(1));
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["status"], "rejected");
    assert_eq!(document["output"], Value::Null);
    assert_eq!(document["diagnostics"][0]["code"], "U2001");
    assert!(!dir.join("target/una/broken").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A function named as a value runs on the bindings where it is named (E-Lam, `spec/11 §4`). Named
/// where no `@log` is bound, and called under a handler of the callee's own, it once checked
/// and stopped `una build` with an internal error; it is U3001 at the name now.
#[test]
fn una_build_rejects_a_function_value_named_without_its_bindings() {
    let dir = package(
        "noref",
        "\n[run]\nentry = \"noref\"\ngrants = [\"console\"]\n",
        &[(
            "src/noref.una",
            "module noref\n\ncapability Log\n  op write(n: Int) -> Unit\n\nhandler quiet() for Log\n  op write(n) = unit\n\nrunner(f: fn(Int) -> Unit uses @log: Log[write]) -> Unit\n=\n  f(1) with @log = quiet()\n\nwrite_it(n: Int) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(n)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  runner(f: write_it)\n  @console.print(\"ran\")\n",
        )],
    );
    let output = una(&dir, &["--json", "build"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["status"], "rejected");
    let codes: Vec<&Value> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| &d["code"])
        .collect();
    assert_eq!(codes, [&Value::from("U3001")], "{document}");
    assert!(!dir.join("target/una/noref").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failure_of_main_shows_what_una_run_shows() {
    let dir = package(
        "stop",
        "\n[run]\nentry = \"stop\"\n",
        &[(
            "src/stop.una",
            "module stop\n\npub kind Pair\n  name: Text\n  pin: secret Int\n\npub kind Stop = | Halt(reason: Text, inner: Option[Pair], code: secret Int, size: Int)\n\npub main() -> Unit\n  fails Stop\n=\n  let pairs = [Pair(name: \"ana\", pin: 7)]\n  fail Halt(reason: \"x\", inner: pairs.first(), code: 1, size: pairs.len)\n",
        )],
    );
    let message = "`main` failed with `Halt(reason: \"x\", inner: Some(Pair(name: \"ana\", pin: <redacted:secret>)), size: 1)`";
    let run = una(&dir, &["run"]);
    assert_eq!(run.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&run.stderr).contains(message));
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/stop"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&native.stderr);
    assert!(
        stderr.starts_with(&format!(
            "[U6001] Error: {message}\n  at src/stop.una:9:5\n"
        )),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failure_of_main_is_described_by_the_kind_the_checker_resolved() {
    // Two modules declare a kind `Stop`; `main` fails with the one whose field is secret, and
    // has no `fail` of its own, so only the resolved kind tells them apart.
    let dir = package(
        "twins",
        "\n[run]\nentry = \"app\"\n",
        &[
            (
                "src/alpha.una",
                "module alpha\n\npub kind Stop = | Halt(code: Int)\n\npub halt_alpha() -> Unit\n  fails Stop\n=\n  fail Halt(code: 1)\n",
            ),
            (
                "src/beta.una",
                "module beta\n\npub kind Stop = | Halt(code: secret Int)\n\npub halt_beta() -> Unit\n  fails Stop\n=\n  fail Halt(code: 4242)\n",
            ),
            (
                "src/app.una",
                "module app\n\nimport beta (Stop, halt_beta)\n\npub main() -> Unit\n  fails Stop\n=\n  halt_beta()\n    if error pass\n",
            ),
        ],
    );
    let message = "`main` failed with `Halt`";
    let run = una(&dir, &["run"]);
    assert_eq!(run.status.code(), Some(1));
    let run_stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run_stderr.contains(message), "{run_stderr}");
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/twins"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&native.stderr);
    assert!(
        stderr.starts_with(&format!("[U6001] Error: {message}\n")),
        "{stderr}"
    );
    assert!(!stderr.contains("4242"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wide_range_over_the_budget_is_u6003_in_both_backends() {
    // #773: a `together for` over a range makes its list before its tasks, and the
    // interpreter checks the memory budget at each element. Native code did not, so a wide
    // range grew past the allocator's hard ceiling and the executable aborted on a signal where
    // `una run` reported U6003. Both now end the same way, at the loop.
    let program = "module wide\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let n: I64 = 5000000\n  let xs = together for i in 0..<n\n    i\n  @console.print(\"{xs.len}\")\n";
    let dir = package(
        "wide",
        "\n[run]\nentry = \"wide\"\ngrants = [\"console\"]\n",
        &[("src/wide.una", program)],
    );
    // `together` is a construct of edition 0.2.
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"wide\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"wide\"\ngrants = [\"console\"]\n",
    )
    .unwrap();
    let expected = "[U6003] Error: this call exceeds the memory limit of 8 MiB\n";
    let run = una(&dir, &["run", "--max-memory", "8"]);
    let run_stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "{run_stderr}");
    assert!(run_stderr.starts_with(expected), "{run_stderr}");
    assert!(run_stderr.contains("src/wide.una:7:12"), "{run_stderr}");
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/wide"))
        .args(["--max-memory", "8"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&native.stderr);
    assert_eq!(native.status.code(), Some(1), "{stderr}");
    assert!(stderr.starts_with(expected), "{stderr}");
    assert!(stderr.contains("src/wide.una:7:12"), "{stderr}");
    assert_eq!(native.stdout, run.stdout);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_declared_budget_holds_in_both_backends_and_the_flag_only_lowers_it() {
    // ADR 0109 rule 4: `[run] max_memory` is the budget of `una run`, `una test` and the
    // executable `una build` makes, and `--max-memory` may lower it, never raise it. The
    // program holds more than any of these budgets, so U6003 names the budget in force.
    let program = "module budget\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let n: I64 = 5000000\n  let xs = together for i in 0..<n\n    i\n  @console.print(\"{xs.len}\")\n";
    let dir = package("budget", "", &[("src/budget.una", program)]);
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"budget\"\nversion = \"0.1.0\"\nedition = \"0.3\"\nprofile = \"systems\"\n\n[run]\nentry = \"budget\"\ngrants = [\"console\"]\nmax_memory = 8\n",
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = |args: &[&str]| {
        Command::new(dir.join("target/una/budget"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap()
    };
    for (flag, mib) in [(None, 8), (Some("4"), 4), (Some("8"), 8)] {
        let args: Vec<&str> = flag.map_or(Vec::new(), |flag| vec!["--max-memory", flag]);
        let expected = format!("[U6003] Error: this call exceeds the memory limit of {mib} MiB\n");
        let run = una(&dir, &[&["run"][..], &args].concat());
        let run_stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(1), "{flag:?}: {run_stderr}");
        assert!(run_stderr.starts_with(&expected), "{flag:?}: {run_stderr}");
        let native = native(&args);
        let stderr = String::from_utf8_lossy(&native.stderr);
        assert_eq!(native.status.code(), Some(1), "{flag:?}: {stderr}");
        assert!(stderr.starts_with(&expected), "{flag:?}: {stderr}");
    }
    let raise = "error: --max-memory 16 is above the package's declared max_memory of 8 MiB; it may lower the budget, never raise it\n";
    let run = una(&dir, &["run", "--max-memory", "16"]);
    assert_eq!(run.status.code(), Some(2), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stderr), raise);
    assert!(run.stdout.is_empty());
    let test = una(&dir, &["test", "--max-memory", "16"]);
    assert_eq!(test.status.code(), Some(2), "{test:?}");
    assert_eq!(String::from_utf8_lossy(&test.stderr), raise);
    assert!(test.stdout.is_empty());
    let lowered = una(&dir, &["test", "--max-memory", "4"]);
    assert_ne!(lowered.status.code(), Some(2), "{lowered:?}");
    let native = native(&["--max-memory", "16"]);
    assert_eq!(native.status.code(), Some(2), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stderr), raise);
    assert!(native.stdout.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_budget_declared_in_ita_una_holds_in_both_backends() {
    // ADR 0109 rule 4 in edition 0.4: the `run` line `max_memory` is the same budget, carried
    // into the executable's description.
    let program = "module budget\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let n: I64 = 5000000\n  let xs = together for i in 0..<n\n    i\n  @console.print(\"{xs.len}\")\n";
    let dir = package("budget_ita", "", &[("src/budget.una", program)]);
    std::fs::remove_file(dir.join("una.toml")).unwrap();
    std::fs::remove_file(dir.join("verification.lock")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita budget\n  version 0.1.0\n  edition 0.4\n  profile systems\n\nrun\n  entry budget\n  max_memory 8\n  grants\n    console\n",
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/project/profile_systems_budget/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let expected = "[U6003] Error: this call exceeds the memory limit of 8 MiB\n";
    let run = una(&dir, &["run"]);
    let run_stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "{run_stderr}");
    assert!(run_stderr.starts_with(expected), "{run_stderr}");
    let native = |args: &[&str]| {
        Command::new(dir.join("target/una/budget"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap()
    };
    let run_native = native(&[]);
    let stderr = String::from_utf8_lossy(&run_native.stderr);
    assert_eq!(run_native.status.code(), Some(1), "{stderr}");
    assert!(stderr.starts_with(expected), "{stderr}");
    let raise = "error: --max-memory 9 is above the package's declared max_memory of 8 MiB; it may lower the budget, never raise it\n";
    let run = una(&dir, &["run", "--max-memory", "9"]);
    assert_eq!(run.status.code(), Some(2), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stderr), raise);
    let raised = native(&["--max-memory", "9"]);
    assert_eq!(raised.status.code(), Some(2), "{raised:?}");
    assert_eq!(String::from_utf8_lossy(&raised.stderr), raise);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_declared_budget_the_flag_keeps_its_meaning() {
    // ADR 0109 rule 4: without the key, `--max-memory 8192` is accepted as before, and the
    // executable's description carries no budget.
    let dir = package(
        "budget_none",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", HELLO)],
    );
    let run = una(&dir, &["run", "--max-memory", "8192"]);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, Una\n");
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    // The executable takes the package's name.
    let native = Command::new(dir.join("target/una/budget_none"))
        .args(["--max-memory", "8192"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "Hello, Una\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wide_range_in_a_task_stops_it_with_u6003_in_both_backends() {
    // #773, in a task: the range is made by a branch of a `together`, so the abort stops that
    // task, and its block reports it. Both backends end the run the same way, after the output
    // made before the block.
    let program = "module wide_task\n\nwide() -> Int\n=\n  let n: I64 = 5000000\n  let xs = together for i in 0..<n\n    i\n  xs.len\n\ntwo() -> Int = 2\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"before\")\n  together\n    let a = wide()\n    let b = two()\n  @console.print(\"{a} {b}\")\n";
    let dir = package("wide_task", "", &[("src/wide_task.una", program)]);
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"wide_task\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"wide_task\"\ngrants = [\"console\"]\n",
    )
    .unwrap();
    let expected = "[U6003] Error: this call exceeds the memory limit of 8 MiB\n";
    let run = una(&dir, &["run", "--max-memory", "8"]);
    let run_stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "{run_stderr}");
    assert!(run_stderr.starts_with(expected), "{run_stderr}");
    assert!(
        run_stderr.contains("src/wide_task.una:6:12"),
        "{run_stderr}"
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "before\n");
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/wide_task"))
        .args(["--max-memory", "8"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&native.stderr);
    assert_eq!(native.status.code(), Some(1), "{stderr}");
    assert!(stderr.starts_with(expected), "{stderr}");
    assert!(stderr.contains("src/wide_task.una:6:12"), "{stderr}");
    assert_eq!(native.stdout, run.stdout);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program of edition 0.2 named `name` whose `main` builds two equal trees of 30 levels of 64
/// forks each, about 2,000 forks a tree, and compares them with `==`: at line 25, column 14, in
/// `main`, or with `in_task` at line 26, column 16, in a branch of a `together`. Each fork's
/// children are at `step * j` and `step * j + 1` of the level below, with steps 2 and 3: the trees
/// share their parts differently, and the comparison meets 76,480 distinct pairs of forks (#815),
/// counted at 128 bytes each, over a budget of 4 MiB (32,768 pairs) and within one of 16 MiB.
/// The sharing is fixed by the source: each fork's children are the values the list of the
/// level below holds, read with `at`, the same objects in both engines. Native code copies the
/// trees into the `together` branch with their sharing (ADR 0113, I9): at 4 MiB the block and its
/// copies fit, so the comparison is where both engines stop (at 2 MiB native code stops at the
/// block itself, as ADR 0113 allows).
fn meeting_trees(name: &str, in_task: bool) -> PathBuf {
    let compare = if in_task {
        "  together\n    let same = x == y\n    let two = 2\n  @console.print(\"same {same} {two}\")\n"
    } else {
        "  let same = x == y\n  @console.print(\"same {same}\")\n"
    };
    let program = format!(
        "module {name}\n\npub kind Tree = Leaf | Fork(l: Tree, r: Tree)\n\n\
         pick(level: List[Tree], index: Int) -> Tree = level.at(index: index mod 64).or_else(default: Leaf)\n\n\
         grow(step: Int) -> Tree\n=\n  let edit level: List[Tree] = []\n  for j in 0..<64\n\
         \x20   level = level.append(item: Leaf)\n  for depth in 0..<30\n\
         \x20   let edit next: List[Tree] = []\n    for j in 0..<64\n\
         \x20     next = next.append(item: Fork(l: pick(level, index: step * j), r: pick(level, index: step * j + 1)))\n\
         \x20   level = next\n  pick(level, index: 0)\n\n\
         pub main() -> Unit\n  uses @console: Console[print]\n=\n\
         \x20 let x = grow(2)\n  let y = grow(3)\n  @console.print(\"built\")\n{compare}"
    );
    let file = format!("src/{name}.una");
    let dir = package(name, "", &[(file.as_str(), program.as_str())]);
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"{name}\"\ngrants = [\"console\"]\n"
        ),
    )
    .unwrap();
    dir
}

#[test]
fn a_comparison_over_the_budget_is_u6003_at_its_site_in_both_backends() {
    // #815: a structural comparison counts the pairs it holds at 64 bytes each, in both
    // engines, so the same comparison of the same values aborts over the same budget, at its
    // own site, and completes within a larger one. Trees that share their parts made it walk
    // every path before, 2^30 of them here.
    for (name, in_task) in [("meet", false), ("meet_task", true)] {
        let dir = meeting_trees(name, in_task);
        let built = una(&dir, &["build"]);
        assert_eq!(built.status.code(), Some(0), "{built:?}");
        let site = if in_task { ":26:16" } else { ":25:14" };
        let expected = "[U6003] Error: this call exceeds the memory limit of 4 MiB\n";
        let run = una(&dir, &["run", "--max-memory", "4"]);
        let native = native_run(&dir, name, "4");
        for (engine, output) in [("una run", &run), ("native", &native)] {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{name} {engine}: {stderr}");
            assert!(stderr.starts_with(expected), "{name} {engine}: {stderr}");
            assert!(
                stderr.contains(&format!("src/{name}.una{site}")),
                "{name} {engine}: {stderr}"
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "built\n",
                "{name} {engine}"
            );
        }
        let printed = if in_task {
            "built\nsame true 2\n"
        } else {
            "built\nsame true\n"
        };
        let run = una(&dir, &["run", "--max-memory", "16"]);
        let native = native_run(&dir, name, "16");
        for (engine, output) in [("una run", &run), ("native", &native)] {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(0), "{name} {engine}: {stderr}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                printed,
                "{name} {engine}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn flat_lists_past_the_budget_s_pairs_compare_in_both_backends() {
    // #815: a comparison holds only pairs of values with parts, and compares integers as it
    // meets them, so two flat lists of 32,769 integers, one more than the 32,768 pairs of 128
    // bytes a budget of 4 MiB holds, compare in both engines within it. A difference in a leaf
    // is covered by the unit tests of each engine.
    // The lists come from `together for`, which makes each at its size in both engines.
    let program = "module flat\n\nmake(n: Int) -> List[Int]\n=\n  let xs = together for i in 0..<n\n    i\n  xs\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let x = make(32769)\n  let y = make(32769)\n  let same = x == y\n  @console.print(\"{same}\")\n";
    let dir = package("flat", "", &[("src/flat.una", program)]);
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"flat\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"flat\"\ngrants = [\"console\"]\n",
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let run = una(&dir, &["run", "--max-memory", "4"]);
    let native = native_run(&dir, "flat", "4");
    for (engine, output) in [("una run", &run), ("native", &native)] {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(0), "{engine}: {stderr}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "true\n",
            "{engine}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program of edition 0.2 named `name` whose module `src/<name>.una` is `body` after its
/// header, with `make(n)`, a list of the integers below `n` built by a loop.
fn copies_program(name: &str, body: &str) -> PathBuf {
    let program = format!(
        "module {name}\n\nmake(n: Int) -> List[Int]\n=\n  let edit xs: List[Int] = []\n  for i in 0..<400000\n    if i < n\n      xs = xs.append(item: i)\n  xs\n\ncount(xs: List[Int]) -> Int = xs.len\n\ntwo() -> Int = 2\n\n{body}"
    );
    let file = format!("src/{name}.una");
    let dir = package(name, "", &[(file.as_str(), program.as_str())]);
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"{name}\"\ngrants = [\"console\"]\n"
        ),
    )
    .unwrap();
    dir
}

/// Runs the native executable `name` of `dir` with `--max-memory` `mib`: its output.
fn native_run(dir: &Path, name: &str, mib: &str) -> Output {
    Command::new(dir.join(format!("target/una/{name}")))
        .args(["--max-memory", mib])
        .current_dir(dir)
        .output()
        .unwrap()
}

/// Asserts that a native run completed, exit code 0, or stopped with U6003 over its budget, exit
/// code 1, and never ended on a signal or at the allocator's hard ceiling.
fn completes_or_u6003(name: &str, output: &Output) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    match output.status.code() {
        Some(1) => assert!(
            stderr.starts_with("[U6003] Error: this call exceeds the memory limit"),
            "{name}: {stderr}"
        ),
        other => assert_eq!(other, Some(0), "{name}: {stderr}"),
    }
    assert!(!stderr.contains("memory allocation"), "{name}: {stderr}");
}

/// Whether the tests run under Rosetta, Apple's x86-64 emulator, on Linux: its mappings show
/// in `/proc/self/maps`. Rosetta's own VM tracker kills a program that starts a million tasks
/// ("VmTracker slab allocator has run out of memory", VMAllocationTracker.cpp:659), which
/// completes natively on arm64 Linux and on macOS, so that one test skips itself there (#1246).
fn under_rosetta() -> bool {
    std::fs::read_to_string("/proc/self/maps").is_ok_and(|maps| maps.contains("rosetta"))
}

#[test]
fn i1_i8_a_block_of_a_million_tasks_completes_or_stops_with_u6003() {
    // ADR 0113 rules 1, 2 and 5: a `together for` over 10^6 small integers under `--max-memory
    // 64` reserves its result list exactly, each batch's state and each task's start, so it
    // either completes or stops with U6003 at the block, and never aborts the process.
    if under_rosetta() {
        println!(
            "skipped under Rosetta: the emulator's VM tracker kills a million-task program (not the program)"
        );
        return;
    }
    let main = "pub main() -> Unit\n  uses @console: Console[print]\n=\n";
    let name = "million_tasks";
    let dir = copies_program(
        name,
        &format!(
            "{main}  let xs = together for i in 0..<1000000\n    i\n  @console.print(\"{{xs.len}}\")\n"
        ),
    );
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let output = native_run(&dir, name, "64");
    completes_or_u6003(name, &output);
    if output.status.code() == Some(0) {
        assert_eq!(String::from_utf8_lossy(&output.stdout), "1000000\n");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn i1_sibling_blocks_whose_copies_pass_alone_and_not_together_stop_with_u6003() {
    // ADR 0113 rule 4: four sibling tasks each run a block that copies a list of 400000
    // integers into two tasks under `--max-memory 64`. One such block fits the budget, with
    // more than half of it to spare (it needs under 28 MiB), which the test checks; four at
    // once need more than the budget. Which step of which block meets the budget first depends
    // on scheduling, so the test checks only how each of three runs ends: it completes or
    // stops with U6003, never on a signal or at the allocator's hard ceiling.
    let main = "pub main() -> Unit\n  uses @console: Console[print]\n=\n";
    let name = "sibling_copies";
    let body = format!(
        "inner() -> Int\n=\n  let xs = make(400000)\n  let ys = together for i in 0..<2\n    xs.len + i\n  ys.len\n\n{main}  together\n    let a = inner()\n    let b = inner()\n    let c = inner()\n    let d = inner()\n  @console.print(\"{{a}} {{b}} {{c}} {{d}}\")\n"
    );
    let dir = copies_program(name, &body);
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let alone = copies_program(
        "sibling_copy_alone",
        &format!(
            "inner() -> Int\n=\n  let xs = make(400000)\n  let ys = together for i in 0..<2\n    xs.len + i\n  ys.len\n\n{main}  let a = inner()\n  @console.print(\"{{a}}\")\n"
        ),
    );
    let built = una(&alone, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let one = native_run(&alone, "sibling_copy_alone", "64");
    assert_eq!(one.status.code(), Some(0), "one block alone fits: {one:?}");
    for run in 0..3 {
        let output = native_run(&dir, name, "64");
        completes_or_u6003(&format!("{name} run {run}"), &output);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&alone);
}

#[test]
fn a_copy_between_tasks_over_the_budget_is_u6003_in_both_backends() {
    // #775: native code copies a value between the heaps of a block's tasks (`spec/16 §4`
    // rule 6) where the interpreter shares it, and the copy made a whole value in one step: under
    // `--max-memory`, it grew past the allocator's hard ceiling and the executable aborted on a
    // signal where `una run` reported U6003. Each copy now checks the budget as it grows. Where
    // each backend runs out is not exact (ADR 0061), so the two may report U6003 at different
    // places; both end the run with it, and neither on a signal.
    let main = "pub main() -> Unit\n  uses @console: Console[print]\n=\n";
    let cases = [
        // The value a task returns, copied at the join.
        (
            "join_result",
            format!(
                "{main}  together\n    let a = make(400000)\n    let b = two()\n  @console.print(\"{{a.len}} {{b}}\")\n"
            ),
        ),
        // A value a task's closure holds, copied at the fork.
        (
            "fork_argument",
            format!(
                "{main}  let xs = make(400000)\n  together\n    let a = count(xs)\n    let b = count(xs)\n  @console.print(\"{{a}} {{b}}\")\n"
            ),
        ),
        // The values of a `together for`, copied at the join. The same join copy as the first
        // case; this one checks the ending, since the released runtime aborted on it only in a
        // narrow band of sizes, which a test cannot hold steadily.
        (
            "for_results",
            format!(
                "{main}  let lists = together for i in 0..<4\n    make(150000 + i)\n  @console.print(\"{{lists.len}}\")\n"
            ),
        ),
    ];
    let expected = "[U6003] Error: this call exceeds the memory limit of 8 MiB\n";
    for (name, body) in &cases {
        let dir = copies_program(name, body);
        let run = una(&dir, &["run", "--max-memory", "8"]);
        let run_stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(1), "{name}: {run_stderr}");
        assert!(run_stderr.starts_with(expected), "{name}: {run_stderr}");
        let built = una(&dir, &["build"]);
        assert_eq!(built.status.code(), Some(0), "{name}: {built:?}");
        let native = Command::new(dir.join(format!("target/una/{name}")))
            .args(["--max-memory", "8"])
            .current_dir(&dir)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&native.stderr);
        assert_eq!(native.status.code(), Some(1), "{name}: {stderr}");
        assert!(stderr.starts_with(expected), "{name}: {stderr}");
        assert!(!stderr.contains("memory allocation"), "{name}: {stderr}");
        assert_eq!(native.stdout, run.stdout, "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn an_executable_over_its_memory_budget_is_u6003() {
    // ADR 0061 rule 2: the executable takes `--max-memory` as `una run` does, and the call that
    // finds the run over its budget is U6003.
    let program = "module hog\n\ngrow(t: Text, n: Int) -> Int = if n <= 0 then t.len else max(grow(\"{t}0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\", n: n - 1), b: t.len)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let size = grow(\"\", n: 9000)\n  @console.print(\"held {size}\")\n";
    let dir = package(
        "hog",
        "\n[run]\nentry = \"hog\"\ngrants = [\"console\"]\n",
        &[("src/hog.una", program)],
    );
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let exe = dir.join("target/una/hog");
    let output = Command::new(&exe)
        .args(["--max-memory", "64"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("[U6003] Error: this call exceeds the memory limit of 64 MiB\n"),
        "{stderr}"
    );
    let output = Command::new(&exe)
        .args(["--max-memory", "many"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    // HOST-4 (#185): a budget of 0 MiB is a usage error too, as for `una run`.
    let output = Command::new(&exe)
        .args(["--max-memory", "0"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: `--max-memory 0` holds nothing; give at least 1 MiB\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_executable_stops_at_its_stack_whatever_its_max_depth() {
    // ADR 0061 rule 1, ADR 0032: the stack is capped at 1 GiB, and a recursion that runs short
    // of it before its depth limit is U6003, never a stack overflow.
    let program = "module never\n\ncountdown(n: Int) -> Int = if n == 0 then 0 else countdown(n - 1)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let steps = countdown(-1)\n  @console.print(\"done {steps}\")\n";
    let dir = package(
        "never",
        "\n[run]\nentry = \"never\"\ngrants = [\"console\"]\n",
        &[("src/never.una", program)],
    );
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let output = Command::new(dir.join("target/una/never"))
        .args(["--max-depth", "100000000000"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with(
            "[U6003] Error: this call exceeds the maximum call depth of 100000000000\n"
        ),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_executable_loop_over_its_memory_budget_is_u6003() {
    // ADR 0061 rule 2: a `for` iteration is not a call, and the executable checks the budget
    // at each one too, at the loop.
    let program = "module hog\n\npub kind Node\n  value: Int\n  next: Option[Node]\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit chain: Option[Node] = None\n  for i in 0..<100000000\n    chain = Some(Node(value: i, next: chain))\n  let built = chain.is_some()\n  @console.print(\"built {built}\")\n";
    let dir = package(
        "loop-hog",
        "\n[run]\nentry = \"hog\"\ngrants = [\"console\"]\n",
        &[("src/hog.una", program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let output = Command::new(dir.join("target/una/loop-hog"))
        .args(["--max-memory", "16"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with(
            "[U6003] Error: this call exceeds the memory limit of 16 MiB\n  at src/hog.una:11:3"
        ),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_json_parse_over_its_memory_budget_is_u6003_in_both_engines() {
    // #314 (review finding H-2): `parse_json` checks the budget as it reads, so a large
    // input ends with U6003 at the call, in the interpreter and natively, rather than with an
    // allocation the process cannot make. The text itself, about 13 MB, fits the budget; the
    // values it parses into do not.
    let program = "module hog\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit t = \"[0\"\n  for i in 0..<1000000\n    t = \"{t},\\\"abcdefghij\\\"\"\n  let doc = parse_json(\"{t}]\")\n    if error use Null\n  @console.print(\"{doc.to_json().len}\")\n";
    let dir = package(
        "json-hog",
        "\n[run]\nentry = \"hog\"\ngrants = [\"console\"]\n",
        &[("src/hog.una", program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/json-hog"))
        .args(["--max-memory", "64"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(1), "{native:?}");
    let stderr = String::from_utf8_lossy(&native.stderr);
    assert!(
        stderr.starts_with(
            "[U6003] Error: this call exceeds the memory limit of 64 MiB\n  at src/hog.una:9:13"
        ),
        "{stderr}"
    );
    let interpreted = una(&dir, &["run", "--max-memory", "64"]);
    assert_eq!(interpreted.status.code(), Some(1), "{interpreted:?}");
    let stderr = String::from_utf8_lossy(&interpreted.stderr);
    assert!(
        stderr.starts_with("[U6003] Error: this call exceeds the memory limit of 64 MiB\n"),
        "{stderr}"
    );
    assert!(stderr.contains("hog.una:9:13"), "{stderr}");
    // Within the default budget the same program parses and writes the whole text.
    let output = Command::new(dir.join("target/una/json-hog"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "13000003\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn i3_i4_a_json_parse_over_its_memory_budget_in_a_task_stops_at_one_site_in_both_engines() {
    // #314: in a task of `together`, the parse that finds the run over its budget stops
    // the task, natively through `una_rt_json_stopped`, and the block ends the run with U6003
    // at that call, as the interpreter does; the line after the block never prints.
    let program = "module hog\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit t = \"[0\"\n  for i in 0..<1000000\n    t = \"{t},\\\"abcdefghij\\\"\"\n  together\n    let doc = parse_json(\"{t}]\")\n      if error use Null\n    let other = t.len\n  @console.print(\"{doc.to_json().len} {other}\")\n";
    let dir = package(
        "json-task-hog",
        "\n[run]\nentry = \"hog\"\ngrants = [\"console\"]\n",
        &[("src/hog.una", program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/json-task-hog"))
        .args(["--max-memory", "64"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(1), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "");
    let stderr = String::from_utf8_lossy(&native.stderr);
    // Native code copies `t` into the tasks at the fork, where the interpreter shares it. Since
    // ADR 0113 the block's copies are made while none of its tasks runs code, and its fork peaks
    // under the budget, so the first task's parse is where the run stops, on every run, as in
    // `una run` (#777).
    assert!(
        stderr.starts_with(
            "[U6003] Error: this call exceeds the memory limit of 64 MiB\n  at src/hog.una:10:15"
        ),
        "{stderr}"
    );
    // The stress variant: the site is the same on 50 runs in a row (ADR 0113, I3 and I4).
    for run in 0..50 {
        let again = Command::new(dir.join("target/una/json-task-hog"))
            .args(["--max-memory", "64"])
            .current_dir(&dir)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&again.stderr);
        assert!(
            stderr.starts_with(
                "[U6003] Error: this call exceeds the memory limit of 64 MiB\n  at src/hog.una:10:15"
            ),
            "run {run}: {stderr}"
        );
    }
    let interpreted = una(&dir, &["run", "--max-memory", "64"]);
    assert_eq!(interpreted.status.code(), Some(1), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "");
    let stderr = String::from_utf8_lossy(&interpreted.stderr);
    assert!(
        stderr.starts_with("[U6003] Error: this call exceeds the memory limit of 64 MiB\n"),
        "{stderr}"
    );
    assert!(stderr.contains("hog.una:10:15"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_text_appended_in_a_loop_grows_in_linear_time() {
    // ADR 0070 X10, `spec/22 §6` rule 2 (#261): natively too, `t = "{t}x"` appends in place
    // when nothing else holds `t`.
    let sizes = [100_000, 1_000_000];
    let dirs: Vec<PathBuf> = sizes
        .iter()
        .map(|steps| {
            let program = format!(
                "module grow\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit t = \"\"\n  for i in 0..<{steps}\n    t = \"{{t}}x\"\n  @console.print(\"{{t.len}}\")\n"
            );
            let dir = package(
                &format!("grow{steps}"),
                "\n[run]\nentry = \"grow\"\ngrants = [\"console\"]\n",
                &[("src/grow.una", &program)],
            );
            let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
            std::fs::write(
                dir.join("una.toml"),
                manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
            )
            .unwrap();
            let built = una(&dir, &["build"]);
            assert_eq!(built.status.code(), Some(0), "{built:?}");
            dir
        })
        .collect();
    // Times a run of the executable built for `steps` appends.
    let measure = |steps: usize| {
        let dir = &dirs[sizes.iter().position(|&s| s == steps).unwrap()];
        let (output, seconds) = timing::timed(|| {
            Command::new(dir.join("target/una").join(format!("grow{steps}")))
                .current_dir(dir)
                .output()
                .unwrap()
        });
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{steps}\n")
        );
        // The deterministic check, the primary one: the debug runtime counts every copy, and
        // there is one at any size, of the literal `""` that the loop starts from, which is
        // never updated in place.
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "note: una-rt (debug build): lists, maps and texts copied: 1\n"
        );
        seconds
    };
    // The timing is the secondary check, with a wide margin: 3.5x measured; the bound of 30x
    // is the linear bound of #341, which only quadratic work (about 100x) exceeds. It no longer
    // separates the 17x measured when copying: the copy count above does. A miss is measured
    // twice more before it fails.
    timing::assert_growth("appends", &sizes, 0.0, 0.05, 30.0, measure);
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_list_built_by_append_in_a_loop_is_never_copied_at_any_size() {
    // #878: `xs = xs.append(item: i)` in a `for` loop updates the list in place, at every size,
    // so building it takes linear time and space. The debug runtime counts every copy. Each
    // program also appends to a list that another binding still holds, which must copy it
    // once: the count is exactly 1 at every size, which shows the counter counts and that the
    // loop adds no copy.
    for n in [1 << 12, 1 << 14, 1 << 16] {
        let program = format!(
            "module built\n\nmake(n: Int) -> List[Int]\n=\n  let edit xs: List[Int] = []\n  for i in 0..<n\n    xs = xs.append(item: i)\n  xs\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let x = make({n})\n  let y = x.append(item: 0)\n  @console.print(\"{{x.len}} {{y.len}}\")\n"
        );
        let dir = package(
            &format!("built{n}"),
            "\n[run]\nentry = \"built\"\ngrants = [\"console\"]\n",
            &[("src/built.una", &program)],
        );
        edition_0_2(&dir);
        let expected = format!("{n} {}\n", n + 1);
        let interpreted = una(&dir, &["run", "."]);
        assert_eq!(interpreted.status.code(), Some(0), "{n}: {interpreted:?}");
        assert_eq!(
            String::from_utf8_lossy(&interpreted.stdout),
            expected,
            "{n}"
        );
        let built = una(&dir, &["build"]);
        assert_eq!(built.status.code(), Some(0), "{n}: {built:?}");
        let output = Command::new(dir.join(format!("target/una/built{n}")))
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{n}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected, "{n}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "note: una-rt (debug build): lists, maps and texts copied: 1\n",
            "{n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn collections_updated_in_a_loop_are_never_copied_at_any_size() {
    // ADR 0119 Costs and rule 19 (#981): a `Queue`, a `Set`, a `PriorityQueue`, a `BitSet` and
    // a `HashSet` updated in a loop while uniquely held are updated in place, at every size, so
    // building them takes linear time; one push while a second binding holds the queue copies
    // it once. The debug runtime counts every copy: exactly 1 at every size. The output is the
    // interpreter's.
    for n in [1 << 10, 1 << 12, 1 << 14] {
        let program = format!(
            "module grown\n\nimport std.collections (BitSet, HashSet, PriorityQueue, Queue, Set, empty_bit_set, empty_hash_set,\n  empty_queue, empty_set, lowest_first\n)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit q: Queue[Int] = empty_queue()\n  let edit s: Set[Int] = empty_set()\n  let edit p: PriorityQueue[Int, Int] = lowest_first()\n  let edit b: BitSet = empty_bit_set(size: {n})\n  let edit h: HashSet[Int] = empty_hash_set()\n  for i in 0..<{n}\n    edit q.push(item: i)\n    edit s.add(item: i)\n    edit p.push(item: i, priority: i)\n    if 0 <= i and i < b.size\n      edit b.add(item: i)\n    edit h.add(item: i)\n  let kept = q\n  edit q.push(item: 0)\n  @console.print(\"{{kept.len}} {{q.len}} {{s.len}} {{p.len}} {{b.len}} {{h.len}}\")\n"
        );
        let dir = ita_package(
            &format!("grown{n}"),
            "\nrun\n  entry grown\n  grants\n    console\n",
            &[("src/grown.una", &program)],
        );
        let expected = format!("{n} {} {n} {n} {n} {n}\n", n + 1);
        let interpreted = una(&dir, &["run"]);
        assert_eq!(interpreted.status.code(), Some(0), "{n}: {interpreted:?}");
        assert_eq!(
            String::from_utf8_lossy(&interpreted.stdout),
            expected,
            "{n}"
        );
        let built = una(&dir, &["build"]);
        assert_eq!(built.status.code(), Some(0), "{n}: {built:?}");
        let output = Command::new(dir.join(format!("target/una/grown{n}")))
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{n}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected, "{n}");
        assert_eq!(copied(&output), 1, "{n}: {output:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_loop_runs_in_constant_memory() {
    // #201: a loop whose state takes constant space runs in constant space, also in the debug
    // runtime the tests link, whose quarantine of freed slots is bounded: 10,000,000 iterations
    // and 1,000,000 appends fit in 64 MiB, and their output is the interpreter's.
    let program = "module steady\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit n = 0\n  for i in 0..<10000000\n    n = n + 1\n  let edit xs: List[Int] = []\n  for i in 0..<1000000\n    xs = xs.append(item: i)\n  @console.print(\"{n} {xs.len}\")\n";
    let dir = package(
        "steady",
        "\n[run]\nentry = \"steady\"\ngrants = [\"console\"]\n",
        &[("src/steady.una", program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let output = Command::new(dir.join("target/una/steady"))
        .args(["--max-memory", "64"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "10000000 1000000\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn updates_of_values_nothing_else_holds_are_in_place() {
    // #30: 200,000 appends in a loop, 200,000 `set`s and 200,000 field edits copy only what
    // another holder still sees: the caller's list, once, and the caller's point, made anew
    // once. The debug runtime reports its copies and, apart, the records made anew (#292; no
    // flag; release builds say nothing).
    let program = "module linear\n\npub kind Point\n  x: Int\n  y: Int\n\npub build(n: Int) -> List[Int]\n=\n  let edit acc: List[Int] = []\n  for i in 0..<n\n    acc = acc.append(item: i)\n  acc\n\npub overwrite(xs: List[Int]) -> List[Int]\n=\n  let edit ys = xs\n  for i in 0..<xs.len\n    if i < ys.len\n      edit ys.set(index: i, item: i * 2)\n    else\n      unit\n  ys\n\npub shift(p: Point, times: Int) -> Point\n=\n  let edit q = p\n  for i in 0..<times\n    q.x = q.x + 1\n  q\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let xs = build(200000)\n  let ys = overwrite(xs)\n  let p = shift(Point(x: 0, y: 7), times: 200000)\n  let third = ys.at(index: 3).or_else(default: 0)\n  @console.print(\"{xs.len} {ys.len} {third} {p.x} {p.y}\")\n";
    let dir = package(
        "linear",
        "\n[run]\nentry = \"linear\"\ngrants = [\"console\"]\n",
        &[("src/linear.una", program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let output = Command::new(dir.join("target/una/linear"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "200000 200000 6 200000 7\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "note: una-rt (debug build): lists, maps and texts copied: 1\nnote: una-rt (debug build): records and variants made anew, not reused: 1\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program that sets, looks up and removes `N` keys, then copies a map another binding holds.
const MAP_UPDATES: &str = "module maps\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit m: Map[Int, Int] = empty_map()\n  for i in 0..<N\n    edit m.set(key: (i * 7919) mod N, value: i)\n  let edit found = 0\n  for i in 0..<N\n    if m.has(key: i)\n      found = found + 1\n  let kept = m\n  edit m.remove(key: 0)\n  for i in 1..<N\n    edit m.remove(key: i)\n  @console.print(\"{found} {m.len} {kept.len}\")\n";

#[test]
fn a_map_passed_by_edit_moves_into_the_callee() {
    // ADR 0072 with `spec/21 §5` rule 2: a helper that sets a key of its `edit` map owns it,
    // so a loop of 200,000 calls updates one map in place, with no copy.
    let program = "module helper\n\nput(edit m: Map[Int, Int], k: Int) -> Unit\n=\n  edit m.set(key: k, value: k)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit m: Map[Int, Int] = empty_map()\n  for i in 0..<200000\n    put(edit m, k: i)\n  @console.print(\"{m.len}\")\n";
    let dir = package(
        "helper",
        "\n[run]\nentry = \"helper\"\ngrants = [\"console\"]\n",
        &[("src/helper.una", program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let output = Command::new(dir.join("target/una/helper"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "200000\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn map_updates_are_in_place_and_scale_as_n_log_n_in_cpu_time() {
    // `spec/21 §5` rule 2 and the performance rule: `set`, `has` and `remove` on a map nothing
    // else holds are O(log n) and in place; the one copy is the map that `kept` still sees.
    // The copy count is the primary check; the timing is the secondary one: ten times the keys
    // take under 40 times as long, n log n giving about 12 and a copy per update about 100. A
    // miss is measured twice more before it fails (#341).
    let sizes = [100_000, 1_000_000];
    let dirs: Vec<PathBuf> = sizes
        .iter()
        .map(|n| {
            let dir = package(
                &format!("maps_{n}"),
                "\n[run]\nentry = \"maps\"\ngrants = [\"console\"]\n",
                &[("src/maps.una", &MAP_UPDATES.replace('N', &n.to_string()))],
            );
            let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
            std::fs::write(
                dir.join("una.toml"),
                manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
            )
            .unwrap();
            let built = una(&dir, &["build"]);
            assert_eq!(built.status.code(), Some(0), "{built:?}");
            dir
        })
        .collect();
    // Times a run of the executable built for `n` keys.
    let measure = |n: usize| {
        let dir = &dirs[sizes.iter().position(|&s| s == n).unwrap()];
        let (output, seconds) = cpu::cpu_output(
            Command::new(dir.join(format!("target/una/maps_{n}"))).current_dir(dir),
        );
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{n} 0 {n}\n")
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "note: una-rt (debug build): lists, maps and texts copied: 1\n"
        );
        seconds
    };
    timing::assert_growth("keys", &sizes, 0.0, timing::FLOOR, 40.0, measure);
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_closed_output_pipe_is_a_failed_write_not_a_signal() {
    // #184: the executable ignores `SIGPIPE`, as `una run` does: when the reader of its output
    // goes away, the writes fail, the run goes on, the trace is written, and the exit code is 2.
    runtime();
    let program = "module chatty\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit n = 0\n  for i in 0..<20000\n    @console.print(\"line {i}\")\n    n = n + 1\n  @console.print(\"done {n}\")\n";
    let dir = package(
        "chatty",
        "\n[run]\nentry = \"chatty\"\ngrants = [\"console\"]\n",
        &[("src/chatty.una", program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let first_line = |command: &mut Command| {
        let mut child = command
            .current_dir(&dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
        drop(reader);
        let output = child.wait_with_output().unwrap();
        (line, output)
    };
    let trace = dir.join("trace.json");
    let (line, native) = first_line(
        Command::new(dir.join("target/una/chatty"))
            .arg("--trace")
            .arg(&trace),
    );
    assert_eq!(line, "line 0\n");
    assert_eq!(native.status.code(), Some(2), "{native:?}");
    assert!(
        String::from_utf8_lossy(&native.stderr).starts_with("error: cannot write the output: "),
        "{native:?}"
    );
    let events: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 20001);
    let (_, interpreted) = first_line(
        Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .args(["run", "."]),
    );
    assert_eq!(interpreted.status.code(), native.status.code());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_write_to_stderr_exits_as_una_run_does() {
    // #198: with stderr on a pipe whose reader is gone, writing a diagnostic fails, and
    // `una run` exits 2 as for any failed write of its output (`spec/13 §5`). The executable
    // exits 2 too, on U3012 before `main`, on U6001 after `main` fails, and on U6003.
    runtime();
    let program = "module oops\n\npub kind Oops = | Bad\n\ndeep(n: Int) -> Int = if n == 0 then 0 else 1 + deep(n - 1)\n\npub main() -> Unit\n  uses @console: Console[print]\n  fails Oops\n=\n  @console.print(\"start\")\n  let d = deep(100)\n  @console.print(\"{d}\")\n  fail Bad\n";
    let dir = package(
        "oops",
        "\n[run]\nentry = \"oops\"\n",
        &[("src/oops.una", program)],
    );
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let closed = |command: &mut Command| {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        command
            .current_dir(&dir)
            .stdout(std::process::Stdio::null())
            .stderr(writer)
            .status()
            .unwrap()
            .code()
    };
    let exe = dir.join("target/una/oops");
    for args in [
        &[][..],
        &["--grant", "console"][..],
        &["--grant", "console", "--max-depth", "20"][..],
    ] {
        let interpreted = closed(
            Command::new(env!("CARGO_BIN_EXE_una"))
                .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                .env_remove("UNA_PROOF_KEY")
                .arg("run")
                .arg(".")
                .args(args),
        );
        let native = closed(Command::new(&exe).args(args));
        assert_eq!(interpreted, Some(2), "una run {args:?}");
        assert_eq!(native, interpreted, "the executable with {args:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_stdout_is_reported_on_stderr_by_both_engines() {
    // #199: with stdout on a pipe whose reader is gone, `una run` exited 2 and said nothing.
    // Like the executable, it now reports the U6001 of a failing `main` and the failed write
    // on stderr, and exits 2.
    runtime();
    let program = "module closed\n\npub kind Oops = | Bad\n\npub main() -> Unit\n  uses @console: Console[print]\n  fails Oops\n=\n  @console.print(\"start\")\n  fail Bad\n";
    let dir = package(
        "closed",
        "\n[run]\nentry = \"closed\"\ngrants = [\"console\"]\n",
        &[("src/closed.una", program)],
    );
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let closed = |command: &mut Command| {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        command
            .current_dir(&dir)
            .env("NO_COLOR", "1")
            .stdout(writer)
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap()
    };
    let interpreted = closed(
        Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .args(["run", "."]),
    );
    let native = closed(&mut Command::new(dir.join("target/una/closed")));
    for (engine, output) in [("una run", &interpreted), ("the executable", &native)] {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{engine}: {stderr}");
        assert!(stderr.contains("U6001"), "{engine}: {stderr}");
        assert!(
            stderr.contains("error: cannot write the output: "),
            "{engine}: {stderr}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_executable_checks_main_against_its_grants() {
    runtime();
    // No `[run] grants`: `una build` accepts it, and the executable needs `--grant console`.
    let dir = package(
        "greet",
        "\n[run]\nentry = \"hello\"\n",
        &[("src/hello.una", HELLO)],
    );
    let output = una(&dir, &["build"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = dir.join("target/una/greet");
    let denied = Command::new(&exe).output().unwrap();
    assert_eq!(denied.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&denied.stdout), "");
    let stderr = String::from_utf8_lossy(&denied.stderr);
    assert!(
        stderr.contains("U3012")
            && stderr.contains(
                "`main` uses `print` of `Console`, which neither `[run] grants` nor a `--grant` flag grants"
            )
            && stderr.contains("src/hello.una:4:3"),
        "{stderr}"
    );
    let granted = Command::new(&exe)
        .args(["--grant", "console"])
        .output()
        .unwrap();
    assert_eq!(granted.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&granted.stdout), "Hello, Una\n");
    // Usage errors, exit code 2, as for `una run`.
    let usage = |args: &[&str]| Command::new(&exe).args(args).output().unwrap();
    let bad = usage(&["--grant", "network"]);
    assert_eq!(bad.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&bad.stderr),
        "error: `network` is not a grant; grants are console, clock, env, keys:<file>, files:<path>:<access> and http:<origin>[:<methods>]\n"
    );
    assert_eq!(usage(&["--json"]).status.code(), Some(2));
    assert_eq!(usage(&["--max-depth", "many"]).status.code(), Some(2));
    // HOST-3 (#185): U3012 comes before the key files are read, as in `una run`: an ungranted
    // `uses` is U3012 even when a `keys:` grant names a file that does not exist, and the
    // missing file is a usage error only once every `uses` is granted.
    let ungranted = usage(&["--grant", "keys:missing.toml"]);
    assert_eq!(ungranted.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&ungranted.stderr);
    assert!(
        stderr.contains("U3012") && !stderr.contains("missing.toml"),
        "{stderr}"
    );
    let missing = usage(&["--grant", "console", "--grant", "keys:missing.toml"]);
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&missing.stdout), "");
    assert!(
        String::from_utf8_lossy(&missing.stderr)
            .starts_with("error: cannot read the key file `missing.toml`"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A package of edition 0.2, which has `together` (ADR 0056), whose entry is `module`.
fn tasks_package(name: &str, module: &str, program: &str) -> PathBuf {
    let dir = package(
        name,
        &format!("\n[run]\nentry = \"{module}\"\ngrants = [\"console\"]\n"),
        &[(&format!("src/{module}.una"), program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}

#[test]
fn one_worker_runs_blocks_nested_three_levels_deep_without_deadlock() {
    // ADR 0117 rule 3 and its Conformance section: a join parks its task, so a `together` whose
    // parent and children share the one worker, nested three levels deep, ends with the
    // interpreter's results, with one worker and with several.
    let program = "module nest\n\nleaf(label: Text, n: Int) -> Int\n  uses @console: Console[print]\n=\n  @console.print(label)\n  n\n\nmiddle(label: Text, n: Int) -> Int\n  uses @console: Console[print]\n=\n  together\n    let x = leaf(\"{label}.1\", n: n)\n    let y = leaf(\"{label}.2\", n: n + 1)\n  x + y\n\ntop(label: Text, n: Int) -> Int\n  uses @console: Console[print]\n=\n  together\n    let x = middle(\"{label}.1\", n: n)\n    let y = middle(\"{label}.2\", n: n + 10)\n  x + y\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  together\n    let a = top(\"a\", n: 1)\n    let b = top(\"b\", n: 100)\n  @console.print(\"{a} {b}\")\n";
    let dir = tasks_package("nest", "nest", program);
    let exe = dir.join("target/una/nest");
    for workers in ["1", "2", "64"] {
        let native = Command::new(&exe)
            .args(["--workers", workers])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        let stdout = String::from_utf8_lossy(&native.stdout);
        let mut lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.pop(), Some("26 422"), "--workers {workers}: {stdout}");
        lines.sort_unstable();
        assert_eq!(
            lines,
            [
                "a.1.1", "a.1.2", "a.2.1", "a.2.2", "b.1.1", "b.1.2", "b.2.1", "b.2.2"
            ],
            "--workers {workers}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Whether the executable at `exe` embeds the description key that starts its threads before
/// `main` (#1145): `una build` writes the description as compact JSON, so the key reads
/// `"threads":true` in the binary, and the runtime's own code never holds that text.
fn embeds_threads(exe: &Path) -> bool {
    let bytes = std::fs::read(exe).unwrap();
    bytes
        .windows(br#""threads":true"#.len())
        .any(|window| window == br#""threads":true"#)
}

#[test]
fn the_description_starts_threads_exactly_for_tasks_and_for_ever() {
    // #1145, ADR 0116 amendment P8, `spec/27 §1` rule 4: codegen marks a program that holds a
    // `together`, a `together for`, a `first of`, a `tasks` statement, a `for ever` or a
    // `within`, and `una build` embeds `"threads": true`, which makes the runtime start its
    // threads and fill its stack pool before `main`; a program with none of them has no such
    // key and starts no thread.
    let cases = [
        (
            "flagtogether",
            "module flagtogether\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  together\n    let a = 1\n    let b = 2\n  @console.print(\"{a} {b}\")\n",
            true,
        ),
        (
            "flagfor",
            "module flagfor\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let squares = together for i in 1..3\n    i * i\n  @console.print(\"{squares.len}\")\n",
            true,
        ),
        (
            "flagnone",
            "module flagnone\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"plain\")\n",
            false,
        ),
    ];
    for (name, program, threads) in cases {
        let dir = tasks_package(name, name, program);
        let exe = dir.join(format!("target/una/{name}"));
        assert_eq!(embeds_threads(&exe), threads, "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // `for ever`, from its conformance case, in edition 0.4.
    let scratch = std::env::temp_dir().join(format!("una-flag-ever-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = copy(
        &root().join("tests/conformance/runtime/for_ever_on_stop"),
        &scratch,
    );
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    assert!(embeds_threads(&dir.join("target/una/for_ever_on_stop")));
    // `within`, from its conformance case, in edition 0.4: a deadline counts as a task
    // construct for P8, so its program starts its threads before `main` too.
    let within = copy(
        &root().join("tests/conformance/runtime/within_runs"),
        &scratch,
    );
    let built = una(&within, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    assert!(embeds_threads(&within.join("target/una/within_runs")));
    // `first of`, from its conformance case: its branches are tasks.
    let race = copy(
        &root().join("tests/conformance/runtime/first_of_runs"),
        &scratch,
    );
    let built = una(&race, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    assert!(embeds_threads(&race.join("target/una/first_of_runs")));
    // A `tasks` statement and its `start`s, from their conformance case.
    let started = copy(
        &root().join("tests/conformance/runtime/tasks_runs"),
        &scratch,
    );
    let built = una(&started, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    assert!(embeds_threads(&started.join("target/una/tasks_runs")));
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn the_flags_of_tasks_out_of_range_are_usage_errors_in_both_engines() {
    // ADR 0117, Diagnostics: a `--workers`, `--task-depth`, `--io-threads` or `--file-threads`
    // that is not a number or is out of its range is a usage error, exit code 2, before `main`.
    let program = "module flags\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"ran\")\n";
    let dir = tasks_package("flags", "flags", program);
    let exe = dir.join("target/una/flags");
    for args in [
        &["--workers", "0"][..],
        &["--workers", "1025"],
        &["--workers", "many"],
        &["--max-depth", "50", "--task-depth", "51"],
        &["--io-threads", "4097"],
        &["--file-threads", "257"],
        // ADR 0123 rule 20: the serve threads, from 1 to 256.
        &["--serve-threads", "0"],
        &["--serve-threads", "257"],
        &["--serve-threads", "x"],
    ] {
        let native = Command::new(&exe)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(2), "{args:?}: {native:?}");
        assert!(native.stdout.is_empty(), "{args:?}");
        let mut run = vec!["run", "."];
        run.extend_from_slice(args);
        let interpreted = una(&dir, &run);
        assert_eq!(
            interpreted.status.code(),
            Some(2),
            "{args:?}: {interpreted:?}"
        );
    }
    let native = Command::new(&exe)
        .args([
            "--workers",
            "1",
            "--task-depth",
            "7",
            "--io-threads",
            "1",
            "--file-threads",
            "1",
            "--serve-threads",
            "256",
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "ran\n");
    let interpreted = una(&dir, &["run", ".", "--serve-threads", "1"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_task_counts_its_depth_from_its_block() {
    // `spec/19 §8` rule 3: `--max-depth` bounds each task's depth, counted from the depth of the
    // block that started it, as the interpreter counts it; past it, the run is U6003.
    let program = "module deep\n\ncountdown(n: Int) -> Int = if n == 0 then 0 else countdown(n - 1)\n\nnested(n: Int) -> Int\n=\n  together\n    let a = countdown(n)\n    let b = countdown(1)\n  a + b\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  together\n    let a = countdown(10)\n    let b = nested(85)\n  @console.print(\"done {a} {b}\")\n";
    let dir = tasks_package("deep", "deep", program);
    let exe = dir.join("target/una/deep");
    for (depth, code) in [("100", 0), ("80", 1)] {
        let native = Command::new(&exe)
            .args(["--max-depth", depth])
            .current_dir(&dir)
            .output()
            .unwrap();
        let interpreted = una(&dir, &["run", ".", "--max-depth", depth]);
        assert_eq!(native.status.code(), Some(code), "{native:?}");
        assert_eq!(interpreted.status.code(), Some(code), "{interpreted:?}");
        assert_eq!(native.stdout, interpreted.stdout);
        if code == 1 {
            let stderr = String::from_utf8_lossy(&native.stderr);
            assert!(
                stderr.starts_with(&format!(
                    "[U6003] Error: this call exceeds the maximum call depth of {depth}\n"
                )),
                "{stderr}"
            );
            // The same diagnostic; `una run` also shows the source around it.
            let interpreted = String::from_utf8_lossy(&interpreted.stderr);
            assert_eq!(stderr.lines().next(), interpreted.lines().next());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tasks_of_a_run_share_its_memory_budget() {
    // `spec/19 §8` rule 3: `--max-memory` bounds the heaps of all tasks together, and the call
    // of a task that finds the run over its budget is U6003.
    let program = "module hogs\n\ngrow(t: Text, n: Int) -> Int = if n <= 0 then t.len else max(grow(\"{t}0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\", n: n - 1), b: t.len)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  together\n    let a = grow(\"\", n: 10)\n    let b = grow(\"\", n: 9000)\n  @console.print(\"held {a} {b}\")\n";
    let dir = tasks_package("hogs", "hogs", program);
    let output = Command::new(dir.join("target/una/hogs"))
        .args(["--max-memory", "64"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("[U6003] Error: this call exceeds the memory limit of 64 MiB\n"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_task_cancels_the_tasks_after_it() {
    // `spec/19 §6` rules 3 and 4: when a task fails, a task after it stops at its next loop
    // iteration, call or operation, or never starts; the block fails with the failure.
    let program = "module cancel\n\npub kind Oops = Bad | Worse\n\nboom() -> Int\n  fails Oops\n=\n  let edit total = 0\n  for i in 0..<3000000\n    total = total + 1\n  if total > 0\n    fail Bad\n  total\n\nspin(n: Int) -> Int\n  uses @console: Console[print]\n=\n  let edit total = 0\n  for i in 0..<n\n    total = total + 1\n  @console.print(\"spun {total}\")\n  total\n\nspins() -> Int\n  uses @console: Console[print]\n=\n  together\n    let a = spin(10000000000)\n    let b = spin(10000000000)\n  a + b\n\npick() -> Int\n  uses @console: Console[print]\n  fails Oops\n=\n  together\n    let a = boom()\n      if error pass\n    let b = spin(10000000000)\n    let c = spins()\n  a + b + c\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let tried = attempt pick()\n  let outcome = when tried\n    is Ok(n) -> \"ok {n}\"\n    is Err(e) -> \"failed\"\n  @console.print(outcome)\n";
    let dir = tasks_package("cancel", "cancel", program);
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una/cancel"))
        .arg("--trace")
        .arg(&trace)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "failed\n");
    // The failing task works first, so the tasks after it have usually started. They, and the
    // tasks of the block that the third one runs, stopped before their prints, each with a `cancelled` event if it had started, in task
    // order.
    let events: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    let events = events.as_array().unwrap();
    let (last, stopped) = events.split_last().unwrap();
    assert_eq!(last["op"], "print");
    let order = ["2", "3.1", "3.2", "3"];
    let mut paths = order.iter();
    for (index, event) in stopped.iter().enumerate() {
        let task = event["task"].as_str().unwrap();
        assert!(paths.any(|path| *path == task), "{events:?}");
        assert_eq!(
            *event,
            serde_json::json!({"seq": index + 1, "task": task, "outcome": {"kind": "cancelled"}})
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program whose first task fails only once the second has started: the second writes
/// `box/started` and then performs operations of a Una handler, `step`, whose body runs `BODY`;
/// the first polls for the file and then fails. The cancellation of the second task so comes
/// while it performs its operations (#200).
const IN_OPERATION: &str = "module opcancel

pub kind Oops = | Bad

pub capability Work
  op step(n: Int) -> Int

pub handler busy() for Work
  op step(n) = BODY

churn(n: Int) -> Int
=
  let edit total = 0
  for i in 0..<n
    total = total + 1
  total

deep(n: Int) -> Int = if n == 0 then 0 else deep(n - 1) + 1

started() -> Bool
  uses @files: Files[read]
=
  let r = attempt @files.read(\"started\")
  when r
    is Ok(t) -> true
    is Err(e) -> false

watcher() -> Int
  uses @files: Files[read]
  fails Oops
=
  let edit seen = false
  for i in 0..<20000
    if not seen
      seen = started()
  fail Bad

worker() -> Int
  uses @files: Files[write]
=
  let w = attempt @files.write(\"started\", content: \"yes\")
  let edit total = 0
  for i in 0..<STEPS
    let v = stepped() with @work = busy()
    total = total + v
  total

stepped() -> Int
  uses @work: Work[step]
=
  @work.step(20000)

pick() -> Int
  uses @files: Files[read, write]
  fails Oops
=
  together
    let a = watcher()
      if error pass
    let b = worker()
  a + b

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let r = attempt pick()
  let shown = when r
    is Ok(v) -> \"ok\"
    is Err(e) -> \"failed\"
  @console.print(shown)
";

/// Builds `IN_OPERATION` with `body` as the handler's operation and `steps` operations, runs
/// it natively with a trace and `args`, then in the interpreter, and returns the native output,
/// the native trace, and the interpreter's output.
fn in_operation(
    name: &str,
    body: &str,
    steps: &str,
    args: &[&str],
) -> (Output, Vec<Value>, Output) {
    let program = IN_OPERATION.replace("BODY", body).replace("STEPS", steps);
    let dir = package(
        name,
        "\n[run]\nentry = \"opcancel\"\ngrants = [\"console\", \"files:box:read,write\"]\n",
        &[("src/opcancel.una", &program)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("box")).unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una").join(name))
        .args(args)
        .arg("--trace")
        .arg(&trace)
        .current_dir(&dir)
        .output()
        .unwrap();
    let events: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    let events = events.as_array().unwrap().clone();
    let _ = std::fs::remove_file(dir.join("box/started"));
    let mut run_args = vec!["run", "."];
    run_args.extend(args);
    let interpreted = una(&dir, &run_args);
    let _ = std::fs::remove_dir_all(&dir);
    (native, events, interpreted)
}

/// The events of task `task`.
fn task_events<'a>(events: &'a [Value], task: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event["task"] == task)
        .collect()
}

#[test]
fn an_operation_in_progress_at_cancellation_completes() {
    // `spec/19 §6` rule 4 and `§7` rule 3 (#200): the second task is cancelled while it performs
    // an operation of a Una handler, whose body makes calls and loops. The operation completes
    // with its own outcome; the task performs no operation after it and ends with `cancelled`.
    // `aborted` belongs to U6003 alone (`§8` rule 4).
    for _ in 0..3 {
        let (native, events, interpreted) = in_operation("opcancel", "churn(n)", "1000000", &[]);
        assert_eq!(native.status.code(), Some(0), "{native:?}");
        assert_eq!(String::from_utf8_lossy(&native.stdout), "failed\n");
        // The run is not ordered, so only the status and the output must equal the
        // interpreter's (`spec/19 §8` rule 4); the interpreter never starts the second task.
        assert_eq!(native.status.code(), interpreted.status.code());
        assert_eq!(native.stdout, interpreted.stdout);
        assert!(
            events
                .iter()
                .all(|event| event["outcome"]["kind"] != "aborted"),
            "{events:?}"
        );
        let worker = task_events(&events, "2");
        let (last, performed) = worker.split_last().unwrap();
        assert_eq!(
            **last,
            serde_json::json!({"seq": last["seq"], "task": "2", "outcome": {"kind": "cancelled"}})
        );
        // The write, then the operations, each complete with its value: the cancellation took
        // effect at the poll before the next operation.
        let (write, steps) = performed.split_first().unwrap();
        assert_eq!(write["op"], "write", "{worker:?}");
        assert!(!steps.is_empty(), "{worker:?}");
        for step in steps {
            assert_eq!(step["op"], "step", "{worker:?}");
            assert_eq!(
                step["outcome"],
                serde_json::json!({"kind": "ok", "value": "20000"}),
                "{worker:?}"
            );
        }
    }
}

#[test]
fn the_limits_hold_inside_an_operation_in_progress() {
    // #200: while an operation in progress ignores the cancellation, its body still stops at
    // `--max-depth` (U6003): the operation is `aborted`, by U6003, and the task stops with no
    // `cancelled` event. The block fails with the first task's failure, as in the interpreter.
    let (native, events, interpreted) = in_operation(
        "opdeep",
        "churn(n) + deep(1000)",
        "1",
        &["--max-depth", "200"],
    );
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "failed\n");
    assert_eq!(native.status.code(), interpreted.status.code());
    assert_eq!(native.stdout, interpreted.stdout);
    let worker = task_events(&events, "2");
    let (step, performed) = worker.split_last().unwrap();
    assert_eq!(performed.len(), 1, "{worker:?}");
    assert_eq!(performed[0]["op"], "write", "{worker:?}");
    assert_eq!(step["op"], "step", "{worker:?}");
    assert_eq!(
        step["outcome"],
        serde_json::json!({"kind": "aborted"}),
        "{worker:?}"
    );
}

#[test]
fn the_executable_checks_each_operation_against_the_files_modes() {
    // #183: a `files` grant gives the operations of its modes, so `uses Files[read, write]`
    // with only a read grant is U3012 before `main` runs, as `una run` reports it.
    runtime();
    let program = "module copy\n\npub main() -> Unit\n  uses @console: Console[print]\n  uses @files: Files[read, write]\n  fails FileError\n=\n  let text = @files.read(\"in.txt\")\n    if error pass\n  @files.write(\"out.txt\", content: text)\n    if error pass\n  @console.print(\"copied\")\n";
    let dir = package(
        "copy",
        "\n[run]\nentry = \"copy\"\n",
        &[("src/copy.una", program)],
    );
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(dir.join("data/in.txt"), "text").unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let exe = dir.join("target/una/copy");
    let grants = ["--grant", "console", "--grant", "files:data:read"];
    let native = Command::new(&exe)
        .args(grants)
        .current_dir(&dir)
        .output()
        .unwrap();
    let mut run_args = vec!["run", "."];
    run_args.extend(grants);
    let interpreted = una(&dir, &run_args);
    assert_eq!(interpreted.status.code(), Some(1), "{interpreted:?}");
    assert_eq!(native.status.code(), Some(1), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "");
    assert!(!dir.join("data/out.txt").exists());
    let stderr = String::from_utf8_lossy(&native.stderr);
    let message = "[U3012] Error: `main` uses `write` of `Files`, which neither `[run] grants` nor a `--grant` flag grants";
    assert!(stderr.starts_with(message), "{stderr}");
    assert!(
        String::from_utf8_lossy(&interpreted.stderr).starts_with(message),
        "{interpreted:?}"
    );
    let both = Command::new(&exe)
        .args(["--grant", "console", "--grant", "files:data:read,write"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(both.status.code(), Some(0), "{both:?}");
    assert_eq!(String::from_utf8_lossy(&both.stdout), "copied\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The program of [`the_executable_confines_files_through_links_as_una_run_does_1343`]: it
/// reads or writes each path and prints the outcome.
#[cfg(unix)]
fn confined_program(cases: &[(&str, String)]) -> String {
    let mut program = String::from(
        "module files\n\ndescribe(failure: FileError) -> Text\n=\n  when failure\n    is NotFound -> \"not found\"\n    is Denied -> \"denied\"\n    is Io -> \"io\"\n\nshow(path: Text) -> Text\n  uses @files: Files[read]\n=\n  let content = attempt @files.read(path)\n  when content\n    is Ok -> \"read\"\n    is Err(failure) -> describe(failure)\n\nsave(path: Text) -> Text\n  uses @files: Files[write]\n=\n  let saved = attempt @files.write(path, content: \"entry\")\n  when saved\n    is Ok -> \"written\"\n    is Err(failure) -> describe(failure)\n\npub main() -> Unit\n  uses @console: Console[print]\n  uses @files: Files[read, write]\n=\n",
    );
    for (index, (operation, path)) in cases.iter().enumerate() {
        let call = if *operation == "read" { "show" } else { "save" };
        program.push_str(&format!(
            "  let r{index} = {call}(\"{path}\")\n  @console.print(\"{index} {operation}: {{r{index}}}\")\n"
        ));
    }
    program
}

#[cfg(unix)]
#[test]
fn the_executable_confines_files_through_links_as_una_run_does_1343() {
    // `spec/14 §3`, #1343: the native world resolves each directory from the one before it
    // (`una_interp::disk_step`), with the outcomes of `una run`: links inside and outside the
    // prefix, dangling links, a link loop, more links in one path than the host follows, and
    // a lexical path longer than the host takes.
    use std::os::unix::fs::symlink;
    runtime();
    let depth = 40;
    let mut cases: Vec<(&str, String)> = [
        "in_dir/inner.txt",
        "in_dir/../note.txt",
        "out_dir/secret.txt",
        "dangling",
        "dangling_dir/new.txt",
        "in_link",
        "loop/x",
        "up/box/note.txt",
        "back/note.txt",
        "missing/new.txt",
        "note.txt",
    ]
    .into_iter()
    .flat_map(|path| [("read", path.to_owned()), ("write", path.to_owned())])
    .collect();
    for links in [0, 1, 20, 31, 32, 33, 39, 40] {
        let path = format!("{}{}f.txt", "l/".repeat(links), "d/".repeat(depth - links));
        cases.push(("write", path.clone()));
        cases.push(("read", path));
    }
    cases.push(("read", format!("{}note.txt", "self/".repeat(300))));
    let program = confined_program(&cases);
    let dir = package(
        "confined",
        "\n[run]\nentry = \"files\"\ngrants = [\"console\", \"files:box:read,write\"]\n",
        &[("src/files.una", &program)],
    );
    for (path, text) in [
        ("box/note.txt", "inside\n"),
        ("box/sub/inner.txt", "inner\n"),
        ("outside/secret.txt", "secret\n"),
    ] {
        std::fs::create_dir_all(dir.join(path).parent().unwrap()).unwrap();
        std::fs::write(dir.join(path), text).unwrap();
    }
    let boxed = dir.join("box");
    symlink("sub", boxed.join("in_dir")).unwrap();
    symlink("../outside", boxed.join("out_dir")).unwrap();
    symlink("missing_target", boxed.join("dangling")).unwrap();
    symlink("../outside/nope", boxed.join("dangling_dir")).unwrap();
    symlink("note.txt", boxed.join("in_link")).unwrap();
    symlink("loop", boxed.join("loop")).unwrap();
    symlink("..", boxed.join("up")).unwrap();
    symlink("../box", boxed.join("back")).unwrap();
    symlink(".", boxed.join("self")).unwrap();
    let mut at = boxed.clone();
    for _ in 0..depth {
        symlink("d", at.join("l")).unwrap();
        at.push("d");
        std::fs::create_dir(&at).unwrap();
    }
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/confined"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    let interpreted = una(&dir, &["run"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let stdout = String::from_utf8_lossy(&native.stdout);
    assert_eq!(stdout, String::from_utf8_lossy(&interpreted.stdout));
    let outcomes: Vec<&str> = stdout.lines().collect();
    assert_eq!(outcomes.len(), cases.len(), "{stdout}");
    // A few outcomes of `spec/14 §3`, by case.
    for (index, outcome) in [
        (0, "read"),
        (4, "denied"),
        (6, "denied"),
        (11, "denied"),
        (18, "not found"),
        (19, "not found"),
        (20, "read"),
    ] {
        assert_eq!(
            outcomes[index],
            format!("{index} {}: {outcome}", cases[index].0)
        );
    }
    assert!(!dir.join("outside/new.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn the_executable_denies_a_directory_past_the_host_path_limit_as_una_run_does_1380() {
    // `spec/14 §3` rule 2, #1380: a directory that exists but whose canonical path is longer
    // than the host takes does not resolve; it is denied in both engines, not opened through
    // the link below it, while a directory that does not exist still fails with `NotFound`.
    // Each hop `h` is a link to the four directories below it, so the lexical paths stay
    // short while the canonical paths grow.
    use std::os::unix::fs::symlink;
    runtime();
    let dir = package(
        "deep",
        "\n[run]\nentry = \"files\"\ngrants = [\"console\", \"files:box:read,write\"]\n",
        &[("src/files.una", &confined_program(&[]))],
    );
    let name = "X".repeat(100);
    let deep = |levels: usize| vec![name.as_str(); levels].join("/");
    let boxed = dir.join("box");
    std::fs::create_dir_all(dir.join("outside")).unwrap();
    std::fs::write(dir.join("outside/secret.txt"), "secret\n").unwrap();
    std::fs::create_dir(&boxed).unwrap();
    let mut hops = String::new();
    let mut at = boxed.clone();
    let mut past = None;
    for _ in 0..64 {
        let mut directory = at.clone();
        for _ in 0..4 {
            directory.push(&name);
            std::fs::create_dir(&directory).unwrap();
        }
        symlink(deep(4), at.join("h")).unwrap();
        past = (1..=4).find(|&levels| std::fs::canonicalize(at.join(deep(levels))).is_err());
        if past.is_some() {
            break;
        }
        at.push("h");
        hops.push_str("h/");
    }
    let levels = past.unwrap_or(1);
    symlink(dir.join("outside"), at.join(deep(levels)).join("esc")).unwrap();
    let escape = format!("{hops}{}/esc", deep(levels));
    let cases: Vec<(&str, String)> = vec![
        ("read", format!("{escape}/secret.txt")),
        ("write", format!("{escape}/new.txt")),
        ("read", format!("{hops}{}/missing.txt", deep(levels))),
        ("read", "h/missing/new.txt".to_owned()),
        ("write", "h/missing/new.txt".to_owned()),
    ];
    std::fs::write(dir.join("src/files.una"), confined_program(&cases)).unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/deep"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    let interpreted = una(&dir, &["run"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let stdout = String::from_utf8_lossy(&native.stdout);
    assert_eq!(stdout, String::from_utf8_lossy(&interpreted.stdout));
    let mut expected = vec!["0 read: denied", "1 write: denied"];
    // The directory past the limit is denied on a host that has one, and otherwise opened.
    expected.push(if past.is_some() {
        "2 read: denied"
    } else {
        "2 read: not found"
    });
    expected.extend(["3 read: not found", "4 write: not found"]);
    assert_eq!(stdout.lines().collect::<Vec<_>>(), expected, "{past:?}");
    // The limit is 1024 bytes on macOS and, on Linux, the C library's: 4096 bytes with glibc,
    // lower with musl (#1456).
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        assert!(past.is_some());
    }
    assert!(!dir.join("outside/new.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Builds the package `name` of edition 0.2 whose entry module `closed` is `source`, and
/// returns its directory and executable.
fn closed_package(name: &str, source: &str) -> (PathBuf, PathBuf) {
    let dir = package(
        name,
        "\n[run]\nentry = \"closed\"\ngrants = [\"console\"]\n",
        &[("src/closed.una", source)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let executable = dir.join("target/una").join(name);
    (dir, executable)
}

/// Runs `executable` with the descriptor `fd` closed, as `... 1>&-` does in a shell.
fn run_closed(executable: &Path, fd: u8) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(format!("exec \"$0\" {fd}>&-"))
        .arg(executable)
        .output()
        .unwrap()
}

#[test]
fn a_closed_stream_discards_the_output_and_a_broken_pipe_fails_natively() {
    // `spec/13 §5`, `spec/16 §3` (#259): natively as in the interpreter, a closed stdout
    // or stderr discards what is written to it, while a reader that went away (EPIPE) is a
    // failure to write output, exit code 2.
    let (prints_dir, prints) = closed_package(
        "closed-prints",
        "module closed\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"hello\")\n",
    );
    let run = run_closed(&prints, 1);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    let (fails_dir, fails) = closed_package(
        "closed-fails",
        "module closed\n\npub kind Stop = | Stopped\n\npub main() -> Unit\n  fails Stop\n=\n  fail Stopped\n",
    );
    let run = run_closed(&fails, 2);
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    let (floods_dir, floods) = closed_package(
        "closed-floods",
        "module closed\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  for i in 0..<20000\n    @console.print(\"a line of output that fills the pipe\")\n",
    );
    let mut child = Command::new(&floods)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    for dir in [prints_dir, fails_dir, floods_dir] {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A program that appends `steps` items to a list in a loop, through `shape`: an `edit`
/// argument one, two or three calls deep, or a binding rebound from a call's result (#262).
fn appends_through_calls(shape: &str, steps: usize) -> String {
    let call = match shape {
        "edit1" => "push_one(edit a, i: i)",
        "edit2" => "push_two(edit a, i: i)",
        "edit3" => "push_three(edit a, i: i)",
        _ => "a = pushed(a, i: i)",
    };
    format!(
        "module grow\n\npush_one(edit acc: List[Int], i: Int) -> Unit\n=\n  acc = acc.append(item: i)\n\npush_two(edit acc: List[Int], i: Int) -> Unit\n=\n  push_one(edit acc, i: i)\n\npush_three(edit acc: List[Int], i: Int) -> Unit\n=\n  push_two(edit acc, i: i)\n\npushed(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit a: List[Int] = []\n  for i in 0..<{steps}\n    {call}\n  @console.print(\"{{a.len}}\")\n"
    )
}

#[test]
fn a_list_appended_through_calls_grows_in_linear_time() {
    // ADR 0072 D1 and D2 (#262): natively too, an `edit` argument, or a binding at its last
    // use rebound from the result, moves into the callee's owned parameter, which appends in
    // place through any number of call levels, copying nothing: the copy count is the primary
    // check. The timing is the secondary one: each tenfold step stays under 30x, against about
    // 100x when every call copies the list. A miss is measured twice more before it fails
    // (#341).
    let runs: &[(&str, &[usize])] = &[
        ("edit1", &[10_000, 100_000]),
        ("edit2", &[10_000, 100_000]),
        ("edit3", &[10_000, 100_000, 1_000_000]),
        ("rebound", &[10_000, 100_000]),
    ];
    for &(shape, sizes) in runs {
        let dirs: Vec<PathBuf> = sizes
            .iter()
            .map(|&steps| {
                let dir = package(
                    &format!("calls-{shape}-{steps}"),
                    "\n[run]\nentry = \"grow\"\ngrants = [\"console\"]\n",
                    &[("src/grow.una", &appends_through_calls(shape, steps))],
                );
                let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
                std::fs::write(
                    dir.join("una.toml"),
                    manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
                )
                .unwrap();
                let built = una(&dir, &["build"]);
                assert_eq!(built.status.code(), Some(0), "{built:?}");
                dir
            })
            .collect();
        // Times a run of the executable built for `steps` appends.
        let measure = |steps: usize| {
            let dir = &dirs[sizes.iter().position(|&s| s == steps).unwrap()];
            let (output, seconds) = timing::timed(|| {
                Command::new(
                    dir.join("target/una")
                        .join(format!("calls-{shape}-{steps}")),
                )
                .current_dir(dir)
                .output()
                .unwrap()
            });
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!("{steps}\n")
            );
            // Nothing is copied: every call takes the list over.
            assert!(
                !String::from_utf8_lossy(&output.stderr).contains("copied"),
                "{shape} {steps}: {output:?}"
            );
            seconds
        };
        timing::assert_growth(shape, sizes, 0.0, 0.05, 30.0, measure);
        for dir in &dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Functions that consume their list parameter on one path only, and let it escape on the
/// other: returned, stored in a record or a list, captured, or passed to an owned parameter
/// of another function (#284).
const BORROWED_ENTRY_FUNCTIONS: &str = "module rc\n\npub kind Box\n  items: List[Int]\n\ncons(xs: List[Int], i: Int) -> Int = if i < 0 then xs.append(item: i).len else xs.len\n\nkeep(xs: List[Int], i: Int) -> List[Int] = if i < 0 then xs.append(item: i) else xs\n\nboxed(xs: List[Int], i: Int) -> Box = if i < 0 then Box(items: xs.append(item: i)) else Box(items: xs)\n\nlisted(xs: List[Int], i: Int) -> List[List[Int]] = if i < 0 then [xs.append(item: i)] else [xs, xs]\n\nadder(xs: List[Int], i: Int) -> fn(Int) -> Int = if i < 0 then fn(n) -> xs.append(item: n).len else fn(n) -> xs.len + n\n\ngrown(xs: List[Int], i: Int) -> List[Int] = xs.append(item: i)\n\nnested(xs: List[Int], i: Int) -> Int = if i < 0 then grown(xs, i: i).len else xs.len\n\nboth(xs: List[Int], ys: List[Int], i: Int) -> Int = if i < 0 then xs.append(item: i).len + ys.append(item: i).len else xs.len + ys.len\n\ngo(acc: List[Int], i: Int, n: Int) -> List[Int] = if i == n then acc else go(acc.append(item: i), i: i + 1, n: n)\n\nfill(acc: List[Int], n: Int) -> List[Int]\n=\n  let edit a: List[Int] = acc\n  for i in 0..<n\n    a = grown(a, i: i)\n  a\n";

/// Moves the package at `dir` to edition 0.2, whose `edit` bindings the programs use.
fn edition_0_2(dir: &Path) {
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
}

#[test]
fn a_call_that_would_retain_every_owned_argument_takes_the_borrowed_entry() {
    // #284: a call whose arguments of owned parameters all stay live takes the entry that
    // borrows them, with no reference counting at the call; one where an argument moves takes
    // the owned entry. Each case runs in its own executable, linked with the debug runtime,
    // which fails with status 3 when a value is used after its release or still referenced
    // once `main` returns: every reference that an escaping parameter takes is counted. The
    // output is the interpreter's, and the copies are those the owned entry made before.
    let cases: &[(&str, &str, &str, usize)] = &[
        // Borrowed entry, both paths: the consuming one copies the list it borrows.
        (
            "borrowed",
            "let a = cons(xs, i: 1)\n  let b = cons(xs, i: -1)\n  @console.print(\"{a} {b} {xs.len}\")",
            "3 4 3",
            1,
        ),
        // Owned entry: the list moves in and grows in place.
        (
            "owned",
            "let fresh = [9]\n  let c = cons(fresh, i: -1)\n  @console.print(\"{c} {xs.len}\")",
            "2 3",
            0,
        ),
        // Mixed: `ys` moves, `xs` stays live, so the owned entry, retaining `xs`.
        (
            "mixed",
            "let ys = [4]\n  let m = both(xs, ys: ys, i: -1)\n  @console.print(\"{m} {xs.len}\")",
            "6 3",
            1,
        ),
        // Recursion from the borrowed entry to the owned one after the first append.
        (
            "recursion",
            "let r = go(xs, i: 0, n: 5)\n  @console.print(\"{r.len} {xs.len}\")",
            "8 3",
            1,
        ),
        (
            "returned",
            "let k = keep(xs, i: 1)\n  let k2 = keep(xs, i: -1)\n  @console.print(\"{k.len} {k2.len} {xs.len}\")",
            "3 4 3",
            1,
        ),
        (
            "record",
            "let bx = boxed(xs, i: 1)\n  let by = boxed(xs, i: -1)\n  @console.print(\"{bx.items.len} {by.items.len} {xs.len}\")",
            "3 4 3",
            1,
        ),
        (
            "list",
            "let ls = listed(xs, i: 1)\n  let lt = listed(xs, i: -1)\n  @console.print(\"{ls.len} {lt.len} {xs.len}\")",
            "2 1 3",
            1,
        ),
        (
            "captured",
            "let f = adder(xs, i: 1)\n  let g = adder(xs, i: -1)\n  @console.print(\"{f(1)} {g(1)} {g(2)} {xs.len}\")",
            "4 4 4 3",
            2,
        ),
        (
            "nested",
            "let d = nested(xs, i: -1)\n  let e = nested(xs, i: 1)\n  @console.print(\"{d} {e} {xs.len}\")",
            "4 3 3",
            1,
        ),
    ];
    for &(case, body, expected, copies) in cases {
        let program = format!(
            "{BORROWED_ENTRY_FUNCTIONS}\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let xs = [1, 2, 3]\n  {body}\n"
        );
        let name = format!("borrowed-entry-{case}");
        let dir = package(
            &name,
            "\n[run]\nentry = \"rc\"\ngrants = [\"console\"]\n",
            &[("src/rc.una", &program)],
        );
        edition_0_2(&dir);
        let interpreted = una(&dir, &["run", "."]);
        assert_eq!(
            interpreted.status.code(),
            Some(0),
            "{case}: {interpreted:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&interpreted.stdout),
            format!("{expected}\n"),
            "{case}"
        );
        let built = una(&dir, &["build"]);
        assert_eq!(built.status.code(), Some(0), "{case}: {built:?}");
        let output = Command::new(dir.join("target/una").join(&name))
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{case}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{expected}\n"),
            "{case}"
        );
        let noted = if copies == 0 {
            String::new()
        } else {
            format!("note: una-rt (debug build): lists, maps and texts copied: {copies}\n")
        };
        assert_eq!(String::from_utf8_lossy(&output.stderr), noted, "{case}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_list_given_while_live_then_appended_through_calls_grows_in_linear_time() {
    // #284: `fill` owns its list, but the caller's `xs` stays live, so the call takes the
    // borrowed entry. Its loop takes the list as its state, copied once at the first append;
    // each later call of `grown` moves the state into the owned entry, which appends in
    // place. One copy at every size is the primary check; the timing, each tenfold step under
    // 30x, the secondary one (#341).
    let sizes: &[usize] = &[10_000, 100_000, 1_000_000];
    let dirs: Vec<PathBuf> = sizes
        .iter()
        .map(|&steps| {
            let program = format!(
                "{BORROWED_ENTRY_FUNCTIONS}\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let xs = [1, 2, 3]\n  let r = fill(xs, n: {steps})\n  @console.print(\"{{r.len}} {{xs.len}}\")\n"
            );
            let dir = package(
                &format!("live-then-owned-{steps}"),
                "\n[run]\nentry = \"rc\"\ngrants = [\"console\"]\n",
                &[("src/rc.una", &program)],
            );
            edition_0_2(&dir);
            let built = una(&dir, &["build"]);
            assert_eq!(built.status.code(), Some(0), "{built:?}");
            dir
        })
        .collect();
    let measure = |steps: usize| {
        let dir = &dirs[sizes.iter().position(|&s| s == steps).unwrap()];
        let (output, seconds) = timing::timed(|| {
            Command::new(
                dir.join("target/una")
                    .join(format!("live-then-owned-{steps}")),
            )
            .current_dir(dir)
            .output()
            .unwrap()
        });
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{} 3\n", steps + 3)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "note: una-rt (debug build): lists, maps and texts copied: 1\n"
        );
        seconds
    };
    timing::assert_growth("live-then-owned", sizes, 0.0, 0.05, 30.0, measure);
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn json_parses_and_writes_in_linear_cpu_time_natively() {
    // ADR 0073 J6 (#238): natively, the debug runtime's count of copies is the same at every
    // size, so nothing is copied per item; the timing is a second check, wide in margin: each
    // tenfold step stays under 30x, against about 100x for quadratic work. A miss is measured
    // twice more before it fails (#341).
    let sizes = [10_000, 100_000, 1_000_000];
    let dirs: Vec<PathBuf> = sizes
        .iter()
        .map(|steps| {
            let program = format!(
                "module grow\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit items: List[Json] = []\n  for i in 0..<{steps}\n    items = items.append(item: Number(i))\n  for i in 0..<{steps}\n    items = items.append(item: Str(\"item {{i}}\"))\n  let text = Array(items).to_json()\n  let back = text.parse_json()\n    if error use Null\n  let again = back.to_json()\n  @console.print(\"{{text.len}} {{again == text}}\")\n"
            );
            let dir = package(
                &format!("json-scaling-{steps}"),
                "\n[run]\nentry = \"grow\"\ngrants = [\"console\"]\n",
                &[("src/grow.una", &program)],
            );
            let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
            std::fs::write(
                dir.join("una.toml"),
                manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
            )
            .unwrap();
            let built = una(&dir, &["build"]);
            assert_eq!(built.status.code(), Some(0), "{built:?}");
            dir
        })
        .collect();
    let mut first_note: Option<String> = None;
    // Times a run of the executable built for `steps` items, and checks its notes.
    let measure = |steps: usize| {
        let dir = &dirs[sizes.iter().position(|&s| s == steps).unwrap()];
        let (output, seconds) = cpu::cpu_output(
            Command::new(dir.join("target/una").join(format!("json-scaling-{steps}")))
                .current_dir(dir),
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).ends_with(" true\n"),
            "{output:?}"
        );
        // The primary check: the same count of copies at every size, on every run.
        let note = String::from_utf8_lossy(&output.stderr).into_owned();
        match &first_note {
            Some(first) => assert_eq!(&note, first, "{steps} items"),
            None => first_note = Some(note),
        }
        seconds
    };
    timing::assert_growth("items", &sizes, 0.0, 0.05, 30.0, measure);
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A text of `COUNT` copies of `ab`, searched, replaced and split.
const TEXT_SCALE: &str = "module scale\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit parts: List[Text] = []\n  for i in 0..<COUNT\n    parts = parts.append(item: \"ab\")\n  let empty = \"\"\n  let t = parts.join(separator: empty)\n  let c = \"c\"\n  let a = \"a\"\n  let b = \"b\"\n  let missing = t.find(text: c)\n  let swapped = t.replace(pattern: a, by: c)\n  let pieces = t.split(on: b)\n  @console.print(\"{t.len} {missing} {swapped.len} {pieces.len}\")\n";

#[test]
fn native_text_search_replace_and_split_take_linear_time() {
    // `spec/22 §2` rule 8 and the performance rule, natively: ten times the text takes under
    // 30 times as long, about 10x when linear against about 100x when quadratic. A miss is
    // measured twice more before it fails (#341).
    let sizes = [100_000, 1_000_000];
    let dirs: Vec<PathBuf> = sizes
        .iter()
        .map(|count| {
            let dir = package(
                &format!("text_{count}"),
                "\n[run]\nentry = \"scale\"\ngrants = [\"console\"]\n",
                &[(
                    "src/scale.una",
                    &TEXT_SCALE.replace("COUNT", &count.to_string()),
                )],
            );
            let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
            std::fs::write(
                dir.join("una.toml"),
                manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
            )
            .unwrap();
            let built = una(&dir, &["build"]);
            assert_eq!(built.status.code(), Some(0), "{built:?}");
            dir
        })
        .collect();
    // Times a run of the executable built for `count` pieces.
    let measure = |count: usize| {
        let dir = &dirs[sizes.iter().position(|&s| s == count).unwrap()];
        let (output, seconds) = timing::timed(|| {
            Command::new(dir.join(format!("target/una/text_{count}")))
                .current_dir(dir)
                .output()
                .unwrap()
        });
        assert_eq!(output.status.code(), Some(0));
        let (len, pieces) = (2 * count, count + 1);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{len} None {len} {pieces}\n")
        );
        seconds
    };
    timing::assert_growth("text", &sizes, 0.0, timing::FLOOR, 30.0, measure);
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A non-ASCII text of `COUNT` scalar values, built by appends, then each one sliced.
const SLICE_SCALE: &str = "module scale

pub main() -> Unit
  uses @console: Console[print]
=
  let edit t = \"\"
  for i in 0..<COUNT
    t = \"{t}é\"
  let edit total = 0
  for i in 0..<COUNT
    if i + 1 <= t.len
      total = total + t.slice(start: i, end: i + 1).len + t.len
  @console.print(\"{t.len} {total}\")
";

#[test]
fn native_text_len_and_slice_take_constant_time_on_any_text() {
    // `spec/22 §6` rule 1, natively: a slice at each of n positions of a non-ASCII text takes
    // linear time. Ten times the text takes under 30 times as long, about 10x when linear
    // against about 100x when a position costs O(n). A miss is measured twice more before it
    // fails (#341).
    let sizes = [100_000, 1_000_000];
    let dirs: Vec<PathBuf> = sizes
        .iter()
        .map(|count| {
            let dir = package(
                &format!("slice_{count}"),
                "\n[run]\nentry = \"scale\"\ngrants = [\"console\"]\n",
                &[(
                    "src/scale.una",
                    &SLICE_SCALE.replace("COUNT", &count.to_string()),
                )],
            );
            let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
            std::fs::write(
                dir.join("una.toml"),
                manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
            )
            .unwrap();
            let built = una(&dir, &["build"]);
            assert_eq!(built.status.code(), Some(0), "{built:?}");
            dir
        })
        .collect();
    // Times a run of the executable built for `count` scalar values.
    let measure = |count: usize| {
        let dir = &dirs[sizes.iter().position(|&s| s == count).unwrap()];
        let (output, seconds) = timing::timed(|| {
            Command::new(dir.join(format!("target/una/slice_{count}")))
                .current_dir(dir)
                .output()
                .unwrap()
        });
        assert_eq!(output.status.code(), Some(0));
        // Each of the `count` slices adds 1 and the length `count`.
        let total = count * (count + 1);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{count} {total}\n")
        );
        seconds
    };
    timing::assert_growth("text", &sizes, 0.0, timing::FLOOR, 30.0, measure);
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A text of 2^`LOG` * 2 scalar values, `ab` repeated, built by doubling, then `K` rounds of one
/// read, `OPERATION`, against a prefix, a suffix or a text that differs at the first scalar value.
const TEXT_READS_SCALE: &str = "module scale

pub main() -> Unit
  uses @console: Console[print]
=
  let edit t = \"ab\"
  for i in 0..<LOG
    t = \"{t}{t}\"
  let p = \"ab\"
  let u = \"b{t}\"
  let edit hits = 0
  for i in 0..<K
    if OPERATION
      hits = hits + 1
  @console.print(\"{t.len} {hits}\")
";

/// A list of `N` texts `ab`, and a long item of 2^`LOG` * 2 scalar values, built by doubling.
const TEXT_CONTAINS_SCALE: &str = "module scale

pub main() -> Unit
  uses @console: Console[print]
=
  let edit xs: List[Text] = []
  for i in 0..<N
    xs = xs.append(item: \"ab\")
  let edit item = \"ab\"
  for i in 0..<LOG
    item = \"{item}{item}\"
  let ab = \"ab\"
  @console.print(\"{xs.contains(item: item)} {xs.contains(item: ab)}\")
";

/// Builds `source` with `replacements` applied, once for each of `values`, in edition 0.2, and
/// returns the packages' directories.
fn text_scale_packages(
    name: &str,
    source: &str,
    values: &[usize],
    key: &str,
    fixed: &[(&str, usize)],
) -> Vec<PathBuf> {
    values
        .iter()
        .map(|value| {
            let mut text = source.replace(key, &value.to_string());
            for (token, number) in fixed {
                text = text.replace(token, &number.to_string());
            }
            let dir = package(
                &format!("{name}_{value}"),
                "\n[run]\nentry = \"scale\"\ngrants = [\"console\"]\n",
                &[("src/scale.una", &text)],
            );
            let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
            std::fs::write(
                dir.join("una.toml"),
                manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
            )
            .unwrap();
            let built = una(&dir, &["build"]);
            assert_eq!(built.status.code(), Some(0), "{built:?}");
            dir
        })
        .collect()
}

#[test]
fn native_text_reads_stop_at_the_first_difference_whatever_the_text_size() {
    // `spec/22 §2` (`ends_with`, O(m)) and the complexity rule of the Proposed ADR for the
    // standard library: `starts_with`, `ends_with`, `==` and `<` on a text of b scalar values
    // and a short operand take constant time, so a text of 4096 times the size takes under 4
    // times as long, about 1x in place against 25x or more when each call copies the text
    // (#1341). Each operation is measured alone, so a miss names it. A miss is measured twice
    // more before it fails (#341).
    const ROUNDS: usize = 10_000;
    let logs = [8, 20];
    // The sizes are the scalar values of the text or the item: 2 << log.
    let sizes: Vec<usize> = logs.iter().map(|log| 2usize << log).collect();
    // The operation, its test, and how many rounds it holds on: `==` holds on none.
    let operations = [
        ("starts_with", "t.starts_with(prefix: p)", ROUNDS),
        ("ends_with", "t.ends_with(suffix: p)", ROUNDS),
        ("equal", "t == u", 0),
        ("less", "t < u", ROUNDS),
    ];
    for (name, test, holds) in operations {
        let source = TEXT_READS_SCALE.replace("OPERATION", test);
        let dirs = text_scale_packages(
            &format!("reads_{name}"),
            &source,
            &logs,
            "LOG",
            &[("K", ROUNDS)],
        );
        // The fixed cost of a run: the same program with no rounds, on the smaller text.
        let idle = text_scale_packages(
            &format!("reads_idle_{name}"),
            &source,
            &[8],
            "LOG",
            &[("K", 0)],
        );
        let run_idle = || {
            timing::timed(|| {
                Command::new(idle[0].join(format!("target/una/reads_idle_{name}_8")))
                    .current_dir(&idle[0])
                    .output()
                    .unwrap()
            })
            .1
        };
        let fixed = timing::best_of_three(run_idle);
        let measure = |size: usize| {
            let log = size.trailing_zeros() as usize - 1;
            let dir = &dirs[logs.iter().position(|&s| s == log).unwrap()];
            let (output, seconds) = timing::timed(|| {
                Command::new(dir.join(format!("target/una/reads_{name}_{log}")))
                    .current_dir(dir)
                    .output()
                    .unwrap()
            });
            assert_eq!(output.status.code(), Some(0), "{name}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!("{} {holds}\n", 2usize << log),
                "{name}"
            );
            seconds
        };
        timing::assert_growth(name, &sizes, fixed, timing::FLOOR, 4.0, measure);
        for dir in dirs.iter().chain(&idle) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[test]
fn native_contains_over_short_texts_does_not_depend_on_the_length_of_the_item() {
    // `spec/22 §2` and the complexity rule of the Proposed ADR for the standard library:
    // `contains` of a long item over 100 000 short texts compares
    // each element in O(1) when the lengths differ, so the cost is linear in size(xs) and a
    // 16 times longer item takes under 4 times as long, about 1x in place against about 16x
    // when each comparison copies the item (#1341). A miss is measured twice more (#341).
    let logs = [8, 20];
    // The sizes are the scalar values of the text or the item: 2 << log.
    let sizes: Vec<usize> = logs.iter().map(|log| 2usize << log).collect();
    let dirs = text_scale_packages(
        "contains",
        TEXT_CONTAINS_SCALE,
        &logs,
        "LOG",
        &[("N", 100_000)],
    );
    // The fixed cost of a run: the same program over an empty list, on the shorter item.
    let idle = text_scale_packages(
        "contains_idle",
        TEXT_CONTAINS_SCALE,
        &[8],
        "LOG",
        &[("N", 0)],
    );
    let run_idle = || {
        timing::timed(|| {
            Command::new(idle[0].join("target/una/contains_idle_8"))
                .current_dir(&idle[0])
                .output()
                .unwrap()
        })
        .1
    };
    let fixed = timing::best_of_three(run_idle);
    let measure = |size: usize| {
        let log = size.trailing_zeros() as usize - 1;
        let dir = &dirs[logs.iter().position(|&s| s == log).unwrap()];
        let (output, seconds) = timing::timed(|| {
            Command::new(dir.join(format!("target/una/contains_{log}")))
                .current_dir(dir)
                .output()
                .unwrap()
        });
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "false true\n");
        seconds
    };
    timing::assert_growth("contains", &sizes, fixed, timing::FLOOR, 4.0, measure);
    for dir in dirs.iter().chain(&idle) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A program that appends `steps` items to a list through a function value, by `shape`: a
/// named function passed as a value (`named`), a lambda (`lambda`), two layers of lambdas over
/// a function value (`layers`), or `fold` over a list (`fold`) or into a text (`text`).
fn appends_through_function_values(shape: &str, steps: usize) -> String {
    let body = match shape {
        "named" => format!("  let a = run_with(r1, n: {steps})\n  @console.print(\"{{a.len}}\")\n"),
        "lambda" => format!(
            "  let a = run_with(fn(acc, i) -> acc.append(item: i), n: {steps})\n  @console.print(\"{{a.len}}\")\n"
        ),
        "layers" => {
            format!("  let a = twice_layered(r1, n: {steps})\n  @console.print(\"{{a.len}}\")\n")
        }
        "fold" => format!(
            "  let a = numbers({steps}).fold(initial: [], step: fn(acc, x) -> acc.append(item: x))\n  @console.print(\"{{a.len}}\")\n"
        ),
        _ => format!(
            "  let t = numbers({steps}).fold(initial: \"\", step: fn(line, x) -> \"{{line}}x\")\n  @console.print(\"{{t.len}}\")\n"
        ),
    };
    format!(
        "module grow\n\nr1(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)\n\nrun_with(f: fn(List[Int], Int) -> List[Int], n: Int) -> List[Int]\n=\n  let edit acc: List[Int] = []\n  for i in 0..<n\n    acc = f(acc, i)\n  acc\n\nlayered(f: fn(List[Int], Int) -> List[Int], n: Int) -> List[Int] = run_with(fn(acc, i) -> f(acc, i), n: n)\n\ntwice_layered(f: fn(List[Int], Int) -> List[Int], n: Int) -> List[Int] = layered(fn(acc, i) -> f(acc, i), n: n)\n\nnumbers(n: Int) -> List[Int]\n=\n  let edit xs: List[Int] = []\n  for i in 0..<n\n    xs = xs.append(item: i)\n  xs\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n{body}"
    )
}

/// Function values that a list goes through twice, or that capture it, never change it.
const ALIASED: &str = "module aliased\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let base = [1, 2]\n  let f = fn(acc: List[Int], i: Int) -> acc.append(item: i)\n  let a = f(base, 3)\n  let b = f(base, 4)\n  let g = fn(i: Int) -> base.append(item: i)\n  let c = g(5)\n  let d = g(6)\n  let folded = [7, 8].fold(initial: base, step: fn(acc, x) -> acc.append(item: x))\n  @console.print(\"{base} {a} {b} {c} {d} {folded} {base}\")\n";

/// What [`ALIASED`] prints.
const ALIASED_OUTPUT: &str = "[1, 2] [1, 2, 3] [1, 2, 4] [1, 2, 5] [1, 2, 6] [1, 2, 7, 8] [1, 2]\n";

#[test]
fn a_list_built_through_function_values_grows_in_linear_time() {
    // #312 (IP-1, IP-2): a call through a function value moves its arguments into the
    // lambda, which owns them, so a list or a text appended through function values and `fold`
    // grows in place: the debug runtime's count of copies is the same at every size, 0, or 1
    // for the literal a text starts from: the copy count is the primary check. The timing is
    // the secondary one: each tenfold step stays under 30x, against about 100x when every call
    // copies. A miss is measured twice more before it fails (#341).
    let runs: &[(&str, &[usize])] = &[
        ("named", &[10_000, 100_000]),
        ("lambda", &[10_000, 100_000]),
        ("layers", &[10_000, 100_000, 1_000_000]),
        ("fold", &[10_000, 100_000]),
        ("text", &[10_000, 100_000]),
    ];
    for &(shape, sizes) in runs {
        let dirs: Vec<PathBuf> = sizes
            .iter()
            .map(|&steps| {
                let dir = package(
                    &format!("values-{shape}-{steps}"),
                    "\n[run]\nentry = \"grow\"\ngrants = [\"console\"]\n",
                    &[(
                        "src/grow.una",
                        &appends_through_function_values(shape, steps),
                    )],
                );
                let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
                std::fs::write(
                    dir.join("una.toml"),
                    manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
                )
                .unwrap();
                let built = una(&dir, &["build"]);
                assert_eq!(built.status.code(), Some(0), "{built:?}");
                dir
            })
            .collect();
        let mut first_copies = None;
        // Times a run of the executable built for `steps` appends, and checks its copies.
        let measure = |steps: usize| {
            let dir = &dirs[sizes.iter().position(|&s| s == steps).unwrap()];
            let (output, seconds) = timing::timed(|| {
                Command::new(
                    dir.join("target/una")
                        .join(format!("values-{shape}-{steps}")),
                )
                .current_dir(dir)
                .output()
                .unwrap()
            });
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!("{steps}\n")
            );
            // The copies do not grow with the size: none, or the one copy of the literal `""`
            // that the first append into the text makes.
            let copies = copied(&output);
            let first = *first_copies.get_or_insert(copies);
            assert!(
                copies == first && copies <= 1,
                "{shape} {steps}: {copies} copies, {first} at the first size"
            );
            seconds
        };
        timing::assert_growth(shape, sizes, 0.0, 0.05, 30.0, measure);
        for dir in &dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// The copies the debug runtime reports on stderr, 0 when it reports none.
/// The records and variants the debug runtime made anew, from its note (#292), or 0.
fn made_anew(output: &std::process::Output) -> u64 {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|line| line.split("made anew, not reused: ").nth(1))
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or(0)
}

fn copied(output: &std::process::Output) -> u64 {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|line| line.split("copied: ").nth(1))
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or(0)
}

#[test]
fn function_values_never_change_a_list_another_name_holds() {
    // A function value called twice with one list, a closure that captures it, and a
    // `fold` that starts from it each see it unchanged.
    let dir = package(
        "aliased",
        "\n[run]\nentry = \"aliased\"\ngrants = [\"console\"]\n",
        &[("src/aliased.una", ALIASED)],
    );
    let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let output = Command::new(dir.join("target/una/aliased"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), ALIASED_OUTPUT);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_native_executable_carries_no_explain_text_413() {
    // #413: the explain texts live in `una-explain`, which only the command line links;
    // the runtime links `una-diag` for codes and rendering, so an executable holds none of them.
    // They were 75,708 bytes, about 5.3 % of `examples/hello`'s 1,430,424.
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-explain-text");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let package = copy(&root().join("examples/hello"), &scratch);
    let exe = scratch.join("hello-exe");
    let built = una(&package, &["build", ".", "--out", exe.to_str().unwrap()]);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let bytes = std::fs::read(&exe).unwrap();
    let holds = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(
        !holds("## Failing example"),
        "the executable holds an explain text"
    );
    for code in una_diag::Code::ALL {
        // The first sentence of each text, which no other string of the runtime repeats.
        let text = una_explain::text(code);
        let body = text.split("\n\n").nth(1).unwrap_or("");
        let sentence: String = body.chars().take(60).collect();
        assert!(
            sentence.len() < 20 || !holds(&sentence),
            "the executable holds the explain text of {code}: {sentence:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn records_made_anew_grow_with_the_loop_and_copies_do_not_292() {
    // #292: a field edit of a record that a list still holds makes the record anew, a constant
    // cost each time, so the count of records made anew grows with the loop while the copies,
    // which scaling tests bound, stay the same at every size.
    let program = |steps: usize| {
        format!(
            "module history\n\npub kind Point\n  x: Int\n  y: Int\n\npub walk(n: Int) -> Int\n=\n  let edit q = Point(x: 0, y: 0)\n  let edit seen: List[Point] = []\n  for i in 0..<n\n    seen = seen.append(item: q)\n    q.x = q.x + 1\n  q.x + seen.len\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let walked = walk({steps})\n  @console.print(\"{{walked}}\")\n"
        )
    };
    let mut first_copies = None;
    for steps in [1_000_usize, 4_000] {
        let dir = package(
            "history",
            "\n[run]\nentry = \"history\"\ngrants = [\"console\"]\n",
            &[("src/history.una", &program(steps))],
        );
        let manifest = std::fs::read_to_string(dir.join("una.toml")).unwrap();
        std::fs::write(
            dir.join("una.toml"),
            manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
        )
        .unwrap();
        let built = una(&dir, &["build"]);
        assert_eq!(built.status.code(), Some(0), "{built:?}");
        let output = Command::new(dir.join("target/una/history"))
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{}\n", 2 * steps)
        );
        assert_eq!(made_anew(&output), steps as u64, "{steps}: {output:?}");
        let copies = copied(&output);
        assert_eq!(
            *first_copies.get_or_insert(copies),
            copies,
            "{steps}: {output:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A package of edition 0.4, with `ita.una` holding `sections` after its head, and the lock of
/// the conformance cases.
fn ita_package(name: &str, sections: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-native-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!("ita {name}\n  version 0.1.0\n  edition 0.4\n{sections}"),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/types/float_edition_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    for (path, text) in files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    dir
}

const COUNTER: &str = "module counter

import std.drive (Tick)

pub init() -> Int = 0

pub step(edit count: Int, input: Tick) -> Unit
  uses @console: Console[print]
=
  count = count + 1
  @console.print(\"step {count}\")

pub on_stop(count: Int) -> Unit
  uses @console: Console[print]
=
  @console.print(\"stopped after {count}\")
";

const DRIVE: &str = "\nrun\n  grants\n    console\n\ndrive\n  module counter\n  ticks 1\n";

#[test]
fn a_driven_counter_runs_alike_in_both_engines() {
    // ADR 0116 rule 3, `spec/26 §2`: `init`, `step` per tick, the stop at the start of the
    // fourth iteration, then `on_stop`, with the same output and trace in both engines.
    runtime();
    let dir = ita_package("counter", DRIVE, &[("src/counter.una", COUNTER)]);
    let run = una(&dir, &["--json", "run", "--stop-after-iterations", "3"]);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let document: Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["step 1", "step 2", "step 3", "stopped after 3"])
    );
    assert_eq!(document["trace"][3]["outcome"]["kind"], "stop");
    let built = una(&dir, &["build"]);
    assert_eq!(
        built.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una/counter"))
        .current_dir(&dir)
        .args(["--stop-after-iterations", "3", "--trace"])
        .arg(&trace)
        .output()
        .unwrap();
    assert_eq!(
        native.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        "step 1\nstep 2\nstep 3\nstopped after 3\n"
    );
    let native_trace: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    assert_eq!(timeless(native_trace), timeless(document["trace"].clone()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A step that prints its tick and the clock's reading, a tick a day.
const CLOCKED: &str = "module counter

import std.drive (Tick)

pub init() -> Int = 0

pub step(edit count: Int, input: Tick) -> Unit
  uses @console: Console[print]
  uses @clock: Clock[now]
=
  count = count + 1
  let now = @clock.now()
  @console.print(\"step {count} at {input.at} missed {input.missed} now {now}\")
";

const CLOCKED_DRIVE: &str =
    "\nrun\n  grants\n    console\n    clock\n\ndrive\n  module counter\n  ticks 86400000\n";

#[test]
fn under_virtual_time_both_engines_read_the_same_clock_and_never_wait() {
    // ADR 0116 rule 19, `spec/26 §8`: under `--virtual-time` the clock starts at 0 and only the
    // driver's waits move it, so a tick a day runs at once, and the output and the whole trace,
    // the clock's readings included, are identical in both engines.
    runtime();
    let dir = ita_package("clocked", CLOCKED_DRIVE, &[("src/counter.una", CLOCKED)]);
    let started = std::time::Instant::now();
    let run = una(
        &dir,
        &[
            "--json",
            "run",
            "--virtual-time",
            "--stop-after-iterations",
            "3",
        ],
    );
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let document: Value = serde_json::from_slice(&run.stdout).unwrap();
    let expected = [
        "step 1 at 0 missed 0 now 0",
        "step 2 at 86400000 missed 0 now 86400000",
        "step 3 at 172800000 missed 0 now 172800000",
    ];
    assert_eq!(document["output"], serde_json::json!(expected));
    let built = una(&dir, &["build"]);
    assert_eq!(
        built.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una/clocked"))
        .current_dir(&dir)
        .args(["--virtual-time", "--stop-after-iterations", "3", "--trace"])
        .arg(&trace)
        .output()
        .unwrap();
    assert_eq!(
        native.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        format!("{}\n", expected.join("\n"))
    );
    let native_trace: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    // Not `timeless`: virtual readings agree exactly.
    assert_eq!(native_trace, document["trace"]);
    // Two days of ticks took no real time.
    assert!(started.elapsed() < std::time::Duration::from_secs(600));
    let flagged = Command::new(dir.join("target/una/clocked"))
        .current_dir(&dir)
        .arg("--virtual-time=yes")
        .output()
        .unwrap();
    assert_eq!(flagged.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_driven_module_without_its_shapes_or_a_bad_stop_count_is_a_usage_error() {
    // ADR 0116 rules 2 and 19: a usage error, exit code 2, in `una run` and `una build`, and
    // `--stop-after-iterations` from 1 to 2^63 - 1, in `una run` and the executable.
    runtime();
    let dir = ita_package(
        "shapeless",
        "\ndrive\n  module counter\n  ticks 1\n",
        &[(
            "src/counter.una",
            "module counter\n\npub init() -> Int = 0\n",
        )],
    );
    for args in [&["run", "--stop-after-iterations", "1"][..], &["build"][..]] {
        let output = una(&dir, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("has no `pub step("),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    let dir = ita_package("bounds", DRIVE, &[("src/counter.una", COUNTER)]);
    for count in ["0", "9223372036854775808", "many"] {
        let output = una(&dir, &["run", "--stop-after-iterations", count]);
        assert_eq!(output.status.code(), Some(2), "{count}");
    }
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0));
    for count in ["0", "9223372036854775808", "many"] {
        let output = Command::new(dir.join("target/una/bounds"))
            .current_dir(&dir)
            .args(["--stop-after-iterations", count])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{count}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A driven state of a function type, labeled higher at its result in `step` and `on_stop`
/// than in the result of `init`.
const RAISED_FUNCTION: &str = "module counter

import std.drive (Tick)

pub init() -> fn(Int) -> Int = double

pub step(edit f: fn(Int) -> secret Int, input: Tick) -> Unit
  uses @console: Console[print]
=
  @console.print(\"step\")

pub on_stop(f: fn(Int) -> secret Int) -> Unit
  uses @console: Console[print]
=
  @console.print(\"stopped\")

double(x: Int) -> Int = x * 2
";

/// A driven state refined in `step`, `Pos`, where `init` returns a plain `Int`: nothing proves
/// the hand-off satisfies the refinement, which the division in `step` relies on.
const REFINED_STEP: &str = "module counter

import std.drive (Tick)

pub kind Pos = Int where it > 0

pub init() -> Int = 0

pub step(edit n: Pos, input: Tick) -> Unit
  uses @console: Console[print]
=
  @console.print(\"q {100 / n}\")
";

/// The same refinement written as a range in each signature.
const RANGED: &str = "module counter

import std.drive (Tick)

pub init() -> Int in 1..10 = 1

pub step(edit n: Int in 1..10, input: Tick) -> Unit
  uses @console: Console[print]
=
  @console.print(\"q {100 / n}\")
";

#[test]
fn a_driven_state_is_one_type_with_labels_aside_and_no_refinement() {
    // `spec/26 §2` rule 2, issue #1644: the labels of the state are the checker's, so a
    // function-typed state labeled higher in `step` runs in both engines, as `una check`
    // accepts it. A refinement in the state is U7008 in `una check`, on one side only or equal
    // on both, so `una run` and `una build` stop at the check and write no executable; the
    // run-time comparison of refinements behind it is tested directly in `runtime.rs`.
    runtime();
    let dir = ita_package("raisedfn", DRIVE, &[("src/counter.una", RAISED_FUNCTION)]);
    let output = "step\nstep\nstopped\n";
    let run = una(&dir, &["run", "--stop-after-iterations", "2"]);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), output);
    let built = una(&dir, &["build"]);
    assert_eq!(
        built.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let native = Command::new(dir.join("target/una/raisedfn"))
        .current_dir(&dir)
        .args(["--stop-after-iterations", "2"])
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&native.stdout), output);
    let _ = std::fs::remove_dir_all(&dir);
    for (name, source) in [("refined", REFINED_STEP), ("ranged", RANGED)] {
        let dir = ita_package(name, DRIVE, &[("src/counter.una", source)]);
        let checked = una(&dir, &["check"]);
        assert_eq!(checked.status.code(), Some(1), "{name}");
        assert!(
            String::from_utf8_lossy(&checked.stdout).contains("U7008")
                || String::from_utf8_lossy(&checked.stderr).contains("U7008"),
            "{name}: {}",
            String::from_utf8_lossy(&checked.stderr)
        );
        for args in [&["run", "--stop-after-iterations", "1"][..], &["build"][..]] {
            let output = una(&dir, args);
            assert_eq!(output.status.code(), Some(1), "{name} {args:?}");
            assert!(output.stdout.is_empty(), "{name} {args:?}");
        }
        assert!(!dir.join("target/una").join(name).exists(), "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The `run` section of a `for ever` program whose entry is `name`, granted the console.
fn ever_run(name: &str) -> String {
    format!("\nrun\n  entry {name}\n  grants\n    console\n")
}

/// Two `for ever` loops in a row, each with its own budget of `--stop-after-iterations`.
const TWICE: &str = "module twice

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let edit n = 0
  for ever
    @console.print(\"first {n}\")
    n = n + 1
  on stop
    @console.print(\"first stopped at {n}\")
  for ever
    @console.print(\"second {n}\")
    n = n + 1
  on stop
    @console.print(\"second stopped at {n}\")
";

/// A `for ever` that says `started` once, then counts without printing until a host stop.
const QUIET: &str = "module quiet

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let edit n = 0
  for ever
    if n == 0
      @console.print(\"started\")
    n = n + 1
  on stop
    @console.print(\"stopped\")
  @console.print(\"after\")
";

/// A `for ever` whose `on stop` counts far past any grace period.
const SLOW_STOP: &str = "module slow

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let edit n = 0
  for ever
    if n == 0
      @console.print(\"started\")
    n = n + 1
  on stop
    for i in 0..<1000000000000
      n = n + 1
  @console.print(\"after\")
";

/// A program that does not say `diverges`, so its run keeps the default action of SIGTERM.
const PLAIN: &str = "module plain

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print(\"started\")
  let edit n = 0
  for i in 0..<1000000000000
    n = n + 1
  @console.print(\"after\")
";

/// Starts `command` with stdout and stderr piped, waits for its first line on stdout, then
/// sends it SIGTERM `signals` times, `gap` apart, and returns its exit status with all its
/// stdout, the first line included, and its stderr.
fn signalled(
    mut command: Command,
    signals: usize,
    gap: std::time::Duration,
) -> (std::process::ExitStatus, String, String) {
    use std::io::{BufRead, Read};
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut stdout = String::new();
    out.read_line(&mut stdout).unwrap();
    assert_eq!(stdout, "started\n");
    for n in 0..signals {
        if n > 0 {
            std::thread::sleep(gap);
        }
        let killed = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
    }
    out.read_to_string(&mut stdout).unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (child.wait().unwrap(), stdout, stderr)
}

/// `una run`, and the executable `una build` writes, of the package in `dir`, as commands.
fn engines(dir: &Path, name: &str, args: &[&str]) -> [Command; 2] {
    runtime();
    let mut run = Command::new(env!("CARGO_BIN_EXE_una"));
    run.env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .arg("run")
        .args(args);
    let built = una(dir, &["build"]);
    assert_eq!(
        built.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut native = Command::new(dir.join("target/una").join(name));
    native.current_dir(dir).args(args);
    [run, native]
}

#[test]
fn stop_after_iterations_is_a_budget_of_each_loop_instance_in_both_engines() {
    // ADR 0116 rule 19, `spec/26 §6` rule 1, #1472: each `for ever` counts its own iterations
    // from 0, so the first loop stops at its third iteration start and the second, entered
    // after it, runs its own two iterations; the budget is no sticky host stop. Same output
    // and trace in both engines.
    runtime();
    let dir = ita_package("twice", &ever_run("twice"), &[("src/twice.una", TWICE)]);
    let run = una(&dir, &["--json", "run", "--stop-after-iterations", "2"]);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let document: Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!([
            "first 0",
            "first 1",
            "first stopped at 2",
            "second 2",
            "second 3",
            "second stopped at 4"
        ])
    );
    assert_eq!(document["trace"][2]["outcome"]["kind"], "stop");
    assert_eq!(document["trace"][6]["outcome"]["kind"], "stop");
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0));
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una/twice"))
        .current_dir(&dir)
        .args(["--stop-after-iterations", "2", "--trace"])
        .arg(&trace)
        .output()
        .unwrap();
    assert_eq!(
        native.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        "first 0\nfirst 1\nfirst stopped at 2\nsecond 2\nsecond 3\nsecond stopped at 4\n"
    );
    let native_trace: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    assert_eq!(native_trace, document["trace"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reproducer of #1472 (S-36): a `for ever` under a `secret` branch runs `s + 1`
/// iterations, then a public loop counts and prints its own; `SECRET` stands for `s`.
const SECRET_SPIN: &str = "module leak

spin(s: secret Int) -> Unit
  diverges
=
  if s > 0
    let edit k: secret Int = 0
    for ever
      stop when k >= s
      k = k + 1

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  spin(s)
  let edit n = 0
  for ever
    @console.print(\"tick {n}\")
    n = n + 1
  on stop
    @console.print(\"public loop ran {n} iterations\")
";

#[test]
fn a_loop_under_a_secret_branch_does_not_move_where_a_public_loop_stops() {
    // #1472 (S-36), ADR 0116 rule 19 and IF2: with a budget per loop instance, the public
    // loop runs its 8 iterations whatever the secret, in `una run` and in the executable: 0
    // and 2 and 5 within the budget, 12 past it, which stops the secret loop by its own
    // budget and no later loop. A run-wide count printed `8 - (s + 1)` iterations.
    runtime();
    let mut expected: Vec<String> = (0..8).map(|n| format!("tick {n}")).collect();
    expected.push("public loop ran 8 iterations".to_owned());
    let expected = format!("{}\n", expected.join("\n"));
    for secret in ["0", "2", "5", "12"] {
        let name = format!("leak{secret}");
        let program = SECRET_SPIN.replace("SECRET", secret);
        let dir = ita_package(
            &name,
            "\nrun\n  entry leak\n  grants\n    console\n",
            &[("src/leak.una", &program)],
        );
        let [mut run, native] = engines(&dir, &name, &["--stop-after-iterations", "8"]);
        let interpreted = run.output().unwrap();
        assert_eq!(
            interpreted.status.code(),
            Some(0),
            "{secret}: {interpreted:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&interpreted.stdout),
            expected,
            "{secret}"
        );
        let mut native = native;
        let compiled = native.output().unwrap();
        assert_eq!(compiled.status.code(), Some(0), "{secret}: {compiled:?}");
        assert_eq!(
            String::from_utf8_lossy(&compiled.stdout),
            expected,
            "{secret}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn sigterm_runs_on_stop_writes_the_trace_and_exits_0_in_both_engines() {
    // ADR 0116 rules 14 and 18, `spec/26 §6`: a `for ever` program installs the handlers, so
    // SIGTERM ends the loop at its next iteration start, `on stop` and the code after the loop
    // run, and the run exits 0; the executable writes its trace, which ends with the stop.
    let dir = ita_package("quiet", &ever_run("quiet"), &[("src/quiet.una", QUIET)]);
    let trace = dir.join("trace.json");
    let trace_arg = trace.to_str().unwrap().to_owned();
    let [run, native] = engines(&dir, "quiet", &[]);
    let (status, stdout, stderr) = signalled(run, 1, std::time::Duration::ZERO);
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert_eq!(stdout, "started\nstopped\nafter\n");
    let mut native = native;
    native.args(["--trace", &trace_arg]);
    let (status, stdout, stderr) = signalled(native, 1, std::time::Duration::ZERO);
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert_eq!(stdout, "started\nstopped\nafter\n");
    let events: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    let kinds: Vec<&str> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["outcome"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["ok", "stop", "ok", "ok"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sigterm_stops_a_driven_program_which_runs_on_stop_and_exits_0() {
    // ADR 0116 rule 3, `spec/26 §2` rule 4: a host stop interrupts the driver's wait for a
    // tick, the loop stops at its next iteration start, `on_stop` runs, and the run exits 0.
    let program = COUNTER.replace("\"step {count}\"", "\"started\"");
    let program = program.replace(
        "  count = count + 1\n  @console.print(\"started\")",
        "  count = count + 1\n  if count == 1\n    @console.print(\"started\")",
    );
    let dir = ita_package(
        "driven",
        &DRIVE.replace("ticks 1", "ticks 50"),
        &[("src/counter.una", &program)],
    );
    for command in engines(&dir, "driven", &[]) {
        let (status, stdout, stderr) = signalled(command, 1, std::time::Duration::ZERO);
        assert_eq!(status.code(), Some(0), "{stderr}");
        assert!(stdout.starts_with("started\nstopped after "), "{stdout}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_sigterm_brings_the_hard_stop_with_u6007_in_both_engines() {
    // ADR 0116 rule 16, `spec/26 §6` rule 6: a second request during a long `on stop` cancels
    // the run at its next check point with U6007, exit code 1.
    let dir = ita_package("slow", &ever_run("slow"), &[("src/slow.una", SLOW_STOP)]);
    for command in engines(&dir, "slow", &["--stop-grace", "60"]) {
        let (status, stdout, stderr) = signalled(command, 2, std::time::Duration::from_millis(500));
        assert_eq!(status.code(), Some(1), "{stderr}");
        assert_eq!(stdout, "started\n");
        assert!(stderr.contains("U6007"), "{stderr}");
        assert!(
            stderr.contains(
                "the host stopped the run before it finished, at a second host stop request"
            ),
            "{stderr}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_deadline_and_its_grace_period_end_in_u6007_or_a_clean_stop_in_both_engines() {
    // ADR 0116 rules 14 and 16, `spec/26 §6`: `--deadline 1` is a host stop request; an
    // `on stop` that outlasts `--stop-grace 1` is cancelled with U6007, exit code 1, and a short
    // one ends the run normally.
    let dir = ita_package("slow", &ever_run("slow"), &[("src/slow.una", SLOW_STOP)]);
    let trace = dir.join("trace.json");
    for (n, mut command) in engines(&dir, "slow", &["--deadline", "1", "--stop-grace", "1"])
        .into_iter()
        .enumerate()
    {
        if n == 1 {
            command.arg("--trace").arg(&trace);
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "started\n");
        assert!(
            stderr.contains(
                "the host stopped the run before it finished, after the grace period of 1 second"
            ),
            "{stderr}"
        );
    }
    // The trace of an abort: the stop event, then nothing of `on stop`.
    let events: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    assert_eq!(events[1]["outcome"]["kind"], "stop");
    let _ = std::fs::remove_dir_all(&dir);
    let dir = ita_package("quick", &ever_run("quiet"), &[("src/quiet.una", QUIET)]);
    for mut command in engines(&dir, "quick", &["--deadline", "1", "--stop-grace", "1"]) {
        let output = command.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "started\nstopped\nafter\n"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_program_that_cannot_reach_for_ever_keeps_the_default_action_of_sigterm() {
    // ADR 0116 rule 18: no handlers without `diverges` or a driver, so SIGTERM ends the run
    // by its default action, in both engines.
    use std::os::unix::process::ExitStatusExt;
    let dir = ita_package("plain", &ever_run("plain"), &[("src/plain.una", PLAIN)]);
    for command in engines(&dir, "plain", &[]) {
        let (status, stdout, _) = signalled(command, 1, std::time::Duration::ZERO);
        assert_eq!(status.signal(), Some(15), "{status:?}");
        assert_eq!(stdout, "started\n");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deadlines_and_grace_periods_out_of_range_are_usage_errors() {
    // ADR 0116 Diagnostics, `spec/26 §6` rule 2: `--deadline` from 1 to 31536000 and
    // `--stop-grace` from 1 to 86400, in `una run` and the executable.
    let dir = ita_package("ranges", &ever_run("quiet"), &[("src/quiet.una", QUIET)]);
    let [_, _] = engines(&dir, "ranges", &[]);
    for args in [
        ["--deadline", "0"],
        ["--deadline", "31536001"],
        ["--deadline", "soon"],
        ["--stop-grace", "0"],
        ["--stop-grace", "86401"],
        ["--stop-grace", "-1"],
    ] {
        let run = una(&dir, &["run", args[0], args[1]]);
        assert_eq!(run.status.code(), Some(2), "{args:?}");
        let native = Command::new(dir.join("target/una/ranges"))
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(2), "{args:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_host_stop_interrupts_a_waiting_request_in_a_for_ever_body_in_both_engines() {
    // ADR 0116 rules 12 and 14, `spec/26 §6` rule 4: a request that waits on a server that
    // never answers fails with `Interrupted` at SIGTERM, the trace records the failure, the
    // next iteration start stops, and the run exits 0, long before the request's deadline.
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let program = format!(
        "module poll

pub main() -> Unit
  uses @files: Files[write]
  uses @http: Http[get]
  diverges
=
  for ever
    let reply = attempt @http.get(\"http://127.0.0.1:{port}/\", headers: [], key_name: None)
    let shown: internal Text = when reply
      is Ok(response) -> \"ok\"
      is Err(e) -> \"{{e}}\"
    let w = attempt @files.write(\"v.txt\", content: shown)
    unit
"
    );
    let dir = ita_package(
        "poll",
        &format!(
            "\nrun\n  entry poll\n  grants\n    files \"out/\" write\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
        &[("src/poll.una", &program)],
    );
    std::fs::create_dir_all(dir.join("out")).unwrap();
    let trace = dir.join("trace.json");
    let [mut run, mut native] = engines(&dir, "poll", &["--http-timeout", "600"]);
    run.arg("--json");
    native.arg("--trace").arg(&trace);
    let mut traces = Vec::new();
    for mut command in [run, native] {
        let started = std::time::Instant::now();
        let child = command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // The request has reached the server, which reads it and never answers.
        let (mut connection, _) = listener.accept().unwrap();
        let mut head = [0u8; 16];
        let _ = connection.read(&mut head).unwrap();
        let killed = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(120));
        traces.push(if traces.is_empty() {
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["trace"].clone()
        } else {
            serde_json::from_slice::<Value>(&std::fs::read(&trace).unwrap()).unwrap()
        });
        drop(connection);
        // S-45 (#1552): the trace redacts the request's outcome; the program wrote its variant.
        assert_eq!(
            std::fs::read_to_string(dir.join("out/v.txt")).unwrap(),
            "Interrupted"
        );
        std::fs::remove_file(dir.join("out/v.txt")).unwrap();
    }
    for events in &traces {
        assert_eq!(events[0]["op"], "get", "{events}");
        // S-45 (#1552, `spec/11 §5`): the request's outcome, `internal`, is redacted in a
        // run's trace, so its `Interrupted` is not shown; that the request returned at once
        // shows in the `stop` of the next iteration start and the time bound above.
        assert_eq!(
            events[0]["outcome"],
            serde_json::json!({"kind": "redacted", "level": "internal"}),
            "{events}"
        );
        assert_eq!(events[1]["op"], "write", "{events}");
        assert_eq!(events[2]["outcome"]["kind"], "stop", "{events}");
        assert_eq!(events.as_array().unwrap().len(), 3, "{events}");
    }
    assert_eq!(traces[0], traces[1]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A package of edition 0.4, in `ita.una`, whose `run` entry is `module`, built.
fn within_package(name: &str, module: &str, program: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-native-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry {module}\n  grants\n    console\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/within_runs/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join(format!("src/{module}.una")), program).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}

#[test]
fn a_deadline_cuts_a_long_computation_at_a_check_point_in_both_engines() {
    // `spec/27 §5`, ADR 0117 rule 5: once the deadline passes, the expression is cancelled at
    // its next check point, here an iteration of a loop that would run for minutes, and the
    // `within` fails with its time-out, `Deadline.Expired`, which `if error use` handles. The
    // timing label is `public`, so the result prints. The trace has the `timeout` event, then
    // the print.
    let program = "module busy\n\nimport std.time (millisecond)\n\nspin(n: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  total\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let total = spin(4000000000) within 20 * millisecond\n    if error use -1\n  @console.print(\"total {total}\")\n";
    let dir = within_package("busy", "busy", program);
    let started = std::time::Instant::now();
    let interpreted = una(&dir, &["--json", "run", "."]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["total -1"]),
        "{document}"
    );
    assert_eq!(
        document["trace"],
        serde_json::json!([
            {"seq": 1, "outcome": {"kind": "timeout"}},
            {
                "seq": 2,
                "instance": "host:Console",
                "capability": "Console",
                "op": "print",
                "args": {"line": "\"total -1\""},
                "outcome": {"kind": "ok", "value": "unit"}
            }
        ])
    );
    let exe = dir.join("target/una/busy");
    for workers in ["1", "4"] {
        let trace = dir.join(format!("trace-{workers}.json"));
        let native = Command::new(&exe)
            .args(["--workers", workers, "--trace"])
            .arg(&trace)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(0), "{native:?}");
        assert_eq!(String::from_utf8_lossy(&native.stdout), "total -1\n");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
        assert_eq!(written, document["trace"]);
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "the deadline cut nothing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_deadline_cancels_the_tasks_started_inside_its_expression() {
    // ADR 0117 rule 5 and `spec/27 §5` rule 7: tasks started inside `e` are cancelled with it,
    // and `e` ends once they have stopped; the `within` then fails with its time-out. Every task
    // of the block that had not returned ends with its `cancelled` event, started or not, in
    // task order, so the trace is the interpreter's whatever the number of workers.
    let program = "module tasks\n\nimport std.time (millisecond)\n\nspin(n: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  total\n\npair(n: Int) -> Int\n=\n  together\n    let a = spin(n)\n    let b = spin(n)\n    let c = spin(n)\n  a + b + c\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let total = pair(4000000000) within 20 * millisecond\n    if error use -1\n  @console.print(\"total {total}\")\n";
    let dir = within_package("tasks", "tasks", program);
    let started = std::time::Instant::now();
    let interpreted = una(&dir, &["--json", "run", "."]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["total -1"]),
        "{document}"
    );
    let kinds: Vec<String> = document["trace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            format!(
                "{}:{}",
                event["task"].as_str().unwrap_or(""),
                event["outcome"]["kind"].as_str().unwrap_or("")
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "1:cancelled",
            "2:cancelled",
            "3:cancelled",
            ":timeout",
            ":ok"
        ],
        "{document}"
    );
    let exe = dir.join("target/una/tasks");
    for workers in ["1", "4"] {
        let trace = dir.join(format!("trace-{workers}.json"));
        let native = Command::new(&exe)
            .args(["--workers", workers, "--trace"])
            .arg(&trace)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        assert_eq!(String::from_utf8_lossy(&native.stdout), "total -1\n");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
        assert_eq!(written, document["trace"], "--workers {workers}");
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "the deadline cut nothing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tasks_inside_a_within_have_one_trace_under_virtual_time() {
    // `spec/27 §5` rule 7 and `spec/26 §8`: under `--virtual-time` no deadline passes inside
    // `e`, so a `within` over a block of tasks that print has the same output and trace in both
    // engines, with one worker and with several.
    let program = "module vt\n\nimport std.time (second)\n\nleaf(label: Text, n: Int) -> Int\n  uses @console: Console[print]\n=\n  @console.print(label)\n  n\n\npair(n: Int) -> Int\n  uses @console: Console[print]\n=\n  together\n    let a = leaf(\"a\", n: n)\n    let b = leaf(\"b\", n: n + 1)\n  a + b\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let total = pair(20) within 1 * second\n    if error use -1\n  @console.print(\"total {total}\")\n";
    let dir = within_package("vt", "vt", program);
    let interpreted = una(&dir, &["--json", "run", ".", "--virtual-time"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["a", "b", "total 41"]),
        "{document}"
    );
    for workers in ["1", "4"] {
        let trace = dir.join(format!("trace-{workers}.json"));
        let native = Command::new(dir.join("target/una/vt"))
            .args(["--virtual-time", "--workers", workers, "--trace"])
            .arg(&trace)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        let mut lines: Vec<String> = String::from_utf8_lossy(&native.stdout)
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(lines.pop().as_deref(), Some("total 41"));
        lines.sort();
        assert_eq!(lines, ["a", "b"], "--workers {workers}");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
        assert_eq!(written, document["trace"], "--workers {workers}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_deadline_met_keeps_the_value_under_virtual_time_in_both_engines() {
    // `spec/27 §5` and `spec/26 §8`: under `--virtual-time` nothing but the run's own waits
    // moves the clock, so a computation meets any deadline, and both engines give the same
    // output and trace.
    let program = "module met\n\nimport std.time (millisecond)\n\nspin(n: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  total\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let total = spin(100000) within 1 * millisecond\n    if error use -1\n  @console.print(\"total {total}\")\n";
    let dir = within_package("met", "met", program);
    let interpreted = una(&dir, &["--json", "run", ".", "--virtual-time"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["total 100000"]),
        "{document}"
    );
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una/met"))
        .args(["--virtual-time", "--trace"])
        .arg(&trace)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "total 100000\n");
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
    assert_eq!(written, document["trace"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program that runs `rounds` `within`s, each over an expression that builds a list of a
/// thousand items and then computes for minutes, so its deadline of 1 ms cuts it with the list
/// alive, and prints how many were cut.
fn cuts_program(rounds: u64) -> String {
    format!(
        "module cuts\n\nimport std.time (millisecond)\n\nkeep(items: List[Int], total: Int) -> Int = total\n\ngrow(n: Int) -> Int\n=\n  let edit items = [0]\n  for i in 1..1000\n    items = items.append(item: i)\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  keep(items, total: total)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit cut = 0\n  for round in 1..{rounds}\n    let got = grow(4000000000) within 1 * millisecond\n      if error use -1\n    if got == -1\n      cut = cut + 1\n  @console.print(\"cut {{cut}}\")\n"
    )
}

#[test]
fn ten_thousand_cuts_release_what_they_cut_in_both_engines() {
    // ADR 0117 rule 5 and its amendment of 2026-10-19: a cut leaves each frame of the
    // expression by the path a failure takes, so each frame releases its words. Ten thousand
    // cuts, each abandoning a list of a thousand items, about 16 KiB, run under a 16 MiB budget,
    // which a leak would exceed some ten times over (U6003). The same program with no cut needs
    // between 8 and 16 MiB in a debug build, whatever the number of rounds.
    let dir = within_package("cuts", "cuts", &cuts_program(10_000));
    let interpreted = una(&dir, &["run", ".", "--max-memory", "16"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "cut 10000\n");
    for workers in ["1", "4"] {
        let native = Command::new(dir.join("target/una/cuts"))
            .args(["--max-memory", "16", "--workers", workers])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        assert_eq!(String::from_utf8_lossy(&native.stdout), "cut 10000\n");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_memory_that_cuts_hold_stays_flat_as_they_grow() {
    // The same program at growing numbers of cuts, natively, under one budget that the
    // program with no cut needs anyway (16 MiB in a debug build): memory that grew with the cuts
    // would pass at the smallest number and fail at the largest. Each cut abandons about 16 KiB;
    // at 10,000 cuts a leak of a tenth of it would need another 16 MiB.
    for rounds in [500_u64, 2_000, 10_000] {
        let dir = within_package(&format!("cuts{rounds}"), "cuts", &cuts_program(rounds));
        let native = Command::new(dir.join(format!("target/una/cuts{rounds}")))
            .args(["--max-memory", "16"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(0), "{rounds} cuts: {native:?}");
        assert_eq!(
            String::from_utf8_lossy(&native.stdout),
            format!("cut {rounds}\n")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A program whose races print in their branches, at the top level, plus a race that ties and
/// three levels of `first of` and `together`.
const RACES: &str = "module race\n\npub kind Refusal = | No\n\nsay(line: Text, value: Int) -> Int\n  uses @console: Console[print]\n=\n  @console.print(line)\n  value\n\nrefuse(line: Text) -> Int\n  uses @console: Console[print]\n  fails Refusal\n=\n  @console.print(line)\n  if line == \"\" then 0 else fail No\n\nrace() -> Int\n  uses @console: Console[print]\n  fails Refusal\n=\n  first of\n    refuse(\"a\")\n      if error pass\n    say(\"b\", value: 2)\n    say(\"c\", value: 3)\n\ntie() -> Int\n  uses @console: Console[print]\n=\n  first of\n    say(\"t1\", value: 1)\n    say(\"t2\", value: 2)\n\ndouble(n: Int) -> Int = n * 2\n\nleaf(n: Int) -> restricted Int\n=\n  first of\n    double(n)\n    double(n + 1)\n\nmiddle(n: Int) -> restricted Int\n=\n  together\n    let a = leaf(n)\n    let b = leaf(n + 10)\n  a + b\n\ntop(n: Int) -> restricted Int\n=\n  first of\n    middle(n)\n    middle(n + 100)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let got = race()\n    if error use -1\n  @console.print(\"won {got}\")\n  let tied = tie()\n  @console.print(\"tie {tied}\")\n  let deep = top(1)\n  @console.print(\"deep done\")\n";

#[test]
fn a_race_has_the_same_winner_output_and_trace_in_both_engines_under_virtual_time() {
    // `spec/27 §6`, ADR 0117 rules 6, 10 and 12: under `--virtual-time` both engines run the
    // branches in index order, up to the first that returns, whatever `--workers` is, so they
    // agree on the winner, the output and the trace, also for branches that end at the same
    // virtual time (the lowest index wins), and for three levels of `first of` and `together`.
    // The trace lists the winner's events first, then the other branches', under `f<c>.<i>`.
    let dir = within_package("races", "race", RACES);
    let interpreted = una(&dir, &["--json", "run", ".", "--virtual-time"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["a", "b", "won 2", "t1", "tie 1", "deep done"]),
        "{document}"
    );
    let tasks: Vec<&str> = document["trace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["task"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(tasks, ["f1.2", "f1.1", "", "f2.1", "", ""], "{document}");
    let seqs: Vec<u64> = document["trace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, [1, 2, 3, 4, 5, 6]);
    let exe = dir.join("target/una/races");
    for workers in ["1", "4"] {
        let trace = dir.join(format!("trace-{workers}.json"));
        let native = Command::new(&exe)
            .args(["--virtual-time", "--workers", workers, "--trace"])
            .arg(&trace)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&native.stdout),
            "a\nb\nwon 2\nt1\ntie 1\ndeep done\n"
        );
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
        assert_eq!(written, document["trace"], "--workers {workers}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn races_never_deadlock_with_one_worker_or_several_under_real_time() {
    // ADR 0117 rule 3 and Security: every join, a race's included, parks its task, so one
    // worker runs a race, the `together` inside its branches and the races inside those.
    let dir = within_package("racesreal", "race", RACES);
    let exe = dir.join("target/una/racesreal");
    for workers in ["1", "2", "8"] {
        let native = Command::new(&exe)
            .args(["--workers", workers])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        let output = String::from_utf8_lossy(&native.stdout).into_owned();
        assert!(
            output.ends_with("deep done\n"),
            "--workers {workers}: {output}"
        );
        // The race prints `a` and `b` and maybe `c`, and one of the branches that return wins.
        assert!(
            output.contains("won 2\n") || output.contains("won 3\n"),
            "--workers {workers}: {output}"
        );
        assert!(output.contains("tie 1\n") || output.contains("tie 2\n"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program that runs `rounds` races, each between a branch that computes a little and
/// returns, and one that builds a list of a thousand items and then computes for minutes, so
/// the winner cancels it with the list alive, and prints how many the first branch won.
fn races_program(rounds: u64) -> String {
    format!(
        "module losers\n\nkeep(items: List[Int], total: Int) -> Int = total\n\ngrow(n: Int) -> Int\n=\n  let edit items = [0]\n  for i in 1..1000\n    items = items.append(item: i)\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  keep(items, total: total)\n\nsettle(n: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  1\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit won = 0\n  for round in 1..{rounds}\n    let got = first of\n      settle(1000)\n      grow(4000000000)\n    if got == 1\n      won = won + 1\n  @console.print(\"won {{won}}\")\n"
    )
}

#[test]
fn ten_thousand_races_release_what_their_losers_hold_in_both_engines() {
    // ADR 0117 rule 6 and the amendment of 2026-10-19 on S3: a cancelled branch leaves its
    // frames by the failure path, as a cut does, and its heap ends with its task. Ten thousand
    // races whose losers each hold a list of a thousand items when they are cancelled run
    // under a 16 MiB budget, which a leak would exceed many times over (U6003). The
    // interpreter runs the first branch, which wins, and never starts the second.
    let dir = within_package("losers", "losers", &races_program(10_000));
    let _removed = RemovedAtEnd(dir.clone());
    let interpreted = una(&dir, &["run", ".", "--max-memory", "16"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "won 10000\n");
    // A loser that is never cancelled computes for minutes: each run has a time limit, so a
    // hang fails here instead of holding the test run (#1188).
    for workers in ["1", "2", "4"] {
        let native = output_within(
            Command::new(dir.join("target/una/losers"))
                .args(["--max-memory", "16", "--workers", workers])
                .current_dir(&dir),
            std::time::Duration::from_secs(120),
        );
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        assert_eq!(String::from_utf8_lossy(&native.stdout), "won 10000\n");
    }
}

/// Removes a test's package directory when dropped, after a failed assertion too, so a run that
/// `output_within` killed leaves nothing behind (#1533).
struct RemovedAtEnd(PathBuf);

impl Drop for RemovedAtEnd {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The output of `command`, which must end within `limit`: otherwise it is killed and the test
/// fails, naming the limit.
fn output_within(command: &mut Command, limit: std::time::Duration) -> std::process::Output {
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    let mut late = false;
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > limit {
            let _ = child.kill();
            late = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!late, "still running after {limit:?}, killed: {output:?}");
    output
}

#[test]
fn a_branch_too_deep_for_task_depth_aborts_the_run_in_both_engines() {
    // `spec/27 §3` and `§6`, ADR 0117 rule 2: a branch may make at most `--task-depth` calls
    // below its block, in both engines, and one that goes deeper aborts the run with U6003.
    let program = "module deep\n\ndown(n: Int) -> Int = if n == 0 then 0 else down(n - 1) + 1\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let shallow = first of\n    down(5)\n    down(5)\n  @console.print(\"shallow {shallow}\")\n  let deep = first of\n    down(50)\n    down(50)\n  @console.print(\"deep {deep}\")\n";
    let dir = within_package("taskdepth", "deep", program);
    let interpreted = una(&dir, &["run", ".", "--virtual-time", "--task-depth", "20"]);
    assert_eq!(interpreted.status.code(), Some(1), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "shallow 5\n");
    assert!(
        String::from_utf8_lossy(&interpreted.stderr).contains("U6003"),
        "{interpreted:?}"
    );
    let native = Command::new(dir.join("target/una/taskdepth"))
        .args(["--virtual-time", "--task-depth", "20"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        native.status.code(),
        interpreted.status.code(),
        "{native:?}"
    );
    assert_eq!(String::from_utf8_lossy(&native.stdout), "shallow 5\n");
    assert!(
        String::from_utf8_lossy(&native.stderr).contains("U6003"),
        "{native:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program whose started tasks run longer than one turn of 1024 check points, park the root
/// at the bound of their statement, run `together` inside a task, and install a handler, with
/// the line each prints.
const TASKS: &str = "module work\n\npub capability Log\n  op write(line: Text) -> Unit\n\npub handler quiet() for Log\n  op write(line) = unit\n\nnote(n: Int) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(\"note {n}\")\n\nnoted(n: Int) -> Unit\n=\n  note(n) with @log = quiet()\n\nspin(label: Text) -> Unit\n  uses @console: Console[print]\n=\n  let edit total = 0\n  for round in 1..4\n    for i in 1..400\n      total = total + 1\n    @console.print(\"{label} {round} {total}\")\n\nshout(line: Text, n: Int) -> Int\n  uses @console: Console[print]\n=\n  @console.print(line)\n  n\n\npair(n: Int) -> Unit\n  uses @console: Console[print]\n=\n  together\n    let a = shout(\"pair a\", n: n)\n    let b = shout(\"pair b\", n: n + 1)\n  @console.print(\"pair {a} {b}\")\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  tasks at most 3 at once\n    start spin(\"a\")\n    start spin(\"b\")\n    start pair(7)\n    start noted(3)\n  @console.print(\"done\")\n";

#[test]
fn started_tasks_have_the_same_output_and_trace_in_both_engines_under_virtual_time() {
    // `spec/27 §8`, ADR 0117 rules 11 and 12: under `--virtual-time`, native code follows the
    // interpreter's schedule on one worker, whatever `--workers` says: the root and its started
    // tasks take turns of 1024 check points in one first-in, first-out queue, the bound parks
    // the root, `together` runs on its task's turn, and installations are numbered run-wide.
    let dir = within_package("tasksvirtual", "work", TASKS);
    let interpreted = una(&dir, &["--json", "run", ".", "--virtual-time"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    let output: Vec<String> = document["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        output.last().map(String::as_str),
        Some("done"),
        "{document}"
    );
    // The spinners outlast one turn, so their lines interleave.
    let spun: Vec<&str> = output
        .iter()
        .filter(|line| line.starts_with("a ") || line.starts_with("b "))
        .map(String::as_str)
        .collect();
    assert_eq!(spun.len(), 8, "{output:?}");
    assert_ne!(
        spun,
        [
            "a 1 400", "a 2 800", "a 3 1200", "a 4 1600", "b 1 400", "b 2 800", "b 3 1200",
            "b 4 1600"
        ],
        "the spinners never switched: {output:?}"
    );
    let tasks: BTreeSet<&str> = document["trace"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["task"].as_str())
        .collect();
    assert!(
        tasks.contains("s1.1") && tasks.contains("s1.4"),
        "{document}"
    );
    let expected = format!("{}\n", output.join("\n"));
    let exe = dir.join("target/una/tasksvirtual");
    for workers in ["1", "4"] {
        let trace = dir.join(format!("trace-{workers}.json"));
        let native = Command::new(&exe)
            .args(["--virtual-time", "--workers", workers, "--trace"])
            .arg(&trace)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&native.stdout),
            expected,
            "--workers {workers}"
        );
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
        assert_eq!(written, document["trace"], "--workers {workers}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn started_tasks_run_on_every_worker_count_under_real_time() {
    // ADR 0117 rules 2, 3 and 7: under the real clock, started tasks run on the workers beside
    // the root, which parks at the bound and at the end of the statement; each task's lines
    // keep their order, and the statement ends after them all.
    let dir = within_package("tasksreal", "work", TASKS);
    let exe = dir.join("target/una/tasksreal");
    for workers in ["1", "2", "8"] {
        let native = Command::new(&exe)
            .args(["--workers", workers])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        let output = String::from_utf8_lossy(&native.stdout).into_owned();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.last(), Some(&"done"), "--workers {workers}: {output}");
        assert_eq!(lines.len(), 12, "--workers {workers}: {output}");
        for label in ["a", "b"] {
            let own: Vec<&str> = lines
                .iter()
                .copied()
                .filter(|line| line.starts_with(&format!("{label} ")))
                .collect();
            assert_eq!(
                own,
                [1, 2, 3, 4].map(|round| format!("{label} {round} {}", round * 400)),
                "--workers {workers}: {output}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_started_task_past_task_depth_ends_alone_in_both_engines() {
    // `spec/27 §7`, ADR 0117 rule 7: a started task that runs past `--task-depth` ends alone,
    // with its `aborted` event and one U6003 line on stderr, and the other tasks run on.
    let program = "module dives\n\ndeep(n: Int) -> Int = if n == 0 then 0 else deep(n - 1) + 1\n\ndive(n: Int) -> Unit\n  uses @console: Console[print]\n=\n  let d = deep(n)\n  @console.print(\"dove {d}\")\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  tasks\n    start dive(100000)\n    start dive(3)\n  @console.print(\"dives done\")\n";
    let dir = within_package("dives", "dives", program);
    let interpreted = una(&dir, &["--json", "run", ".", "--virtual-time"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["dove 3", "dives done"]),
        "{document}"
    );
    let line = "[U6003] Error: the started task s1.1 ended alone: this call exceeds the maximum call depth of";
    assert!(
        String::from_utf8_lossy(&interpreted.stderr).contains(line),
        "{interpreted:?}"
    );
    let aborted: Vec<&Value> = document["trace"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["task"] == "s1.1")
        .collect();
    assert_eq!(aborted.len(), 1, "{document}");
    assert_eq!(aborted[0]["outcome"]["kind"], "aborted", "{document}");
    for clock in [&["--virtual-time"][..], &[][..]] {
        let trace = dir.join("trace.json");
        let native = Command::new(dir.join("target/una/dives"))
            .args(clock)
            .arg("--trace")
            .arg(&trace)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(0), "{clock:?}: {native:?}");
        assert_eq!(
            String::from_utf8_lossy(&native.stdout),
            "dove 3\ndives done\n"
        );
        assert!(
            String::from_utf8_lossy(&native.stderr).contains(line),
            "{clock:?}: {native:?}"
        );
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
        if clock.is_empty() {
            let mut kinds: Vec<(String, String)> = written
                .as_array()
                .unwrap()
                .iter()
                .map(|event| {
                    (
                        event["task"].as_str().unwrap_or("").to_owned(),
                        event["outcome"]["kind"].as_str().unwrap_or("").to_owned(),
                    )
                })
                .collect();
            kinds.sort();
            let mut expected: Vec<(String, String)> = document["trace"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| {
                    (
                        event["task"].as_str().unwrap_or("").to_owned(),
                        event["outcome"]["kind"].as_str().unwrap_or("").to_owned(),
                    )
                })
                .collect();
            expected.sort();
            assert_eq!(kinds, expected);
        } else {
            assert_eq!(written, document["trace"]);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_host_stop_ends_the_for_ever_of_a_tasks_block_and_its_tasks_drain_in_both_engines() {
    // `spec/27 §7`, ADR 0117 rule 7 and ADR 0116 rule 14: the host stop reaches the `for ever`
    // of the block, whose `on stop` runs; the statement then waits for the tasks it started,
    // which drain. Under `--virtual-time` both engines print the same lines in the same order.
    let program = "module serve\n\nsay(n: Int) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"job {n}\")\n\nserve() -> Int\n  uses @console: Console[print]\n  diverges\n=\n  let edit served = 0\n  tasks at most 4 at once\n    for ever\n      start say(served)\n      served = served + 1\n    on stop\n      @console.print(\"stopping after {served}\")\n  served\n\npub main() -> Unit\n  uses @console: Console[print]\n  diverges\n=\n  let served = serve()\n  @console.print(\"served {served}\")\n";
    let dir = within_package("drain", "serve", program);
    let expected = "stopping after 3\njob 0\njob 1\njob 2\nserved 3\n";
    let interpreted = una(
        &dir,
        &["run", ".", "--virtual-time", "--stop-after-iterations", "3"],
    );
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), expected);
    let native = Command::new(dir.join("target/una/drain"))
        .args(["--virtual-time", "--stop-after-iterations", "3"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), expected);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program that runs `rounds` `tasks` statements, each of which starts a task that builds a
/// list of a thousand items and then computes for minutes, and then fails once the task has
/// had a turn, which cancels the task with its list alive; it prints how many failed.
fn cancelled_program(rounds: u64) -> String {
    format!(
        "module cancelled\n\npub kind Round = | Over\n\nkeep(items: List[Int], total: Int) -> Int = total\n\ngrow(n: Int) -> Int\n=\n  let edit items = [0]\n  for i in 1..1000\n    items = items.append(item: i)\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  keep(items, total: total)\n\nhold() -> Unit\n=\n  let total = grow(4000000000)\n  unit\n\nspin(n: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  total\n\nover(n: Int) -> Unit\n  fails Round\n=\n  if n > 0 then fail Over else unit\n\nround() -> Unit\n  fails Round\n=\n  tasks\n    start hold()\n    let spun = spin(1100)\n    over(spun)\n      if error pass\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit failed = 0\n  for r in 1..{rounds}\n    let outcome = attempt round()\n    if outcome.is_ok() == false\n      failed = failed + 1\n  @console.print(\"failed {{failed}}\")\n"
    )
}

#[test]
fn freed_slots_in_quarantine_are_not_charged_to_the_budget_in_both_engines() {
    // #1506, ADR 0061: a debug runtime reuses a freed slot only after 65,536 others were freed,
    // so a heap that frees more grows its slot table that much past the most objects it ever
    // held at once, the length of a release runtime's table. The budget does not charge those
    // freed slots beyond the capacity a release table would have: a loop that makes and frees
    // 200,000 lists of two items, at most one alive at a time, runs under 10 MiB in both
    // engines. Without the credit the debug executable needs 12 MiB and stops here with U6003.
    // The table's spare capacity stays charged, and the credit is at most a quarter of the
    // budget.
    let program = "module churn\n\nkeep(items: List[Int], total: Int) -> Int = total + 2\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit total = 0\n  for i in 1..200000\n    let items = [i, i]\n    total = keep(items, total: total)\n  @console.print(\"total {total}\")\n";
    let dir = within_package("churn", "churn", program);
    let interpreted = una(&dir, &["run", ".", "--max-memory", "10"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert_eq!(
        String::from_utf8_lossy(&interpreted.stdout),
        "total 400000\n"
    );
    let native = Command::new(dir.join("target/una/churn"))
        .args(["--max-memory", "10"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "total 400000\n");
    // Review finding 2: under a budget the debug table does not fit, the run stops with U6003,
    // not in the allocator at its hard ceiling: the credit, at most a quarter of the budget,
    // keeps the charge within a quarter budget of the bytes held.
    for budget in ["1", "2", "3", "4"] {
        let small = Command::new(dir.join("target/una/churn"))
            .args(["--max-memory", budget])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            small.status.code(),
            Some(1),
            "--max-memory {budget}: {small:?}"
        );
        assert!(
            String::from_utf8_lossy(&small.stderr).contains("[U6003]"),
            "--max-memory {budget}: {small:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ten_thousand_cancelled_started_tasks_release_what_they_hold_in_both_engines() {
    // `spec/27 §7`, ADR 0117 rule 7 and the amendment of 2026-10-19 on S2: a block that fails
    // cancels the tasks it started, each of which leaves its frames by the failure path and
    // ends with its heap. Ten thousand rounds natively, each cancelling a task that holds a list
    // of a thousand items, run under a 16 MiB budget, which a leak would exceed many times over.
    // The interpreter runs two thousand, about 16 s in a debug build (ten thousand take 80 s),
    // which a leak of the same size would still exceed.
    let interpreted_dir = within_package("cancelledi", "cancelled", &cancelled_program(2_000));
    let interpreted = una(&interpreted_dir, &["run", ".", "--max-memory", "16"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert_eq!(
        String::from_utf8_lossy(&interpreted.stdout),
        "failed 2000\n"
    );
    let _ = std::fs::remove_dir_all(&interpreted_dir);
    let dir = within_package("cancelled", "cancelled", &cancelled_program(10_000));
    let _removed = RemovedAtEnd(dir.clone());
    // One worker is the tight leak guard: it needs 12 MiB. Under load, several workers see an
    // occasional spike of a few MiB in the charged memory, so they run under 24 MiB, which a
    // per-round leak, about 160 MiB over ten thousand rounds, still exceeds many times over.
    // A started task that keeps its worker, so the task that cancels it never runs, computes for minutes: each run has a time
    // limit, so a hang fails here instead of holding the test run (#1533, as #1188 for races).
    for (workers, budget) in [("1", "16"), ("2", "24"), ("4", "24")] {
        let native = output_within(
            Command::new(dir.join("target/una/cancelled"))
                .args(["--max-memory", budget, "--workers", workers])
                .current_dir(&dir),
            std::time::Duration::from_secs(120),
        );
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        assert_eq!(String::from_utf8_lossy(&native.stdout), "failed 10000\n");
    }
}

/// S-39 reproducer 2 (#1507): sixteen public iterations, each calling a function whose loop runs
/// only under a `secret` branch, which only a host stop ends.
const SECRET_COUNT: &str = "module leak

spin(go: secret Bool) -> Unit
  diverges
=
  if go
    let edit k: secret Int = 0
    for ever
      k = k + 1

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  for i in 0..<16
    spin(s > i)
  @console.print(\"done\")
";

/// S-39 reproducer 1 (#1507), without a public loop after it: a loop under a `secret` branch
/// that `stop when` ends before the budget or the budget ends, depending on the secret.
const SECRET_BUDGET: &str = "module leak

spin(s: secret Int) -> Unit
  diverges
=
  if s > 0
    let edit k: secret Int = 0
    for ever
      stop when k >= s
      k = k + 1

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  spin(s)
  @console.print(\"done\")
";

/// S-39 (#1507): loops whose own code runs under a `public` program counter, reached under a
/// `secret` one: by a call, by a call whose callee starts a task, by a `start`, and by a call in
/// method form.
const SECRET_CALLS: &str = "module leak

pub kind Box
  n: Int

turn(b: Box) -> Unit
  diverges
=
  let edit k = b.n
  for ever
    k = k + 1

spin() -> Unit
  diverges
=
  let edit k = 0
  for ever
    k = k + 1

spawn() -> Unit
  diverges
=
  tasks
    start spin()

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  for i in 0..<4
    if s > i
      spin()
    if s > i + 4
      spawn()
    if s > i + 8
      tasks
        start spin()
    if s > i + 12
      let b = Box(n: i)
      b.turn()
  @console.print(\"done\")
";

/// S-39 (#1507): the loops under a `secret` branch are entered after a host stop, which is
/// sticky, so each ends at its first iteration start.
const SECRET_LATE: &str = "module leak

spin(go: secret Bool) -> Unit
  diverges
=
  if go
    let edit k: secret Int = 0
    for ever
      k = k + 1

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  @console.print(\"started\")
  let edit n = 0
  for ever
    n = n + 1
  for i in 0..<16
    spin(s > i)
  @console.print(\"done\")
";

/// The trace of `una --json run` and of the executable, and the output of both, for the
/// program `source` with `SECRET` replaced by `secret`, run with `args`.
fn secret_run(name: &str, source: &str, secret: i64, args: &[&str]) -> (Value, Value, Value) {
    let (output, run, native_output, native) = secret_runs(name, source, secret, args);
    assert_eq!(native_output, output, "{name} {secret}");
    (output, run, native)
}

/// As [`secret_run`], the executable's output apart: that of `una --json run`, its trace,
/// and the executable's output and trace.
fn secret_runs(
    name: &str,
    source: &str,
    secret: i64,
    args: &[&str],
) -> (Value, Value, Value, Value) {
    let package = format!("{name}{secret}");
    let program = source.replace("SECRET", &secret.to_string());
    let dir = ita_package(&package, &ever_run("leak"), &[("src/leak.una", &program)]);
    let mut run_args = vec!["--json", "run"];
    run_args.extend_from_slice(args);
    let run = una(&dir, &run_args);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{name} {secret}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let document: Value = serde_json::from_slice(&run.stdout).unwrap();
    let built = una(&dir, &["build"]);
    assert_eq!(
        built.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una").join(&package))
        .current_dir(&dir)
        .args(args)
        .arg("--trace")
        .arg(&trace)
        .output()
        .unwrap();
    assert_eq!(
        native.status.code(),
        Some(0),
        "{name} {secret}: {}",
        String::from_utf8_lossy(&native.stderr)
    );
    let lines: Vec<Value> = String::from_utf8_lossy(&native.stdout)
        .lines()
        .map(Value::from)
        .collect();
    let native_trace: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (
        document["output"].clone(),
        document["trace"].clone(),
        Value::from(lines),
        native_trace,
    )
}

/// The trace of a run that prints `done` and nothing else.
fn done_only() -> Value {
    serde_json::json!([{
        "seq": 1,
        "instance": "host:Console",
        "capability": "Console",
        "op": "print",
        "args": {"line": "\"done\""},
        "outcome": {"kind": "ok", "value": "unit"}
    }])
}

#[test]
fn s39_a_loop_under_a_secret_branch_that_a_budget_ends_adds_no_event_to_the_trace() {
    // S-39 (#1507), `spec/26 §6` rule 4: the `stop` event is a sink that accepts `public`, so a
    // `for ever` under a `secret` program counter that a host stop ends appends none, and the
    // `seq` of later events does not move. With 16 loops, one stop event per loop entered would
    // spell out the secret in one run; the traces are the same for every secret, in both
    // engines.
    for secret in [0, 3, 9, 16] {
        let (output, run, native) = secret_run(
            "s39count",
            SECRET_COUNT,
            secret,
            &["--stop-after-iterations", "1"],
        );
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, done_only(), "{secret}");
        assert_eq!(native, done_only(), "{secret}");
    }
    // A loop that `stop when` ends before the budget appends nothing either, so whether the
    // secret is below the budget does not show.
    for secret in [0, 2, 5, 12] {
        let (output, run, native) = secret_run(
            "s39budget",
            SECRET_BUDGET,
            secret,
            &["--stop-after-iterations", "8"],
        );
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, done_only(), "{secret}");
        assert_eq!(native, done_only(), "{secret}");
    }
}

#[test]
fn s39_a_public_loop_reached_under_a_secret_branch_adds_no_event_to_the_trace() {
    // S-39 (#1507): a loop whose own code runs under a `public` program counter, reached from a
    // `secret` one by a call, by the task a callee starts, or by a `start`, appends no `stop`
    // event either, in both engines.
    for secret in [0, 1, 5, 9, 12] {
        let (output, run, native) = secret_run(
            "s39calls",
            SECRET_CALLS,
            secret,
            &["--stop-after-iterations", "1"],
        );
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, done_only(), "{secret}");
        assert_eq!(native, done_only(), "{secret}");
    }
}

#[test]
fn s39_loops_under_a_secret_branch_entered_after_a_host_stop_add_no_event_to_the_trace() {
    // S-39 (#1507): after SIGTERM, or the end of `--deadline`, the request is sticky, and each
    // loop under a `secret` branch entered after it ends at its first iteration start, without a
    // `stop` event; the public loop's own event stays. The same trace for every secret, in both
    // engines.
    let expected = |kinds: &Value| {
        let events = kinds.as_array().unwrap();
        let kinds: Vec<&str> = events
            .iter()
            .map(|event| event["outcome"]["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["ok", "stop", "ok"], "{events:?}");
        let seqs: Vec<u64> = events
            .iter()
            .map(|event| event["seq"].as_u64().unwrap())
            .collect();
        assert_eq!(seqs, [1, 2, 3], "{events:?}");
    };
    for secret in [0, 3, 9] {
        let program = SECRET_LATE.replace("SECRET", &secret.to_string());
        let package = format!("s39late{secret}");
        let dir = ita_package(&package, &ever_run("leak"), &[("src/leak.una", &program)]);
        let trace = dir.join("trace.json");
        let trace_arg = trace.to_str().unwrap().to_owned();
        for mut command in engines(&dir, &package, &[]) {
            command.args(["--trace", &trace_arg]);
            let (status, stdout, stderr) = signalled(command, 1, std::time::Duration::ZERO);
            assert_eq!(status.code(), Some(0), "{secret}: {stderr}");
            assert_eq!(stdout, "started\ndone\n", "{secret}");
            expected(&serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap());
        }
        for mut command in engines(&dir, &package, &["--deadline", "1"]) {
            command.args(["--trace", &trace_arg]);
            let output = command.output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{secret}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "started\ndone\n",
                "{secret}"
            );
            expected(&serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// S-39 (#1507): a call with an `edit` argument under a `secret` branch and loops under an
/// `internal` one, then a public loop, whose `stop` event stays.
const SECRET_MIXED: &str = "module leak

bump(edit n: secret Int) -> Unit
  diverges
=
  for ever
    n = n + 1

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  let t: internal Int = SECRET
  let edit k: secret Int = 0
  for i in 0..<4
    if s > i
      bump(edit k)
    if t > i
      let edit m: internal Int = 0
      for ever
        m = m + 1
  let edit n = 0
  for ever
    n = n + 1
  @console.print(\"done\")
";

/// S-39 (#1507): a call under a `secret` branch that a `within` cuts, then a public loop.
const SECRET_CUT: &str = "module leak

import std.time (millisecond)

spin(go: secret Bool) -> Unit
  diverges
=
  if go
    let edit k: secret Int = 0
    for ever
      k = k + 1

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  let r = attempt spin(s > 0) within 50 * millisecond
  let edit n = 0
  for ever
    n = n + 1
  @console.print(\"done\")
";

/// The kinds of the events of `trace`, and whether their `seq` numbers run from 1 without a gap.
fn kinds_in_order(trace: &Value) -> (Vec<String>, bool) {
    let events = trace.as_array().unwrap();
    let kinds = events
        .iter()
        .map(|event| event["outcome"]["kind"].as_str().unwrap().to_owned())
        .collect();
    let gapless = events
        .iter()
        .enumerate()
        .all(|(n, event)| event["seq"].as_u64() == Some(n as u64 + 1));
    (kinds, gapless)
}

#[test]
fn s39_a_public_loop_after_quiet_code_keeps_its_stop_event() {
    // S-39 (#1507): under an `internal` program counter too, and through an `edit` argument, a
    // loop appends no `stop` event; the count of `quiet` code is restored after it, so the
    // public loop after it keeps its own event, in both engines.
    for secret in [0, 2, 4] {
        let (output, run, native) = secret_run(
            "s39mixed",
            SECRET_MIXED,
            secret,
            &["--stop-after-iterations", "1"],
        );
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, native, "{secret}");
        assert_eq!(
            kinds_in_order(&run),
            (vec!["stop".to_owned(), "ok".to_owned()], true),
            "{secret}: {run}"
        );
    }
}

#[test]
fn s39_a_quiet_call_that_a_within_cuts_restores_the_count() {
    // S-39 (#1507): a `within` that cuts a call under a `secret` branch leaves it by the path of a
    // failure, which restores the count of `quiet` code, so the public loop after it keeps its
    // `stop` event, in both engines. The `timeout` event of the cut is the reported timing
    // channel of `spec/27 §5`: the report's `timing` list names the `within`.
    for secret in [0, 3] {
        let (output, run, native) = secret_run("s39cut", SECRET_CUT, secret, &["--deadline", "1"]);
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, native, "{secret}");
        let expected: Vec<String> = if secret == 0 {
            vec!["stop".to_owned(), "ok".to_owned()]
        } else {
            vec!["timeout".to_owned(), "stop".to_owned(), "ok".to_owned()]
        };
        assert_eq!(kinds_in_order(&run), (expected, true), "{secret}: {run}");
    }
}

/// S-39 round 2 (#1507, H1): a `within` with a public deadline, which always cuts its loop, in a
/// function that says `diverges`, called under a `secret` branch.
const SECRET_TIMEOUT_QUIET: &str = "module leak

import std.time (millisecond)

spin() -> Unit
  diverges
=
  let edit k = 0
  for ever
    k = k + 1

wrapper() -> Unit
  diverges
=
  let r = attempt spin() within 20 * millisecond
  noop()

noop() -> Unit = unit

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  for i in 0..<4
    if s > i
      wrapper()
  @console.print(\"done\")
";

/// S-39 round 2 (#1507, H1): a `tasks` statement that starts a task and then fails, in a
/// function that says `diverges`, called under a `secret` branch: no host stop, no deadline.
const SECRET_CANCEL_QUIET: &str = "module leak

pub kind Bad = Boom | Other

spin() -> Unit
  diverges
=
  let edit k = 0
  for ever
    k = k + 1

boom() -> Unit
  fails Bad
=
  fail Boom

run_block() -> Unit
  fails Bad
  diverges
=
  tasks
    start spin()
    boom()
      if error pass

wrapper() -> Unit
  diverges
=
  let r = attempt run_block()
  noop()

noop() -> Unit = unit

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  for i in 0..<4
    if s > i
      wrapper()
  @console.print(\"done\")
";

/// S-39 round 2 (#1507): the same events reached under a `secret` branch without a function
/// that says `diverges`: a `within` written there, a call chain to a `tasks` statement that
/// fails, the same through a function value, called directly and by `apply`, a `first of` in a
/// callee, and a `tasks` statement that `skip` leaves.
const SECRET_EVENTS_QUIET: &str = "module leak

import std.time (millisecond)

pub kind Bad = Boom | Other

spin() -> Unit
  diverges
=
  let edit k = 0
  for ever
    k = k + 1

noop() -> Unit = unit

boom() -> Unit
  fails Bad
=
  fail Boom

run_block() -> Unit
  fails Bad
=
  tasks
    start noop()
    boom()
      if error pass

inner() -> Unit
=
  let r = attempt run_block()
  noop()

middle() -> Unit = inner()

apply(f: fn() -> Unit) -> Unit = f()

double(n: Int) -> Int = n * 2

race(i: Int) -> Int
=
  first of
    double(i)
    double(i)

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  let edit n = 0
  for i in 0..<4
    n = n + 1
    if s > i
      let r = attempt spin() within 20 * millisecond
      middle()
    if s > i + 4
      apply(inner)
      let g = inner
      g()
    if s > i + 8
      let v = race(i)
      tasks
        start noop()
        if i == 1
          skip
  @console.print(\"done {n}\")
";

#[test]
fn s39_timeout_and_cancelled_events_under_a_secret_branch_add_nothing_to_the_trace() {
    // S-39 round 2 (#1507, H1), `spec/26 §6` rule 4: the `timeout` and `cancelled` events that
    // the runtime appends by itself are sinks that accept `public`, as `stop` is. Code under a
    // `secret` program counter, or reached from it, appends none and takes no `seq`, so the
    // traces are the same for every secret, in both engines. Before the fix, `s` gave `s`
    // events.
    for secret in [0, 1, 3, 4] {
        for (name, source) in [
            ("s39timeoutq", SECRET_TIMEOUT_QUIET),
            ("s39cancelq", SECRET_CANCEL_QUIET),
        ] {
            let (output, run, native) = secret_run(name, source, secret, &[]);
            assert_eq!(output, serde_json::json!(["done"]), "{name} {secret}");
            assert_eq!(run, done_only(), "{name} {secret}");
            assert_eq!(native, done_only(), "{name} {secret}");
        }
    }
}

#[test]
fn s39_events_reached_under_a_secret_branch_without_diverges_add_nothing_to_the_trace() {
    // S-39 round 2 (#1507): a `within` written under the branch, a call chain or a function
    // value that reaches a failing `tasks` statement, a `first of` in a callee, and a `tasks`
    // statement that `skip` leaves, whose count of `quiet` code is restored on the way out:
    // the same trace for every secret, in both engines.
    for secret in [0, 3, 6, 13] {
        let (output, run, native) = secret_run("s39events", SECRET_EVENTS_QUIET, secret, &[]);
        assert_eq!(output, serde_json::json!(["done 4"]), "{secret}");
        let mut expected = done_only();
        expected[0]["args"]["line"] = Value::from("\"done 4\"");
        assert_eq!(run, expected, "{secret}");
        assert_eq!(native, expected, "{secret}");
    }
}

/// The functions of the round-4 reproducers (#1507, review round 3, H1): `blk` runs a `tasks`
/// statement that fails and cancels its started task, which appends a `cancelled` event.
const SECRET_BLOCK_FNS: &str = "module leak

pub kind Bad = Boom | Other

pub kind Job
  run: fn() -> Unit

noop() -> Unit = unit

boom() -> Unit
  fails Bad
=
  fail Boom

run_block() -> Unit
  fails Bad
=
  tasks
    start noop()
    boom()
      if error pass

blk() -> Unit
=
  let r = attempt run_block()
  noop()

nothing() -> Unit = unit

when_true(b: Bool) -> Unit
=
  if b
    blk()

pick[T](a: T, b: T, f: fn(Bool) -> Unit) -> Unit = f(a == b)
";

/// S-39 round 4 (#1507, review round 3, H1, `f6c.una`): a function value that is itself
/// `secret`, called at a `public` program counter.
const SECRET_VALUE_CALL: &str = "
pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  let f = if s > 0 then blk else nothing
  f()
  let g = if s > 1 then blk else nothing
  g()
  let h = if s > 2 then blk else nothing
  h()
  @console.print(\"done\")
";

/// S-39 round 4 (#1507, review round 3, H1, `f6k.una`): the same through a kind's field.
const SECRET_FIELD_CALL: &str = "
pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  let a = if s > 0 then Job(run: blk) else Job(run: nothing)
  a.run()
  let b = if s > 1 then Job(run: blk) else Job(run: nothing)
  b.run()
  let c = if s > 2 then Job(run: blk) else Job(run: nothing)
  c.run()
  @console.print(\"done\")
";

/// S-39 round 4 (#1507, review round 3, H1, the decision): a generic function calls a public
/// function value with an argument decided by values of its type parameter, `ℓ*_T`, which
/// raises the call to `pc ⊔ ℓ*_T` (ADR 0067 amendment) at a `public` program counter.
const SECRET_DECIDED_CALL: &str = "
pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  pick(s, b: 0, f: when_true)
  pick(s, b: 1, f: when_true)
  pick(s, b: 2, f: when_true)
  @console.print(\"done\")
";

#[test]
fn s39_calls_of_secret_function_values_add_nothing_to_the_trace() {
    // S-39 round 4 (#1507, review round 3, H1), `spec/26 §6` rule 4 with `spec/07 §3` rule 4: a
    // call of a function value labeled `l_f` stands under `pc ⊔ l_f`, with the decisions of its
    // arguments, so it is quiet at a `public` program counter too, called directly, through a
    // kind's field, or from a generic function under a decision. Whole events are the same for
    // every secret, in both engines. Before the fix, `s` gave up to three `cancelled` events.
    for (name, main) in [
        ("s39valuecall", SECRET_VALUE_CALL),
        ("s39fieldcall", SECRET_FIELD_CALL),
        ("s39decided", SECRET_DECIDED_CALL),
    ] {
        let source = format!("{SECRET_BLOCK_FNS}{main}");
        for secret in [0, 1, 2, 3, 5] {
            let (output, run, native) = secret_run(name, &source, secret, &[]);
            assert_eq!(output, serde_json::json!(["done"]), "{name} {secret}");
            assert_eq!(run, done_only(), "{name} {secret}");
            assert_eq!(native, done_only(), "{name} {secret}");
        }
    }
}

/// S-39 round 4 (#1507, review round 3, H1, `f6.una`): `tasks` statements behind `secret`
/// function values, then a public `tasks` statement whose task prints.
const SECRET_VALUE_PATHS: &str = r#"module leak

pub kind Job
  run: fn() -> Unit

noop() -> Unit = unit

blk() -> Unit
=
  tasks
    start noop()

nothing() -> Unit = unit

worker(n: Int) -> Unit
  uses @console: Console[print]
=
  @console.print("w {n}")

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  let f = if s > 0 then blk else nothing
  f()
  let g = if s > 1 then blk else nothing
  g()
  let a = if s > 2 then Job(run: blk) else Job(run: nothing)
  a.run()
  tasks
    start worker(1)
"#;

#[test]
fn s39_quiet_calls_of_secret_function_values_do_not_move_the_paths_of_later_public_ones() {
    // S-39 round 4 (#1507, review round 3, H1): the `tasks` statements that a call of a `secret`
    // function value runs are quiet, so they are numbered `qs<c>`, and the later public task
    // prints at `s1.1` for every secret, in both engines. Before the fix, it printed at
    // `s<min(s, 3) + 1>.1`.
    let expected = Value::from(vec![worker_print(1, "s1.1", 1)]);
    for secret in [0, 1, 2, 3, 5] {
        let (output, run, native) = secret_run("s39vpaths", SECRET_VALUE_PATHS, secret, &[]);
        assert_eq!(output, serde_json::json!(["w 1"]), "{secret}");
        assert_eq!(run, expected, "{secret}");
        assert_eq!(native, expected, "{secret}");
    }
}

/// S-39 reproducer 1 verbatim (#1507), with S-36's per-loop budget (#1478): a loop under a
/// `secret` branch that `stop when` or the budget ends, then a public loop whose `on stop`
/// prints its count.
const SECRET_BUDGET_THEN_PUBLIC: &str = "module leak

spin(s: secret Int) -> Unit
  diverges
=
  if s > 0
    let edit k: secret Int = 0
    for ever
      stop when k >= s
      k = k + 1

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  spin(s)
  let edit n = 0
  for ever
    n = n + 1
  on stop
    @console.print(\"public loop ran {n}\")
";

#[test]
fn s39_a_budget_ended_secret_loop_then_a_public_loop_show_the_same_output_and_trace() {
    // S-39 with S-36 (#1507, #1478): each loop instance has its own budget, so the public loop
    // runs its 10 iterations whatever the secret loop took, and only the public loop appends a
    // `stop` event: the output and the trace are the same for every secret, in both engines.
    for secret in [0, 3, 7, 20] {
        let (output, run, native) = secret_run(
            "s39budgetpublic",
            SECRET_BUDGET_THEN_PUBLIC,
            secret,
            &["--stop-after-iterations", "10"],
        );
        assert_eq!(
            output,
            serde_json::json!(["public loop ran 10"]),
            "{secret}"
        );
        assert_eq!(run, native, "{secret}");
        assert_eq!(
            kinds_in_order(&run),
            (vec!["stop".to_owned(), "ok".to_owned()], true),
            "{secret}: {run}"
        );
    }
}

/// S-39 round 3 (#1507), review H1 (`path1.una`, extended): quiet `tasks` statements under a
/// `secret` branch, in the root task and in a task started at a `public` program counter,
/// then a public `tasks` statement in each, whose task prints.
const SECRET_TASKS_PATHS: &str = r#"module leak

noop() -> Unit = unit

worker(n: Int) -> Unit
  uses @console: Console[print]
=
  @console.print("w {n}")

inner(s: secret Int) -> Unit
  uses @console: Console[print]
=
  for i in 0..<3
    if s > i
      tasks
        start noop()
  tasks
    start worker(2)

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  for i in 0..<3
    if s > i
      tasks
        start noop()
  tasks
    start worker(1)
  tasks
    start inner(s)
"#;

/// The event of `print("w <n>")` with the path `task` and the number `seq`.
fn worker_print(seq: u64, task: &str, n: u64) -> Value {
    serde_json::json!({
        "seq": seq,
        "task": task,
        "instance": "host:Console",
        "capability": "Console",
        "op": "print",
        "args": {"line": format!("\"w {n}\"")},
        "outcome": {"kind": "ok", "value": "unit"}
    })
}

#[test]
fn s39_quiet_tasks_statements_do_not_move_the_paths_of_later_public_ones() {
    // S-39 round 3 (#1507, review H1): quiet code counts its `tasks` statements apart, so the
    // `task` path of a later public event, `s<c>.<k>`, does not depend on how many statements
    // quiet code ran, in the root task or in a started one. Whole events, the path included,
    // are the same for every secret, in both engines.
    let expected = Value::from(vec![
        worker_print(1, "s1.1", 1),
        worker_print(2, "s2.1.s1.1", 2),
    ]);
    for secret in [0, 1, 2, 3, 7] {
        let (output, run, native) = secret_run("s39tpaths", SECRET_TASKS_PATHS, secret, &[]);
        assert_eq!(output, serde_json::json!(["w 1", "w 2"]), "{secret}");
        assert_eq!(run, expected, "{secret}");
        assert_eq!(native, expected, "{secret}");
    }
}

/// S-39 round 3 (#1507), review H1 (`path3.una` verbatim): a `first of` in a callee called
/// under a `secret` branch, then a public `first of` whose branches print.
const SECRET_RACE_PATHS: &str = r#"module leak

double(n: Int) -> Int = n * 2

noop(n: Int) -> Unit = unit

race(i: Int) -> Int
=
  first of
    double(i)
    double(i)

show(n: Int) -> Int
  uses @console: Console[print]
=
  @console.print("w {n}")
  n

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  for i in 0..<3
    if s > i
      let v = race(i)
      noop(v)
  let r = first of
    show(1)
    show(1)
  noop(r)
"#;

#[test]
fn s39_quiet_first_of_blocks_do_not_move_the_paths_of_later_public_ones() {
    // S-39 round 3 (#1507, review H1): quiet code counts its `first of` blocks apart, so the
    // branches of the public race are `f1.<i>` for every secret. The interpreter's trace is
    // the same whole for every secret. Native code runs the losing branch on to its output
    // (review M1, #1516, an engine difference at a `public` program counter that does not
    // depend on the secret), and the order of its branches is the schedule's, so its
    // events are compared by path: each is a branch of the first race, `f1.1` or `f1.2`.
    for secret in [0, 1, 2, 3, 6] {
        let (output, run, _, native) = secret_runs("s39fpaths", SECRET_RACE_PATHS, secret, &[]);
        assert_eq!(output, serde_json::json!(["w 1"]), "{secret}");
        assert_eq!(
            run,
            Value::from(vec![worker_print(1, "f1.1", 1)]),
            "{secret}"
        );
        let events = native.as_array().unwrap();
        assert!(!events.is_empty(), "{secret}: {native}");
        for event in events {
            let task = event["task"].as_str().unwrap_or_default();
            assert!(
                task == "f1.1" || task == "f1.2",
                "{secret}: a public branch at {task}: {native}"
            );
        }
        assert!(kinds_in_order(&native).1, "{secret}: {native}");
    }
}

/// S-39 round 3 (#1507), review H1 (`exits.una` verbatim): a quiet loop that `skip` and `stop`
/// leave across quiet `tasks` statements, a `within` that cuts a quiet `tasks` statement, and a
/// quiet failing block, then the same constructs in public code.
const SECRET_EXIT_PATHS: &str = r#"module leak

import std.time (millisecond)

pub kind Bad = Boom | Other

noop() -> Unit = unit

spin() -> Unit
  diverges
=
  let edit k = 0
  for ever
    k = k + 1

boom() -> Unit
  fails Bad
=
  fail Boom

run_block() -> Unit
  fails Bad
=
  tasks
    start noop()
    boom()
      if error pass

blocked() -> Unit
  diverges
=
  tasks
    start spin()
    spin()

rb() -> Unit
=
  let q = attempt run_block()
  noop()

loops(s: secret Int) -> Unit
  diverges
=
  for j in 0..<4
    if s > j
      for m in 0..<3
        tasks
          start noop()
          if m == 1
            skip
          if m == 2
            stop
      let r = attempt blocked() within 5 * millisecond
      rb()

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  loops(s)
  let p = attempt run_block()
  let r = attempt blocked() within 5 * millisecond
  spin()
  @console.print("done")
"#;

/// The events of `trace` by the path of the task that wrote them, the root's under `""`, each
/// task's in its own order and without their `seq`: what `spec/27 §8` rule 4 compares of a run
/// whose schedule decides the order across tasks.
fn per_task(trace: &Value) -> std::collections::BTreeMap<String, Vec<Value>> {
    let mut tasks: std::collections::BTreeMap<String, Vec<Value>> =
        std::collections::BTreeMap::new();
    for event in trace.as_array().unwrap() {
        let mut event = event.clone();
        event.as_object_mut().unwrap().remove("seq");
        let task = event["task"].as_str().unwrap_or_default().to_owned();
        tasks.entry(task).or_default().push(event);
    }
    tasks
}

#[test]
fn s39_quiet_blocks_left_by_exits_do_not_move_the_paths_of_later_public_ones() {
    // S-39 round 3 (#1507, review H1): whatever way quiet `tasks` statements end, by `skip`,
    // `stop`, a `within`'s cut or a failure, the public statements after them are `s1` and
    // `s2`: the whole trace, paths included, is the same for every secret, in both engines.
    //
    // Under the real clock the public `tasks` statement of `blocked()` runs its started task
    // `s2.1` on worker 1 beside the root on worker 0, both in a loop that stops after three
    // iterations, so which of their two `stop` events comes first in the native trace is the
    // order of real time across tasks, out of model (`spec/27 §8` rules 1 and 4, #1573): the
    // native trace is compared with the interpreter's task by task, and is gapless. Under
    // `--virtual-time` native code follows the interpreter's schedule, and the whole traces
    // are equal (rule 3). The interpreter, deterministic, gives the same whole trace for every
    // secret under both clocks.
    let mut first: Option<Value> = None;
    let mut first_virtual: Option<Value> = None;
    for secret in [0, 1, 2, 4] {
        let (output, run, native) = secret_run(
            "s39xpaths",
            SECRET_EXIT_PATHS,
            secret,
            &["--stop-after-iterations", "3"],
        );
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(
            per_task(&run),
            per_task(&native),
            "{secret}: {run} {native}"
        );
        assert!(kinds_in_order(&native).1, "{secret}: {native}");
        assert_eq!(run[0]["task"], "s1.1", "{secret}: {run}");
        assert_eq!(run[0]["outcome"]["kind"], "cancelled", "{secret}: {run}");
        assert!(kinds_in_order(&run).1, "{secret}: {run}");
        match &first {
            None => first = Some(run),
            Some(first) => assert_eq!(&run, first, "{secret}"),
        }

        let (output, run, native) = secret_run(
            "s39xpathsvt",
            SECRET_EXIT_PATHS,
            secret,
            &["--stop-after-iterations", "3", "--virtual-time"],
        );
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, native, "{secret} under virtual time");
        assert!(kinds_in_order(&run).1, "{secret}: {run}");
        match &first_virtual {
            None => first_virtual = Some(run),
            Some(first) => assert_eq!(&run, first, "{secret} under virtual time"),
        }
    }
}

/// S-41 (#1514), review round 2 of #1511, H2 (`inst1.una`, `inst2.una`): handler installations
/// under a `secret` branch, through a callee and written directly under the branch, then a
/// public installation whose operation is traced. `INSTALL` is replaced by the installation
/// under the branch.
const SECRET_INSTALLS: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit

handler keep() for Sink
  op put(x) = unit

use_it() -> Unit
  uses @k: Sink[put]
=
  @k.put(1)

quietly() -> Int
=
  let n = 1
  n

inner() -> Unit
=
  let r = quietly() with @k = keep()
  unit

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  for i in 0..<3
    if s > i
INSTALL
  use_it() with @k = keep()
  @console.print("done")
"#;

/// The event of `put(1)` on the instance `instance`, with the number `seq` and the path `task`.
fn sink_put(seq: u64, task: &str, instance: &str) -> Value {
    let mut event = serde_json::json!({
        "seq": seq,
        "instance": instance,
        "capability": "Sink",
        "op": "put",
        "args": {"x": "1"},
        "outcome": {"kind": "ok", "value": "unit"}
    });
    if !task.is_empty() {
        event["task"] = Value::from(task);
    }
    event
}

/// The event of `print("done")` with the number `seq`.
fn done_print(seq: u64) -> Value {
    let mut event = done_only()[0].clone();
    event["seq"] = Value::from(seq);
    event
}

#[test]
fn s41_installations_under_a_secret_branch_do_not_move_later_public_instance_ids() {
    // S-41 (#1514), `spec/26 §6` rule 4: an installation under a raised program counter is
    // quiet and takes no number, so the public installation after it is `with:keep#1` for every
    // secret: whole events are the same, in both engines.
    let expected = Value::from(vec![sink_put(1, "", "with:keep#1"), done_print(2)]);
    for install in [
        "      inner()",
        "      let r = quietly() with @k = keep()\n      unit",
    ] {
        let source = SECRET_INSTALLS.replace("INSTALL", install);
        for secret in [0, 1, 2, 3] {
            let (output, run, native) = secret_run("s41inst", &source, secret, &[]);
            assert_eq!(output, serde_json::json!(["done"]), "{install} {secret}");
            assert_eq!(run, expected, "{install} {secret}");
            assert_eq!(native, expected, "{install} {secret}");
        }
    }
}

/// S-41 (#1514): quiet installations whose operations are traced, in the root task and in a
/// task started at a `public` program counter, then public installations in each.
const SECRET_INSTALL_IDS: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit

handler keep() for Sink
  op put(x) = unit

use_it() -> Unit
  uses @k: Sink[put]
=
  @k.put(1)

quiet_put() -> Unit
=
  use_it() with @k = keep()

worker(s: secret Int) -> Unit
=
  for i in 0..<3
    if s > i
      quiet_put()
  use_it() with @k = keep()

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  for i in 0..<3
    if s > i
      quiet_put()
  tasks
    start worker(s)
  use_it() with @k = keep()
  @console.print("done")
"#;

#[test]
fn s41_quiet_installations_take_no_number_in_the_root_and_in_started_tasks() {
    // S-41 (#1514), `spec/26 §6` rule 4, `spec/27 §8`: quiet code numbers none of its
    // installations, `with:<h>#q`, in the root task and in a started one, whose block leaves
    // those ids as they are at its join and renumbers the public ones. The operations that the
    // quiet instances serve under the branch append no event (#1544): before that fix there was
    // one `put` per quiet installation, `s` in each task. Whole traces are equal for every
    // secret, in both engines.
    let expected = Value::from(vec![
        sink_put(1, "s1.1", "with:keep#1"),
        sink_put(2, "", "with:keep#2"),
        done_print(3),
    ]);
    for secret in [0, 1, 2, 3] {
        let (output, run, native) = secret_run("s41ids", SECRET_INSTALL_IDS, secret, &[]);
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, expected, "{secret}");
        assert_eq!(native, expected, "{secret}");
    }
}

/// S-41 review of #1541, H1 (`p_floor.una`): quiet installations under a `secret` branch whose
/// bodies perform no operation, then, at a `public` program counter, an operation of a
/// capability that accepts `secret`, whose body is quiet only by its effect floor and installs
/// a handler. Since #1544 the operation of that quiet instance appends no event, although it
/// always happens: the safe direction.
const SECRET_INSTALL_FLOOR: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit

capability Box
  op hold(x: Int) -> Unit
    accepts secret

handler keep() for Sink
  op put(x) = unit

use_it() -> Unit
  uses @k: Sink[put]
=
  @k.put(1)

quietly() -> Int
=
  let n = 1
  n

inner() -> Unit
=
  use_it() with @k = keep()

handler box() for Box
  op hold(x) = inner()

hold_one() -> Unit
  uses @b: Box[hold]
=
  @b.hold(1)

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  for i in 0..<3
    if s > i
      let r = quietly() with @k = keep()
      unit
  hold_one() with @b = box()
  @console.print("done")
"#;

/// S-41 review of #1541, H1 (`p_qid.una`): the same quiet installations, then a generic
/// function called with public arguments, whose decision is quiet only by the rule for values
/// of a type parameter, and installs a handler, whose operation appends no event since #1544;
/// then a public installation.
const SECRET_INSTALL_GENERIC: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit

handler keep() for Sink
  op put(x) = unit

use_it() -> Unit
  uses @k: Sink[put]
=
  @k.put(1)

quietly() -> Int
=
  let n = 1
  n

inner() -> Unit
=
  use_it() with @k = keep()

branchy[T](a: T, b: T) -> Unit
=
  if a == b
    inner()

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  for i in 0..<3
    if s > i
      let r = quietly() with @k = keep()
      unit
  branchy(1, b: 1)
  use_it() with @k = keep()
  @console.print("done")
"#;

/// The whole traces of `source` for the secrets 0 to 3 are one, in both engines; returns it.
fn one_trace_for_every_secret(name: &str, source: &str, output: &Value) -> Value {
    let mut first: Option<Value> = None;
    for secret in [0, 1, 2, 3] {
        let (printed, run, native) = secret_run(name, source, secret, &[]);
        assert_eq!(&printed, output, "{name} {secret}");
        assert_eq!(run, native, "{name} {secret}");
        match &first {
            None => first = Some(run),
            Some(first) => assert_eq!(&run, first, "{name} {secret}"),
        }
    }
    first.unwrap_or_default()
}

#[test]
fn s41_quiet_installations_reached_by_the_conservative_rules_show_no_secret() {
    // S-41 review of #1541, H1, `spec/26 §6` rule 4: code is quiet also under an effect floor
    // above `public` and a decision on values of a type parameter, where the occurrence of
    // its events is public. A run-wide ordinal of quiet installations gave the `put` there
    // `with:keep#q#(s+1)`; quiet ids take no number, so whole traces are equal across secrets.
    // Since #1544 the operations of quiet instances append no event, the `put` there included,
    // though it always happens: the safe direction (`spec/26 §6` rule 4, "Core").
    let done = serde_json::json!(["done"]);
    let floor = one_trace_for_every_secret("s41floor", SECRET_INSTALL_FLOOR, &done);
    let ops: Vec<&Value> = floor
        .as_array()
        .into_iter()
        .flatten()
        .map(|event| &event["op"])
        .collect();
    assert_eq!(ops, ["hold", "print"], "{floor}");
    assert_eq!(floor[0]["instance"], "with:box#1", "{floor}");
    let generic = one_trace_for_every_secret("s41qid", SECRET_INSTALL_GENERIC, &done);
    let expected = Value::from(vec![sink_put(1, "", "with:keep#1"), done_print(2)]);
    assert_eq!(generic, expected);
}

#[test]
fn s41_the_conformance_case_has_one_trace_for_every_secret() {
    // S-41 (#1514), review of #1541, L1: the conformance case `installs_under_secret` checks
    // only its output; here its whole trace is compared across secrets, in both engines.
    let path = root().join("tests/conformance/runtime/installs_under_secret/src/installs.una");
    let source = std::fs::read_to_string(&path).unwrap();
    let fixed = "let s: secret Int = 2";
    assert_eq!(source.matches(fixed).count(), 1, "{}", path.display());
    let source = source
        .replace(fixed, "let s: secret Int = SECRET")
        .replace("module installs", "module leak");
    let trace = one_trace_for_every_secret("s41conf", &source, &serde_json::json!(["done 3"]));
    let ids: Vec<&str> = trace
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["op"] == "put")
        .filter_map(|event| event["instance"].as_str())
        .collect();
    // The two `put`s of quiet instances, under an effect floor and a decision on values of a
    // type parameter, append no event since #1544.
    assert_eq!(ids, ["with:keep#1", "with:keep#2"], "{trace}");
}

/// #1544, `spec/26 §6` rule 4: the reproducer of the review of #1543 (`p_local.una`): a
/// function with no effects of its own installs a handler and performs an operation served by
/// it; `main` calls it under a `secret` branch. `INSTALL` is the callee's body: through a
/// function that uses the capability, or the operation written in the `with` itself.
const LOCAL_HANDLER: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit

handler keep() for Sink
  op put(x) = unit

use_it() -> Unit
  uses @k: Sink[put]
=
  @k.put(1)

pinger() -> Unit
=
INSTALL

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  if s > 0
    pinger()
  @console.print("done")
"#;

#[test]
fn l1544_an_operation_of_a_handler_installed_in_a_callee_under_a_secret_branch_adds_no_event() {
    // #1544: the instance that the callee installs is quiet, `with:keep#q`, since the call stands
    // under a raised program counter (S-41); its operation appended an event, so the trace of
    // `s > 0` held a `put` that the trace of `s <= 0` did not, in both engines. Operations of a
    // quiet instance now append no event: whole traces are one for every secret.
    for install in [
        "  use_it() with @k = keep()",
        "  @k.put(1) with @k = keep()",
    ] {
        let source = LOCAL_HANDLER.replace("INSTALL", install);
        for secret in [0, 1, 2, 3, 7] {
            let (output, run, native) = secret_run("l1544", &source, secret, &[]);
            assert_eq!(output, serde_json::json!(["done"]), "{install} {secret}");
            assert_eq!(run, done_only(), "{install} {secret}");
            assert_eq!(native, done_only(), "{install} {secret}");
        }
    }
}

#[test]
fn l1544_the_conformance_case_has_one_trace_for_every_secret() {
    // #1544: the conformance case `local_handler_under_secret` checks only its output; here its
    // whole trace is compared across secrets, in both engines, and is exactly the events of its
    // public installations: a `put` in the started task, then one in the root task.
    let path = root().join("tests/conformance/runtime/local_handler_under_secret/src/local.una");
    let source = std::fs::read_to_string(&path).unwrap();
    let fixed = "let s: secret Int = 2";
    assert_eq!(source.matches(fixed).count(), 1, "{}", path.display());
    let source = source
        .replace(fixed, "let s: secret Int = SECRET")
        .replace("module local", "module leak");
    let trace = one_trace_for_every_secret("l1544conf", &source, &serde_json::json!(["done"]));
    let expected = Value::from(vec![
        sink_put(1, "s1.1", "with:keep#1"),
        sink_put(2, "", "with:keep#2"),
        done_print(3),
    ]);
    assert_eq!(trace, expected);
}

/// S-42 (#1515), review H3 of #1511 round 2 (`op1.una`): an operation of a capability that
/// accepts `secret`, performed under `if COND`, once per `i` the condition holds for.
const SECRET_OPS: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit
    accepts secret

handler keep() for Sink
  op put(x) = unit

feed(s: secret Int) -> Unit
  uses @k: Sink[put]
=
  for i in 0..<4
    if COND
      @k.put(s)

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  feed(s) with @k = keep()
  @console.print("done")
"#;

#[test]
fn s42_operations_under_a_secret_branch_append_no_event() {
    // S-42 (#1515), `spec/26 §6` rule 4, `spec/11 §5`: an operation performed in quiet code
    // appends no event and takes no `seq`. Before, `s` `put` events (their arguments redacted)
    // came before `done`, whose `seq` was `s + 1`: the whole secret in one run.
    let source = SECRET_OPS.replace("COND", "s > i");
    for secret in [0, 1, 3, 4] {
        let (output, run, native) = secret_run("s42ops", &source, secret, &[]);
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, done_only(), "{secret}");
        assert_eq!(native, done_only(), "{secret}");
    }
    // The control: at a `public` program counter the same operations keep their events, their
    // `secret` argument redacted.
    let source = SECRET_OPS.replace("COND", "3 > i");
    let (_, run, native) = secret_run("s42pub", &source, 2, &[]);
    assert_eq!(run, native);
    let events = run.as_array().cloned().unwrap_or_default();
    assert_eq!(events.len(), 4, "{run}");
    for (index, event) in events.iter().take(3).enumerate() {
        assert_eq!(event["seq"], index + 1, "{run}");
        assert_eq!(event["instance"], "with:keep#1", "{run}");
        assert_eq!(event["args"]["x"], "<redacted:secret>", "{run}");
    }
}

#[test]
fn s42_the_conformance_case_has_one_trace_for_every_secret() {
    // S-42 (#1515): the conformance case `operations_under_secret` checks its output; here its
    // whole trace is compared across secrets, in both engines. Only the operations at a
    // `public` program counter are left: the `hold`, and the last `put`; those of code quiet by
    // a `secret` branch, by the effect floor of `hold`'s body or by a decision on values of a
    // type parameter, and those of the started task under its branch, append nothing.
    let path = root().join("tests/conformance/runtime/operations_under_secret/src/operations.una");
    let source = std::fs::read_to_string(&path).unwrap();
    let fixed = "let s: secret Int = 2";
    assert_eq!(source.matches(fixed).count(), 1, "{}", path.display());
    let source = source
        .replace(fixed, "let s: secret Int = SECRET")
        .replace("module operations", "module leak");
    let trace = one_trace_for_every_secret("s42conf", &source, &serde_json::json!(["done 2"]));
    let ops: Vec<(&str, &str)> = trace
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|event| Some((event["instance"].as_str()?, event["op"].as_str()?)))
        .collect();
    assert_eq!(
        ops,
        [
            ("with:box#1", "hold"),
            ("with:keep#3", "put"),
            ("host:Console", "print")
        ],
        "{trace}"
    );
}

/// S-42 review of #1551, L2 (`p_l1.una` verbatim): a `for ever within` iteration whose quiet
/// code, under an `internal` branch, has its `Limit.acquire` cut by the deadline, then performs
/// a public `acquire`.
const QUIET_DEADLINE: &str = r#"module leak

import std.limit (Limit, limiter)
import std.time (millisecond, minute)

maybe(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
=
  if s > 0
    let a = attempt @gate.acquire()
    unit
  else
    unit

step(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
=
  maybe(s)
  let b = attempt @gate.acquire()
  unit

poll(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
  diverges
=
  for ever within 100 * millisecond
    step(s)
  unit

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: internal Int = SECRET
  poll(s) with @gate = limiter(at_most: 1, per: minute, waiting: 1000)
  @console.print("done")
"#;

/// The outcome of each event of `trace`: its kind, or `fail:<variant>`.
fn outcome_kinds(trace: &Value) -> Vec<String> {
    trace
        .as_array()
        .into_iter()
        .flatten()
        .map(|event| {
            let outcome = &event["outcome"];
            match outcome["variant"].as_str() {
                Some(variant) => format!("fail:{variant}"),
                None => outcome["kind"].as_str().unwrap_or("").to_owned(),
            }
        })
        .collect()
}

#[test]
fn s42_quiet_code_does_not_consume_the_iteration_timeout_event() {
    // S-42 (#1515), review L1 of #1511 round 2 and L2 of #1551, `spec/26 §7` rule 4: a failure
    // that the iteration's deadline causes in quiet code neither appends the `timeout` event
    // nor counts as the iteration's first observation of it, so the public `acquire` after it
    // still appends the event, in every iteration, in both engines. Before, quiet code marked it
    // reported and no `timeout` event was left for `s > 0`.
    let args = ["--virtual-time", "--stop-after-iterations", "3"];
    let source = QUIET_DEADLINE;
    // Since S-45 (#1552, `spec/11 §5`) every `acquire` outcome is redacted at `internal`.
    let iteration = ["redacted", "timeout"];
    let mut expected: Vec<&str> = iteration.repeat(3);
    expected.extend(["stop", "ok"]);
    for secret in [1, 3] {
        let (output, run, native) = secret_run("s42l1", source, secret, &args);
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, native, "{secret}");
        assert_eq!(outcome_kinds(&run), expected, "{secret}: {run}");
    }
    // `s = 0`: the public `acquire` of the first iteration takes the token. Its outcome is
    // redacted (S-45, #1552), but its iteration has no `timeout` event: whether it waited at
    // the limit is timing, out of model (`spec/27 §10` rule 5, `spec/26 §6` rule 4).
    let (_, run, native) = secret_run("s42l1", source, 0, &args);
    assert_eq!(run, native);
    assert_eq!(
        outcome_kinds(&run),
        [
            "redacted", "redacted", "timeout", "redacted", "timeout", "stop", "ok"
        ],
        "{run}"
    );
}

/// S-46 (#1563): a hidden `acquire` under an `internal` branch takes the one token, then a
/// public `acquire` follows. What the program decides from whether that call failed stays
/// `internal`: a count it returns, an `if error use` that handles the failure in place, and a
/// value it never prints.
const LIMITED_KEPT: &str = r#"module leak

import std.limit (Limit, Limited, limiter)
import std.time (minute)

maybe(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
=
  if s > 0
    let a = attempt @gate.acquire()
    unit
  else
    unit

taken(got: internal Result[Unit, internal Limited]) -> internal Int
=
  when got
    is Ok(took) -> 1
    is Err(failure) -> 0

step(s: internal Int) -> internal Int
  uses @gate: Limit[acquire]
  uses @console: Console[print]
=
  maybe(s)
  let b = attempt @gate.acquire()
  @gate.acquire()
    if error use unit
  @console.print("asked")
  taken(b)

pub main() -> Unit
  uses @console: Console[print]
=
  let s: internal Int = SECRET
  let count = step(s) with @gate = limiter(at_most: 1, per: minute, waiting: 0)
  let kept: internal Int = count + 1
  @console.print("done")
"#;

/// S-46 (#1563), `lim_branch.una` of the review of #1562 verbatim: it printed "token" when
/// `s = 0` and "none" when `s = 1`.
const LIMITED_BRANCH: &str = r#"module leak

import std.limit (Limit, limiter)
import std.time (minute)

maybe(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
=
  if s > 0
    let a = attempt @gate.acquire()
    unit
  else
    unit

step(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
  uses @console: Console[print]
=
  maybe(s)
  let b = attempt @gate.acquire()
  when b
    is Ok(v) -> @console.print("token")
    is Err(e) -> @console.print("none")

pub main() -> Unit
  uses @console: Console[print]
=
  let s: internal Int = SECRET
  step(s) with @gate = limiter(at_most: 1, per: minute, waiting: 0)
  @console.print("done")
"#;

#[test]
fn s46_limited_is_internal_so_the_output_is_the_same_for_every_secret() {
    // S-46 (#1563), `spec/27 §10` rule 4: `Limited` is labeled `internal`. The program still
    // compiles, and its whole output and trace are the same in both engines for every secret, although the
    // hidden `acquire` takes the token only when `s > 0`. `secret_run` checks the executable's
    // output against the interpreter's.
    // Since S-45 (#1552) the `acquire` outcomes are redacted in the trace, and the hidden
    // `acquire` is quiet (S-42), so the whole trace is the same for every secret too, and the
    // executable's equals the interpreter's.
    let mut outputs = Vec::new();
    let mut traces = Vec::new();
    for secret in [0, 1, 3] {
        let (output, run, native) = secret_run("s46kept", LIMITED_KEPT, secret, &[]);
        assert_eq!(run, native, "{secret}");
        outputs.push(output);
        traces.push(run);
    }
    assert_eq!(outputs[0], serde_json::json!(["asked", "done"]));
    assert!(outputs.iter().all(|o| *o == outputs[0]), "{outputs:?}");
    assert!(traces.iter().all(|t| *t == traces[0]), "{traces:?}");
    // The public `acquire`s are recorded, with their outcomes redacted.
    let acquires = traces[0]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["op"] == "acquire")
        .inspect(|event| assert_eq!(event["outcome"]["kind"], "redacted", "{event}"))
        .count();
    assert_eq!(acquires, 2, "{}", traces[0]);
    // The program that prints whether the public `acquire` failed is rejected: its prints run
    // under the `internal` condition (U4003), in `una check` and in `una build`.
    let branch = LIMITED_BRANCH.replace("SECRET", "1");
    let dir = ita_package("s46branch", &ever_run("leak"), &[("src/leak.una", &branch)]);
    let checked = una(&dir, &["--json", "check"]);
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    let document: Value = serde_json::from_slice(&checked.stdout).unwrap();
    let codes: Vec<&str> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["U4003", "U4003"], "{document}");
    let built = una(&dir, &["build"]);
    assert_ne!(built.status.code(), Some(0), "{built:?}");
}

/// S-40 (#1512), review H2 of #1511 (`taskid3.una` verbatim): a finite task started under a
/// `secret` branch, then a public task that prints, in the same `tasks` statement.
const SECRET_START_INDEX: &str = r#"module leak

spin() -> Unit
  diverges
=
  let edit k = 0
  for ever
    k = k + 1

worker() -> Unit
  uses @console: Console[print]
=
  @console.print("worker")

noop() -> Unit = unit

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  for i in 0..<5
    tasks
      if s > i
        start noop()
      start worker()
  @console.print("done")
"#;

/// S-40 (#1512): the conformance case `tests/conformance/runtime/start_paths_under_secret`,
/// with `SECRET` for its secret: `start`s under a `secret` branch before, between and after
/// public ones of the same statement, one task at a time so that the order of the prints is
/// fixed, in the root task and in a task started at a `public` program counter.
fn secret_start_mixed() -> String {
    let case = root().join("tests/conformance/runtime/start_paths_under_secret/src/starts.una");
    let text = std::fs::read_to_string(case).unwrap();
    let program = text
        .replace("module starts", "module leak")
        .replace("let s: secret Int = 2", "let s: secret Int = SECRET");
    assert!(program.contains("SECRET"), "{program}");
    program
}

/// S-40 (#1512), review H2 of #1511 (`cancel.una` verbatim): a task that never ends, started
/// under a `secret` branch, then a public failure that ends the block and cancels it. With
/// `LABEL` `secret`; with it empty, the public twin, whose tasks keep their `cancelled` events.
const SECRET_START_CANCEL: &str = r#"module leak

pub kind Bad = Boom | Other

spin() -> Unit
  diverges
=
  let edit k = 0
  for ever
    k = k + 1

boom() -> Unit
  fails Bad
=
  fail Boom

run_block(s: LABEL Int, i: Int) -> Unit
  fails Bad
  diverges
=
  tasks
    if s > i
      start spin()
    boom()
      if error pass

noop() -> Unit = unit

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: LABEL Int = SECRET
  for i in 0..<4
    let r = attempt run_block(s, i: i)
    noop()
  @console.print("done")
"#;

/// The event of `print(line)` with the path `task` and the number `seq`.
fn print_event(seq: u64, task: &str, line: &str) -> Value {
    serde_json::json!({
        "seq": seq,
        "task": task,
        "instance": "host:Console",
        "capability": "Console",
        "op": "print",
        "args": {"line": format!("\"{line}\"")},
        "outcome": {"kind": "ok", "value": "unit"}
    })
}

#[test]
fn s40_a_start_under_a_secret_branch_does_not_move_the_paths_of_later_public_tasks() {
    // S-40 (#1512), `spec/27 §7` rule 5: a `start` under a raised program counter is quiet, and
    // its statement does not count it: its task is `s<c>.q`, with no `k`, so the public
    // `worker` is `s<c>.1` for every secret, where it was `s<c>.2` in the first `s`
    // statements, in both engines. The same index places it (`spec/27 §1` rule 2), so its
    // worker does not depend on the secret either.
    let mut expected: Vec<Value> = (1..=5)
        .map(|c| print_event(c, &format!("s{c}.1"), "worker"))
        .collect();
    let mut done = done_only()[0].clone();
    done["seq"] = Value::from(6);
    expected.push(done);
    let expected = Value::from(expected);
    for secret in [0, 1, 3, 5, 9] {
        let (output, run, native) = secret_run("s40index", SECRET_START_INDEX, secret, &[]);
        assert_eq!(
            output,
            serde_json::json!(["worker", "worker", "worker", "worker", "worker", "done"]),
            "{secret}"
        );
        assert_eq!(run, expected, "{secret}");
        assert_eq!(native, expected, "{secret}");
    }
}

#[test]
fn s40_starts_under_a_secret_branch_between_public_ones_do_not_move_their_paths() {
    // S-40 (#1512): quiet `start`s before, between and after the public ones of a statement,
    // none at all for secret 0, and in a task started at a `public` program counter: whole
    // events, paths included, are the same for every secret, in both engines.
    let expected = Value::from(vec![
        print_event(1, "s1.1", "w 1"),
        print_event(2, "s1.2", "w 2"),
        print_event(3, "s2.1.s1.1", "w 3"),
    ]);
    for secret in [0, 1, 2, 3, 6] {
        let (output, run, native) = secret_run("s40mixed", &secret_start_mixed(), secret, &[]);
        assert_eq!(output, serde_json::json!(["w 1", "w 2", "w 3"]), "{secret}");
        assert_eq!(run, expected, "{secret}");
        assert_eq!(native, expected, "{secret}");
    }
}

#[test]
fn s40_tasks_started_under_a_secret_branch_end_without_their_cancelled_events() {
    // S-40 (#1512), `spec/26 §6` rule 4: a task started under a raised program counter runs
    // under it, so when a public failure ends its block and cancels it, it appends no
    // `cancelled` event and takes no `seq`. Before the fix, secret `s` gave `s` events.
    let program = SECRET_START_CANCEL.replace("LABEL", "secret");
    for secret in [0, 1, 2, 4, 7] {
        let (output, run, native) = secret_run("s40cancel", &program, secret, &[]);
        assert_eq!(output, serde_json::json!(["done"]), "{secret}");
        assert_eq!(run, done_only(), "{secret}");
        assert_eq!(native, done_only(), "{secret}");
    }
}

#[test]
fn s40_tasks_started_under_a_public_branch_keep_their_cancelled_events_and_paths() {
    // S-40 (#1512), the public twin: a `start` under a `public` program counter is not quiet, so
    // each cancelled task keeps its `cancelled` event and its path `s<c>.<k>`, as before.
    let program = SECRET_START_CANCEL.replace("LABEL ", "");
    for count in [0_usize, 2, 4] {
        let secret = i64::try_from(count).unwrap();
        let (output, run, native) = secret_run("s40cancelpub", &program, secret, &[]);
        assert_eq!(output, serde_json::json!(["done"]), "{count}");
        assert_eq!(run, native, "{count}");
        let events = run.as_array().unwrap();
        assert_eq!(events.len(), count + 1, "{count}: {run}");
        for (c, event) in events.iter().take(count).enumerate() {
            assert_eq!(event["seq"], Value::from(c + 1), "{count}: {run}");
            assert_eq!(event["task"], format!("s{}.1", c + 1), "{count}: {run}");
            assert_eq!(event["outcome"]["kind"], "cancelled", "{count}: {run}");
        }
    }
}

#[test]
fn s40_a_start_under_a_secret_branch_still_may_not_print() {
    // S-40 (#1512): marking the `start` quiet weakens no check. Its task runs under the
    // program counter of the `start` (`spec/27 §7` rule 3), so an operation there is still
    // refused by the effect bound, U4002.
    let program = SECRET_START_INDEX
        .replace("SECRET", "3")
        .replace("start noop()", "start worker()");
    let dir = ita_package("s40print", &ever_run("leak"), &[("src/leak.una", &program)]);
    let output = una(&dir, &["check"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("U4002"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// S-40 round 2 (#1512, review H1, `p_generic.una` verbatim): a decision on values of a type
/// parameter makes the second `start` quiet with no `secret` branch around it, so its task's
/// `put` occurs whatever the secret; before, its path counted the quiet `start`s that ran before
/// it under the `secret` branch (`s1.q1` for 0, `s1.q2` for 2).
const SECRET_START_GENERIC: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit

handler keep() for Sink
  op put(x) = unit

noop() -> Unit = unit

use_it() -> Unit
  uses @k: Sink[put]
=
  @k.put(1)

pinger() -> Unit
=
  use_it() with @k = keep()

block[T](a: T, b: T, s: secret Int, i: Int) -> Unit
  diverges
=
  tasks
    if s > i
      start noop()
    if a == b
      start pinger()

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  for i in 0..<3
    block(1, b: 1, s: s, i: i)
  @console.print("done")
"#;

/// S-40 round 2 (#1512, review H1, after `p_floor.una`): a `tasks` statement in the body of
/// an operation of a capability that accepts `secret` is quiet by the effect floor, and its last
/// task's `put` occurs whatever the secret; before, its path counted the quiet `start`s under
/// the `secret` branches before it (`qs1.q1` for 0, `qs1.q3` for 2). The `put` is served by an
/// instance that public code installed, so that it keeps its event (#1544), and its capability
/// accepts `secret`, as an effect under that floor must.
const SECRET_START_FLOOR: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit
    accepts secret

capability Box
  op hold(x: Int) -> Unit
    accepts secret

handler keep() for Sink
  op put(x) = unit

noop() -> Unit = unit

use_it() -> Unit
  uses @k: Sink[put]
=
  @k.put(1)

block(x: secret Int) -> Unit
  uses @k: Sink[put]
=
  tasks
    if x > 0
      start noop()
    if x > 1
      start noop()
    start use_it()

handler box() for Box
  uses @k: Sink[put]
  op hold(x) = block(x)

hold_it(s: secret Int) -> Unit
  uses @b: Box[hold]
=
  @b.hold(s)

boxed(s: secret Int) -> Unit
  uses @k: Sink[put]
=
  hold_it(s) with @b = box()

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  boxed(s) with @k = keep()
  @console.print("done")
"#;

/// S-40 round 2 (#1512): as [`SECRET_START_FLOOR`], with quiet `tasks` statements and `first of`
/// blocks under `secret` branches before the statement whose task's `put` occurs whatever the
/// secret: quiet code counts no statement and no race either.
const SECRET_BLOCKS_FLOOR: &str = r#"module leak

capability Sink
  op put(x: Int) -> Unit
    accepts secret

capability Box
  op hold(x: Int) -> Unit
    accepts secret

handler keep() for Sink
  op put(x) = unit

noop() -> Unit = unit

double(n: Int) -> Int = n * 2

use_it() -> Unit
  uses @k: Sink[put]
=
  @k.put(1)

block(x: secret Int) -> Unit
  uses @k: Sink[put]
=
  for i in 0..<3
    if x > i
      tasks
        start noop()
      let r = first of
        double(i)
        double(i)
      noop()
  let v = first of
    double(1)
    double(2)
  tasks
    start use_it()

handler box() for Box
  uses @k: Sink[put]
  op hold(x) = block(x)

hold_it(s: secret Int) -> Unit
  uses @b: Box[hold]
=
  @b.hold(s)

boxed(s: secret Int) -> Unit
  uses @k: Sink[put]
=
  hold_it(s) with @b = box()

pub main() -> Unit
  uses @console: Console[print]
=
  let s: secret Int = SECRET
  boxed(s) with @k = keep()
  @console.print("done")
"#;

/// The whole traces of `source` for each secret, both engines, after checking that the
/// executable's equals the interpreter's and that each is the same for every secret.
fn same_traces_across(name: &str, source: &str, secrets: &[i64]) -> Value {
    let mut first: Option<(Value, Value, Value)> = None;
    for &secret in secrets {
        let (output, run, native) = secret_run(name, source, secret, &[]);
        assert_eq!(run, native, "{name} {secret}");
        match &first {
            None => first = Some((output, run, native)),
            Some(seen) => assert_eq!(
                (&output, &run, &native),
                (&seen.0, &seen.1, &seen.2),
                "{name} {secret}"
            ),
        }
    }
    first.unwrap().1
}

#[test]
fn s40_quiet_tasks_have_no_number_so_quiet_code_without_a_secret_branch_shows_nothing() {
    // S-40 round 2 (#1512, review H1), `spec/27 §7` rule 5: code quiet by a decision on values
    // of a type parameter, or by the effect floor of an operation body, writes events whatever
    // the secret. Quiet `start`s, statements and races take no number, so those events' paths
    // do not count how many quiet ones ran before under a `secret` branch: whole events are the
    // same for every secret, in both engines.
    // Since #1544 the `put`s that the generic case's quiet tasks perform, on instances they
    // install themselves, append no event, and an effect under a decision on values of a type
    // parameter is U4002: no event of those tasks is left to carry a path. The floor cases
    // perform their `put` on an instance that public code installed, in quiet code (the effect
    // floor of `hold`'s body), so since S-42 (#1515) it appends no event either: only the
    // `hold` and the `print` are left, and no event carries a path.
    let generic = same_traces_across("s40generic", SECRET_START_GENERIC, &[0, 1, 2, 3]);
    assert_eq!(generic, done_only(), "{generic}");
    for (name, source) in [
        ("s40floor", SECRET_START_FLOOR),
        ("s40blocks", SECRET_BLOCKS_FLOOR),
    ] {
        let trace = same_traces_across(name, source, &[0, 1, 2, 3]);
        let ops: Vec<&Value> = trace
            .as_array()
            .into_iter()
            .flatten()
            .map(|event| &event["op"])
            .collect();
        assert_eq!(ops, ["hold", "print"], "{name}: {trace}");
        assert!(
            trace
                .as_array()
                .into_iter()
                .flatten()
                .all(|e| e.get("task").is_none()),
            "{name}: {trace}"
        );
    }
}

/// S-40 (#1512, review M1, `p_bound.una` verbatim): a quiet `start` between two public ones of
/// a statement bounded at 2.
const SECRET_START_BOUND: &str = r#"module leak

worker(n: Int) -> Unit
  uses @console: Console[print]
=
  @console.print("worker {n}")

noop() -> Unit = unit

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: secret Int = SECRET
  tasks at most 2 at once
    start worker(1)
    if s > 0
      start noop()
    start worker(2)
    @console.print("main")
  @console.print("done")
"#;

#[test]
fn s40_open_channel_1545_a_quiet_start_holds_a_slot_of_the_shared_bound() {
    // OPEN CHANNEL, #1545 (`spec/26 §6` rule 4): this test documents a leak, it does not show
    // a fix. Quiet and public tasks share the one bound of ADR 0117 rule 7, so the quiet `start`
    // under `if s > 0` holds one of the two slots, `start worker(2)` waits for it, and the
    // interpreter runs `worker(1)` before `main` prints: its output and trace depend on the
    // secret, deterministically. Separate bounds would close it but let a statement run twice
    // its bound (`docs/plan/0117-quiet-bounds-amendment-draft.md`). When #1545 is fixed, these
    // expectations become one, the same for every secret. What is already closed is checked
    // here too: the paths, and the executable's events task by task, equal the interpreter's.
    let leaked = |secret: i64| {
        if secret == 0 {
            serde_json::json!(["main", "worker 1", "worker 2", "done"])
        } else {
            serde_json::json!(["worker 1", "main", "worker 2", "done"])
        }
    };
    let mut paths: Option<std::collections::BTreeMap<String, Vec<Value>>> = None;
    for secret in [0, 1, 2] {
        let (output, run, native_output, native) =
            secret_runs("s40bound", SECRET_START_BOUND, secret, &[]);
        assert_eq!(output, leaked(secret), "{secret}");
        assert_eq!(by_task(&native), by_task(&run), "{secret}");
        let mut lines: Vec<String> = serde_json::from_value(native_output).unwrap();
        lines.sort();
        assert_eq!(lines, ["done", "main", "worker 1", "worker 2"], "{secret}");
        let tasks = by_task(&run);
        assert_eq!(
            tasks.keys().map(String::as_str).collect::<Vec<_>>(),
            ["", "s1.1", "s1.2"],
            "{secret}: {run}"
        );
        match &paths {
            None => paths = Some(tasks),
            Some(seen) => assert_eq!(&tasks, seen, "{secret}"),
        }
    }
}
