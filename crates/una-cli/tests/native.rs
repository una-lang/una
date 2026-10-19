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

#[path = "common/timing.rs"]
mod timing;

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
        // A case of several modules is the directory above its `src`, described by `una.toml`
        // or, from edition 0.4, by `ita.una` (ADR 0122).
        let path = match case
            .ancestors()
            .find(|a| a.join("una.toml").is_file() || a.join("ita.una").is_file())
        {
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

#[test]
fn i1_i8_a_block_of_a_million_tasks_completes_or_stops_with_u6003() {
    // ADR 0113 rules 1, 2 and 5: a `together for` over 10^6 small integers under `--max-memory
    // 64` reserves its result list exactly, each batch's state and each task's start, so it
    // either completes or stops with U6003 at the block, and never aborts the process.
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
fn map_updates_are_in_place_and_scale_as_n_log_n() {
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
        let (output, seconds) = timing::timed(|| {
            Command::new(dir.join(format!("target/una/maps_{n}")))
                .current_dir(dir)
                .output()
                .unwrap()
        });
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
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "ran\n");
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
fn json_parses_and_writes_in_linear_time_natively() {
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
        let (output, seconds) = timing::timed(|| {
            Command::new(dir.join("target/una").join(format!("json-scaling-{steps}")))
                .current_dir(dir)
                .output()
                .unwrap()
        });
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

/// The `run` section of a `for ever` program whose entry is `name`, granted the console.
fn ever_run(name: &str) -> String {
    format!("\nrun\n  entry {name}\n  grants\n    console\n")
}

/// Two `for ever` loops in a row: a stop that reaches the first stays for the second.
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
fn a_sticky_stop_after_iterations_runs_alike_in_both_engines() {
    // ADR 0116 rules 14 and 19, `spec/26 §6`: the run's own stop arrives once two iterations
    // have started, ends the first loop at the third iteration start, and stays for the second
    // loop, which stops at its first; same output and trace in both engines.
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
            "second stopped at 2"
        ])
    );
    assert_eq!(document["trace"][2]["outcome"]["kind"], "stop");
    assert_eq!(document["trace"][4]["outcome"]["kind"], "stop");
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
        "first 0\nfirst 1\nfirst stopped at 2\nsecond stopped at 2\n"
    );
    let native_trace: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    assert_eq!(native_trace, document["trace"]);
    let _ = std::fs::remove_dir_all(&dir);
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
  uses @http: Http[get]
  diverges
=
  for ever
    let reply = attempt @http.get(\"http://127.0.0.1:{port}/\", headers: [], key_name: None)
    unit
"
    );
    let dir = ita_package(
        "poll",
        &format!("\nrun\n  entry poll\n  grants\n    http \"http://127.0.0.1:{port}\" get\n"),
        &[("src/poll.una", &program)],
    );
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
    }
    for events in &traces {
        assert_eq!(events[0]["op"], "get", "{events}");
        assert_eq!(events[0]["outcome"]["kind"], "fail", "{events}");
        assert_eq!(events[0]["outcome"]["variant"], "Interrupted", "{events}");
        assert_eq!(events[1]["outcome"]["kind"], "stop", "{events}");
        assert_eq!(events.as_array().unwrap().len(), 2, "{events}");
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
    // timing label is `public`, so the result prints. One `timeout` event is in the trace.
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
        serde_json::json!([{"seq": 1, "outcome": {"kind": "timeout"}}])
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
    // ADR 0117 rule 5: tasks started inside `e` are cancelled with it, and `e` ends once they
    // have stopped; the `within` then fails with its time-out. Natively each task sees the
    // `within`'s cancellation at its own check points, with one worker and with several.
    let program = "module tasks\n\nimport std.time (millisecond)\n\nspin(n: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  total\n\npair(n: Int) -> Int\n=\n  together\n    let a = spin(n)\n    let b = spin(n)\n  a + b\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let total = pair(4000000000) within 20 * millisecond\n    if error use -1\n  @console.print(\"total {total}\")\n";
    let dir = within_package("tasks", "tasks", program);
    let started = std::time::Instant::now();
    let interpreted = una(&dir, &["run", "."]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "total -1\n");
    let exe = dir.join("target/una/tasks");
    for workers in ["1", "2"] {
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
        assert_eq!(String::from_utf8_lossy(&native.stdout), "total -1\n");
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "the deadline cut nothing"
    );
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
    // cuts, each abandoning a list of a thousand items, about 16 KiB, run under a 4 MiB budget,
    // which a leak would exceed some forty times over (U6003).
    let dir = within_package("cuts", "cuts", &cuts_program(10_000));
    let interpreted = una(&dir, &["run", ".", "--max-memory", "4"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "cut 10000\n");
    for workers in ["1", "4"] {
        let native = Command::new(dir.join("target/una/cuts"))
            .args(["--max-memory", "4", "--workers", workers])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(0), "--workers {workers}: {native:?}");
        assert_eq!(String::from_utf8_lossy(&native.stdout), "cut 10000\n");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_memory_that_cuts_hold_stays_flat_as_they_grow() {
    // The same program at growing numbers of cuts, natively, under one small budget: memory
    // that grew with the cuts would pass at the smallest number and fail at the largest. Each
    // cut abandons about 16 KiB; at 10,000 cuts a leak of a tenth of it would need 16 MiB.
    for rounds in [500_u64, 2_000, 10_000] {
        let dir = within_package(&format!("cuts{rounds}"), "cuts", &cuts_program(rounds));
        let native = Command::new(dir.join(format!("target/una/cuts{rounds}")))
            .args(["--max-memory", "2"])
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
