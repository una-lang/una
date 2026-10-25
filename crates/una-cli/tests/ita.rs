//! `una ita lock` and `una fmt` of the project files of edition 0.4 (ADR 0122, `spec/30`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;

const MANIFEST: &str = "ita demo\n  version 0.1.0\n  edition 0.4\n";
const MODULE: &str = "module demo\n\npub one() -> Int = 1\n";

/// A fresh package directory named `name`, with `ita.una` and one module.
fn package(name: &str, manifest: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("ita")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("ita.una"), manifest).unwrap();
    std::fs::write(dir.join("src/demo.una"), MODULE).unwrap();
    dir
}

/// Runs `una` with `args`: the exit code, stdout and stderr.
fn una(args: &[&str]) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = una_cli::run(
        args.iter().copied(),
        &mut stdout,
        &mut stderr,
        false,
        &|_| Ok(None),
        una_cli::Heap::unmeasured(),
    );
    (
        status.code(),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn path(dir: &Path) -> &str {
    dir.to_str().unwrap()
}

#[test]
fn lock_writes_the_default_solver_then_is_up_to_date() {
    let dir = package("writes", MANIFEST);
    let (code, out, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "wrote ita.lock\n", "")
    );
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert_eq!(
        lock,
        format!(
            "-- Written by `una ita`; do not edit.\nlock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"{}\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n",
            una_project::DEFAULT_SOLVER_VERSION
        )
    );
    let (code, out, _) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!((code, out.as_str()), (0, "ita.lock is up to date\n"));
    let (code, out, _) = una(&["una", "ita", "lock", "--check", path(&dir)]);
    assert_eq!((code, out.as_str()), (0, ""));
    // No temporary file is left beside it.
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(names.iter().all(|name| !name.starts_with('.')), "{names:?}");
    // The package then checks at tier 0 with no diagnostic.
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    assert_eq!(checked.diagnostics, []);
}

