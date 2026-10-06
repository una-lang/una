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
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use serde_json::Value;

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Builds the runtime library next to the `una` binary, once: `cargo test` builds `una-rt`'s
/// static library only when asked.
fn runtime() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "-p", "una-rt"])
            .current_dir(root())
            .status()
            .unwrap();
        assert!(status.success(), "cargo build -p una-rt failed");
    });
}

/// Runs `una` in `dir`, once the runtime library is built: `una build` links it, and cargo
/// builds the test binaries before the staticlib on a cold target directory.
fn una(dir: &Path, args: &[&str]) -> Output {
    runtime();
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

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

/// A runnable program: where it is in the repository, and the grants it runs with.
struct Program {
    path: PathBuf,
    grants: Vec<String>,
}

/// The programs of the suites that may run.
fn candidates() -> Vec<Program> {
    let root = root();
    let mut programs = Vec::new();
    for entry in sorted(&root.join("examples")) {
        programs.push(Program {
            path: entry,
            grants: Vec::new(),
        });
    }
    for case in files(&root.join("tests/conformance"), "una") {
        let text = std::fs::read_to_string(&case).unwrap_or_default();
        let directives: Vec<&str> = text.lines().take_while(|l| l.starts_with("-- ")).collect();
        if !directives
            .iter()
            .any(|d| d.starts_with("-- expect: run-output") || d.starts_with("-- grant: "))
        {
            continue;
        }
        let grants = directives
            .iter()
            .filter_map(|d| d.strip_prefix("-- grant: "))
            .map(str::to_owned)
            .collect();
        // A case of several modules is the directory above its `src`.
        let path = match case.ancestors().find(|a| a.join("una.toml").is_file()) {
            Some(package) if package.starts_with(root.join("tests/conformance")) => {
                package.to_path_buf()
            }
            _ => case,
        };
        programs.push(Program { path, grants });
    }
    for review in sorted(&root.join("docs/reviews")) {
        let mut dirs = vec![review.join("adversarial")];
        dirs.extend(sorted(&review).into_iter().map(|d| d.join("adversarial")));
        for dir in dirs.into_iter().filter(|d| d.is_dir()) {
            for program in sorted(&dir) {
                let text: String = files(&program, "una")
                    .iter()
                    .chain(std::iter::once(&program))
                    .filter_map(|f| std::fs::read_to_string(f).ok())
                    .collect();
                if !text.contains("pub main(") {
                    continue;
                }
                // A single file has no `[run] grants`: it runs with the grants that need no
                // path.
                let grants = if program.is_file() {
                    vec!["console".to_owned(), "clock".to_owned(), "env".to_owned()]
                } else {
                    Vec::new()
                };
                programs.push(Program {
                    path: program,
                    grants,
                });
            }
        }
    }
    programs.sort_by(|a, b| a.path.cmp(&b.path));
    programs.dedup_by(|a, b| a.path == b.path);
    programs
}

fn sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    entries.retain(|e| {
        !e.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    });
    entries.sort();
    entries
}

fn files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    if dir.is_file() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in sorted(dir) {
        if entry.is_dir() {
            out.extend(files(&entry, extension));
        } else if entry.extension().is_some_and(|e| e == extension) {
            out.push(entry);
        }
    }
    out
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

#[test]
fn every_runnable_program_runs_natively_as_in_the_interpreter() {
    runtime();
    let root = root();
    let listed = unsupported();
    let scratch = std::env::temp_dir().join(format!("una-native-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let mut failures = Vec::new();
    let mut compiled = Vec::new();
    let mut not_compiled = Vec::new();
    for (index, program) in candidates().into_iter().enumerate() {
        let name = program
            .path
            .strip_prefix(&root)
            .unwrap()
            .display()
            .to_string();
        let dir = scratch.join(index.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let copied = copy(&program.path, &dir);
        let (work, target) = if copied.is_file() {
            (
                dir.clone(),
                copied.file_name().unwrap().to_string_lossy().into_owned(),
            )
        } else {
            (copied.clone(), ".".to_owned())
        };
        let mut run_args = vec!["--json", "run", target.as_str()];
        for grant in &program.grants {
            run_args.push("--grant");
            run_args.push(grant);
        }
        let run = una(&work, &run_args);
        let Ok(document) = serde_json::from_slice::<Value>(&run.stdout) else {
            continue;
        };
        if document["status"] == "rejected" {
            continue;
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
                not_compiled.push(name.clone());
                if !listed.contains(&name) {
                    failures.push(format!(
                        "{name}: runnable but not compiled, and not listed in tests/native/unsupported.txt: {}",
                        stderr.trim()
                    ));
                }
            } else {
                failures.push(format!("{name}: una build failed: {}", stderr.trim()));
            }
            continue;
        }
        compiled.push(name.clone());
        if listed.contains(&name) {
            failures.push(format!(
                "{name}: compiles now; remove it from tests/native/unsupported.txt"
            ));
        }
        let trace = dir.join("trace.json");
        let mut native = Command::new(&exe);
        native.current_dir(&work).arg("--trace").arg(&trace);
        for grant in &program.grants {
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
            continue;
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
    for name in &listed {
        if !compiled.contains(name) && !not_compiled.contains(name) {
            failures.push(format!(
                "{name}: listed in tests/native/unsupported.txt, but not a runnable program"
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(
        compiled.iter().any(|name| name == "examples/hello"),
        "examples/hello must compile: {failures:#?}"
    );
    assert!(failures.is_empty(), "{failures:#?}");
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
fn an_executable_over_its_memory_budget_is_u6003() {
    // ADR 0061 rule 2: the executable takes `--max-memory` as `una run` does, and the call that
    // finds the run over its budget is U6003.
    let program = "module hog\n\ngrow(t: Text, n: Int) -> Int = if n <= 0 then t.len else grow(\"{t}0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\", n: n - 1)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let size = grow(\"\", n: 9000)\n  @console.print(\"held {size}\")\n";
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
fn updates_of_values_nothing_else_holds_are_in_place() {
    // #30: 200,000 appends in a loop, 200,000 `set`s and 200,000 field edits copy only what
    // another holder still sees: the caller's list, once, and the caller's point, once. The
    // debug runtime reports its copies (no flag; release builds say nothing).
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
        "note: una-rt (debug build): lists and records copied: 2\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
            .args(["run", "."]),
    );
    assert_eq!(interpreted.status.code(), native.status.code());
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
        "error: `network` is not a grant; grants are console, clock, env, keys:<file> and files:<path>:<access>\n"
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
    let program = "module hogs\n\ngrow(t: Text, n: Int) -> Int = if n <= 0 then t.len else grow(\"{t}0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\", n: n - 1)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  together\n    let a = grow(\"\", n: 10)\n    let b = grow(\"\", n: 9000)\n  @console.print(\"held {a} {b}\")\n";
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
