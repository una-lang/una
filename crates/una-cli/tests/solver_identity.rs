//! The `solver-identity` gate (ADR 0124 rule 31): over every query of the repository's
//! obligations (the conformance suite's verification cases and the examples), the solver child
//! of `una` and the standalone `z3` that the project built from the same library, recipe and
//! flags must write the same bytes; and, when the Z3 project's own release binary is given,
//! the statuses must agree, a difference being listed for review rather than failing. Each
//! query runs twice in each comparison. The test is `#[ignore]`d: it needs the built library
//! (`UNA_Z3_STANDALONE`, the `bin/z3` of the cache entry) and runs only in the gate:
//! `UNA_Z3_STANDALONE=<cache>/<key>/bin/z3 cargo test --release --locked -p una-cli --test
//! solver_identity -- --ignored --nocapture`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use una_cli::Tier;
use una_verify::Solver;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The scripts of every obligation of `path`, as the verifier sends them.
fn scripts(path: &Path) -> Vec<String> {
    let checked = una_cli::check(path, Tier::Zero).unwrap();
    let layout = una_verify::SourceLayout::Joined;
    let obligations = una_verify::obligations(&checked.program, &checked.sources, layout).unwrap();
    let lock_text = std::fs::read_to_string(repository().join("verification.lock")).unwrap();
    let (lock, problems) = una_project::parse_lock(&lock_text);
    assert!(problems.is_empty(), "{problems:?}");
    let lock = una_verify::Lock {
        text: lock_text,
        solver_version: lock.solver_version,
        step_budget: lock.step_budget,
        random_seed: lock.random_seed,
    };
    una_verify::queries(&obligations, &lock)
}

/// What the standalone `z3 -in -smt2` at `z3` writes for `script`.
fn standalone(z3: &Path, script: &str) -> (String, bool) {
    let mut child = Command::new(z3)
        .args(["-in", "-smt2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.success(),
    )
}

/// The first status word of an answer.
fn status(output: &str) -> &str {
    output.split_whitespace().next().unwrap_or("")
}

/// The packages whose obligations the gate covers: every verification case of the conformance
/// suite that is a package or a single file, and every example.
fn packages() -> Vec<PathBuf> {
    let root = repository();
    let mut all = Vec::new();
    for dir in ["tests/conformance/verification", "examples"] {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        entries.sort();
        all.extend(entries);
    }
    all
}

#[test]
#[ignore = "the solver-identity release gate: needs the built library's standalone z3 (UNA_Z3_STANDALONE)"]
fn the_solver_child_and_the_project_built_z3_write_the_same_bytes() {
    #[allow(clippy::disallowed_methods)]
    let standalone_z3 = PathBuf::from(
        std::env::var_os("UNA_Z3_STANDALONE")
            .expect("UNA_Z3_STANDALONE names the project-built z3"),
    );
    #[allow(clippy::disallowed_methods)]
    let upstream = std::env::var_os("UNA_Z3_UPSTREAM").map(PathBuf::from);
    assert!(
        una_z3::BUILD.linked,
        "the test binary must link the solver (UNA_Z3_LIB)"
    );
    let launcher = una_cli::solver_child::Launcher::new(4096);
    let mut queries = 0;
    let mut differences = Vec::new();
    let mut upstream_differences = Vec::new();
    for package in packages() {
        let Ok(scripts) = std::panic::catch_unwind(|| scripts(&package)) else {
            // A case that does not check at tier 0 has no obligations to compare.
            continue;
        };
        for script in scripts {
            queries += 1;
            for round in 0..2 {
                let raw = launcher
                    .run(&script, Duration::from_secs(120), None)
                    .unwrap();
                let (reference, _) = standalone(&standalone_z3, &script);
                if raw.stdout != reference {
                    differences.push(format!(
                        "{} query {queries} round {round}: child {:?} vs standalone {:?}",
                        package.display(),
                        raw.stdout,
                        reference
                    ));
                }
                if let Some(upstream) = &upstream {
                    let (theirs, _) = standalone(upstream, &script);
                    if status(&raw.stdout) != status(&theirs) {
                        upstream_differences.push(format!(
                            "{} query {queries}: child {} vs upstream {}",
                            package.display(),
                            status(&raw.stdout),
                            status(&theirs)
                        ));
                    }
                }
            }
        }
    }
    println!(
        "solver-identity: {queries} queries, {} front-end differences, {} upstream status differences",
        differences.len(),
        upstream_differences.len()
    );
    for difference in &upstream_differences {
        println!("upstream (for review): {difference}");
    }
    assert!(queries > 0, "no query was compared");
    assert!(differences.is_empty(), "{differences:#?}");
}