/// #1160: links planted at the temporary names `una ita lock` used to write are never written
/// through, and a lock that is a link leading outside the package is refused.
#[cfg(unix)]
#[test]
fn lock_never_writes_through_a_planted_link() {
    let dir = package("planted-link", MANIFEST);
    let victim = dir.parent().unwrap().join("planted-link-victim.txt");
    std::fs::write(&victim, "untouched\n").unwrap();
    for name in [".ita.lock.tmp", ".ita.lock.fmt-tmp"] {
        std::os::unix::fs::symlink(&victim, dir.join(name)).unwrap();
    }
    let (code, out, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!((code, out.as_str()), (0, "wrote ita.lock\n"), "{err}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched\n");
    assert!(
        !std::fs::symlink_metadata(dir.join("ita.lock"))
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let outside = dir.parent().unwrap().join("planted-link-outside.lock");
    std::fs::write(&outside, "outside\n").unwrap();
    std::fs::remove_file(dir.join("ita.lock")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("ita.lock")).unwrap();
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_ne!(code, 0);
    assert!(err.contains("outside the package"), "{err}");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside\n");
    let _ = std::fs::remove_file(&victim);
    let _ = std::fs::remove_file(&outside);
}

/// #1160: `una fmt ita.una` never writes through a link planted at its old temporary name.
#[cfg(unix)]
#[test]
fn fmt_of_ita_una_never_writes_through_a_planted_link() {
    let dir = package(
        "fmt-planted-link",
        "ita  demo\n  edition 0.4\n  version   0.1.0\n",
    );
    let victim = dir.parent().unwrap().join("fmt-planted-link-victim.txt");
    std::fs::write(&victim, "untouched\n").unwrap();
    std::os::unix::fs::symlink(&victim, dir.join(".ita.una.fmt-tmp")).unwrap();
    let (code, _, err) = una(&["una", "fmt", path(&dir.join("ita.una"))]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        "ita demo\n  version 0.1.0\n  edition 0.4\n"
    );
    let _ = std::fs::remove_file(&victim);
}

#[test]
fn lock_keeps_the_solver_section_it_replaces() {
    let dir = package("keeps", MANIFEST);
    let pinned = "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 7\n  seed 3\n";
    std::fs::write(dir.join("ita.lock"), pinned).unwrap();
    let (code, out, _) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!((code, out.as_str()), (0, "wrote ita.lock\n"));
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(lock.contains("  steps 7\n  seed 3\n"), "{lock}");
}

#[test]
fn lock_refuses_an_invalid_lock_and_writes_nothing() {
    let dir = package("invalid", MANIFEST);
    std::fs::write(dir.join("ita.lock"), "lock 1\n").unwrap();
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1);
    assert!(err.contains("U7009"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.lock")).unwrap(),
        "lock 1\n"
    );
}

#[test]
fn lock_refuses_needs_until_dependencies_load() {
    let dir = package("needs", &format!("{MANIFEST}\nneeds\n  json 2.0\n"));
    let (code, out, _) = una(&["una", "--json", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["schema"], "una.ita/1");
    assert_eq!(document["command"], "lock");
    assert_eq!(document["written"], serde_json::json!([]));
    assert_eq!(document["diagnostics"][0]["code"], "U7008");
    assert!(!dir.join("ita.lock").exists());
    // The schema comes first.
    assert!(out.starts_with("{\"schema\":\"una.ita/1\""), "{out}");
}

#[test]
fn lock_check_reports_a_missing_lock() {
    let dir = package("check-missing", MANIFEST);
    let (code, _, err) = una(&["una", "ita", "lock", "--check", path(&dir)]);
    assert_eq!(code, 1);
    assert!(err.contains("U7009"), "{err}");
    assert!(!dir.join("ita.lock").exists());
}

#[test]
fn lock_needs_an_ita_una() {
    let dir = package("no-manifest", MANIFEST);
    std::fs::remove_file(dir.join("ita.una")).unwrap();
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 2);
    assert!(err.contains("has no ita.una"), "{err}");
}

#[test]
fn the_binary_invoked_as_ita_is_una_ita() {
    let dir = package("argv0", MANIFEST);
    for program in ["ita", "/usr/local/bin/ita", "ita.exe"] {
        let _ = std::fs::remove_file(dir.join("ita.lock"));
        let (code, out, err) = una(&[program, "lock", path(&dir)]);
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (0, "wrote ita.lock\n", ""),
            "{program}"
        );
    }
    // Another name stays `una`.
    let (code, _, _) = una(&["una-dev", "lock", path(&dir)]);
    assert_eq!(code, 2);
}

#[test]
fn fmt_formats_ita_una_only_when_named() {
    let messy = "ita  demo\n  edition 0.4\n  version   0.1.0\n\n\n\nlabels\n  PII\n";
    let canonical = "ita demo\n  version 0.1.0\n  edition 0.4\n\nlabels\n  PII\n";
    let dir = package("fmt", messy);
    // `una fmt` of the package leaves ita.una alone.
    let (code, _, _) = una(&["una", "fmt", path(&dir)]);
    assert_eq!(code, 0);
    assert_eq!(std::fs::read_to_string(dir.join("ita.una")).unwrap(), messy);
    let manifest = dir.join("ita.una");
    let (code, _, err) = una(&["una", "fmt", "--check", path(&manifest)]);
    assert_eq!(code, 1);
    assert!(err.contains("U7001"), "{err}");
    assert_eq!(std::fs::read_to_string(&manifest).unwrap(), messy);
    let (code, _, err) = una(&["una", "fmt", path(&manifest)]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(std::fs::read_to_string(&manifest).unwrap(), canonical);
    let (code, _, _) = una(&["una", "fmt", "--check", path(&manifest)]);
    assert_eq!(code, 0);
}

#[test]
fn fmt_leaves_an_ita_una_with_grammar_errors() {
    let broken = "ita demo\n  version 0.1.0\n  edition 0.4\n\nforeign\n";
    let dir = package("fmt-broken", broken);
    let manifest = dir.join("ita.una");
    let (code, _, err) = una(&["una", "fmt", path(&manifest)]);
    assert_eq!(code, 1);
    assert!(err.contains("U7008"), "{err}");
    assert_eq!(std::fs::read_to_string(&manifest).unwrap(), broken);
}

const TOML_LOCK: &str = "encoding_version = \"una-smt-1\"\nsolver = \"z3\"\nsolver_version = \"5.1.0\"\nlogic = \"QF_UFLIA\"\nstep_budget = 5000000\nrandom_seed = 0\n";

/// A fresh package of `una.toml` and `verification.lock`, with one module of `source`.
fn toml_package(name: &str, edition: &str, source: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("ita")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!("[package]\nname = \"demo-pkg\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n"),
    )
    .unwrap();
    std::fs::write(dir.join("verification.lock"), TOML_LOCK).unwrap();
    std::fs::write(dir.join("src/demo.una"), source).unwrap();
    dir
}

#[test]
fn migrate_moves_the_project_files_and_the_package_checks() {
    let dir = toml_package("migrate", "0.2", MODULE);
    let (code, out, err) = una(&["una", "migrate", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "wrote ita.una, ita.lock; removed una.toml, verification.lock\nname: demo-pkg -> demo_pkg\nedition 0.2 -> 0.4: 0 diagnostics\n"
    );
    assert!(!dir.join("una.toml").exists());
    assert!(!dir.join("verification.lock").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        "ita demo_pkg\n  version 0.1.0\n  edition 0.4\n"
    );
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    assert_eq!(checked.diagnostics, []);
    // Edition-gated: a package with ita.una is not migrated again.
    let (code, _, err) = una(&["una", "migrate", path(&dir)]);
    assert_eq!(code, 2);
    assert!(err.contains("already has ita.una"), "{err}");
}

#[test]
fn migrate_dry_run_writes_nothing() {
    let dir = toml_package("migrate-dry", "0.1", MODULE);
    let (code, out, _) = una(&["una", "--json", "migrate", "--dry-run", path(&dir)]);
    assert_eq!(code, 0);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(out.starts_with("{\"schema\":\"una.migrate/1\""), "{out}");
    assert_eq!(document["written"], serde_json::json!([]));
    assert_eq!(document["edition"]["from"], "0.1");
    assert!(
        document["ita_una"]
            .as_str()
            .unwrap()
            .starts_with("ita demo_pkg\n")
    );
    assert!(dir.join("una.toml").exists());
    assert!(!dir.join("ita.una").exists());
}

#[test]
fn migrate_prints_the_clashes_of_edition_0_4() {
    let source = "module demo\n\npub within() -> Int = 1\n";
    let dir = toml_package("migrate-clash", "0.3", source);
    let (code, out, err) = una(&["una", "migrate", path(&dir)]);
    assert_eq!(code, 0);
    assert!(out.ends_with("edition 0.3 -> 0.4: 1 diagnostic\n"), "{out}");
    assert!(err.contains("U0103"), "{err}");
    // It applies no repair: the module is unchanged.
    assert_eq!(
        std::fs::read_to_string(dir.join("src/demo.una")).unwrap(),
        source
    );
}

#[test]
fn una_toml_naming_edition_0_4_is_refused_with_migrate() {
    let dir = toml_package("toml-0-4", "0.4", MODULE);
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    let codes: Vec<String> = checked
        .diagnostics
        .iter()
        .map(|d| format!("{} {}", d.code, d.message))
        .collect();
    assert_eq!(
        codes,
        ["U7002 edition 0.4 is described by ita.una, not una.toml; run `una migrate`"]
    );
    let (code, _, _) = una(&["una", "migrate", path(&dir)]);
    assert_eq!(code, 0);
    assert!(dir.join("ita.una").exists());
}

/// A package with the path dependency `deps/util`, needed with `ceiling` as its `may` lines.
fn with_dependency(name: &str, may: &str) -> PathBuf {
    let manifest = format!(
        "ita demo\n  version 0.1.0\n  edition 0.4\n\nneeds\n  util from path \"deps/util\"\n{may}    accept name\n"
    );
    let dir = package(name, &manifest);
    std::fs::create_dir_all(dir.join("deps/util/src")).unwrap();
    std::fs::write(
        dir.join("deps/util/ita.una"),
        "ita util\n  version 0.2.0\n  edition 0.4\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps/util/src/say.una"),
        "module say\n\npub say(line: Text) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(line)\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/demo.una"),
        "module demo\n\nimport util.say (say)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  say(\"hi\") with @console = @console[print]\n",
    )
    .unwrap();
    dir
}

