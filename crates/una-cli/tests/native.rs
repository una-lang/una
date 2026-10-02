//! The differential test of the native backend (`spec/16 §5`, ADR 0055 NB-4): every runnable
//! program of the repository's suites runs in the interpreter (`una run --json`) and as a
//! native executable (`una build`), and their output, exit status and trace must be identical.
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

fn una(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
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
        if String::from_utf8_lossy(&native.stdout) != expected_stdout {
            failures.push(format!(
                "{name}: stdout differs: interpreter {expected_stdout:?}, native {:?}",
                String::from_utf8_lossy(&native.stdout)
            ));
        }
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
        if native_trace != document["trace"] {
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
fn a_construct_the_backend_does_not_compile_yet_is_named() {
    let dir = package(
        "clock",
        "\n[run]\nentry = \"clock\"\ngrants = [\"clock\"]\n",
        &[(
            "src/clock.una",
            "module clock\n\npub main() -> Unit\n  uses @clock: Clock[now]\n=\n  let t = @clock.now()\n  unit\n",
        )],
    );
    let output = una(&dir, &["build"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: the native backend does not compile `Clock.now` yet (#26)\n"
    );
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
    let _ = std::fs::remove_dir_all(&dir);
}