#[test]
fn lock_records_a_path_dependency_and_its_consented_ceiling() {
    let dir = with_dependency("path-lock", "    may Console[print]\n");
    // A ceiling is recorded only when accepted.
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1);
    assert!(
        err.contains("U7009") && err.contains("--accept-authority"),
        "{err}"
    );
    assert!(!dir.join("ita.lock").exists());
    let (code, out, err) = una(&["una", "ita", "lock", "--accept-authority", path(&dir)]);
    assert_eq!((code, out.as_str()), (0, "wrote ita.lock\n"), "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(
        lock.contains("\npackage util 0.2.0\n  from path \"deps/util\"\n  digest \"sha256:"),
        "{lock}"
    );
    assert!(
        lock.ends_with("  granted by demo may Console[print]\n"),
        "{lock}"
    );
    // The package checks, and the lock is what `una ita lock` writes.
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    assert_eq!(checked.diagnostics, []);
    let (code, _, _) = una(&["una", "ita", "lock", "--check", path(&dir)]);
    assert_eq!(code, 0);
    // A changed byte of the dependency makes the lock stale for `--check`.
    std::fs::write(
        dir.join("deps/util/src/say.una"),
        "module say\n\npub say(line: Text) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(line)\n\npub two() -> Int = 2\n",
    )
    .unwrap();
    let (code, _, err) = una(&["una", "ita", "lock", "--check", path(&dir)]);
    assert_eq!(code, 1);
    assert!(err.contains("U7009"), "{err}");
}

#[test]
fn a_wider_ceiling_needs_accept_authority() {
    let dir = with_dependency("path-widen", "    may Console[print]\n");
    let (code, _, _) = una(&["una", "ita", "lock", "--accept-authority", path(&dir)]);
    assert_eq!(code, 0);
    let manifest = std::fs::read_to_string(dir.join("ita.una"))
        .unwrap()
        .replace("may Console[print]", "may Console[print], Clock[now]");
    std::fs::write(dir.join("ita.una"), manifest).unwrap();
    // Loading reports the widening at the `may` line.
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    assert!(
        checked
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "U7009"),
        "{:?}",
        checked.diagnostics
    );
    let before = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1);
    assert!(err.contains("widens to `Clock[now]`"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.lock")).unwrap(),
        before
    );
    let (code, _, _) = una(&["una", "ita", "lock", "--accept-authority", path(&dir)]);
    assert_eq!(code, 0);
    assert!(
        std::fs::read_to_string(dir.join("ita.lock"))
            .unwrap()
            .contains("may Clock[now], Console[print]")
    );
}

#[test]
fn a_dependency_reached_through_a_link_is_refused() {
    let dir = with_dependency("path-link", "");
    #[cfg(unix)]
    {
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("x.una"), "module x\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("deps/util/src/linked")).unwrap();
        let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
        let messages: Vec<String> = checked
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect();
        assert!(
            messages.iter().any(|m| m.contains("symbolic link")),
            "{messages:?}"
        );
    }
}
