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
/// through, and a lock that is a link leading outside the package is replaced, not followed.
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
    // A valid lock that differs from the one the command writes, so the run reaches the write.
    let pinned = "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 7\n  seed 3\n";
    std::fs::write(&outside, pinned).unwrap();
    std::fs::remove_file(dir.join("ita.lock")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("ita.lock")).unwrap();
    // `ita.lock` belongs to the toolchain: the link is replaced by a regular file, never
    // followed, and the file outside keeps its bytes.
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), pinned);
    assert!(
        std::fs::symlink_metadata(dir.join("ita.lock"))
            .unwrap()
            .file_type()
            .is_file()
    );
    let _ = std::fs::remove_file(&victim);
    let _ = std::fs::remove_file(&outside);
}

/// An `ita.lock` that is a link to another file inside the package is replaced by a regular
/// file, and the other file is untouched: the lock is the toolchain's, so a link at its name
/// is never followed, wherever it leads.
#[cfg(unix)]
#[test]
fn a_lock_that_is_a_link_inside_the_package_is_replaced() {
    let dir = package("lock-link-inside", MANIFEST);
    let pinned = "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 7\n  seed 3\n";
    std::fs::write(dir.join("other.lock"), pinned).unwrap();
    std::os::unix::fs::symlink(dir.join("other.lock"), dir.join("ita.lock")).unwrap();
    let (code, out, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!((code, out.as_str()), (0, "wrote ita.lock\n"), "{err}");
    assert!(
        std::fs::symlink_metadata(dir.join("ita.lock"))
            .unwrap()
            .file_type()
            .is_file()
    );
    // The solver section was read through the link and kept; the other file is untouched.
    assert!(
        std::fs::read_to_string(dir.join("ita.lock"))
            .unwrap()
            .contains("  steps 7\n")
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("other.lock")).unwrap(),
        pinned
    );
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
fn lock_refuses_a_registry_need_it_cannot_resolve_offline() {
    let dir = package("needs", &format!("{MANIFEST}\nneeds\n  json 2.0\n"));
    let (code, out, _) = una(&["una", "--json", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["schema"], "una.ita/1");
    assert_eq!(document["command"], "lock");
    assert_eq!(document["written"], serde_json::json!([]));
    assert_eq!(document["changes"], serde_json::json!([]));
    assert_eq!(document["diagnostics"][0]["code"], "U7013");
    assert!(
        document["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("per-name state of `itaty`"),
        "{out}"
    );
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
fn migrate_rewrites_the_modules_in_the_canonical_form_of_edition_0_4() {
    // ADR 0125, amendment of 2026-10-25: soft wrapping is the canonical form of edition 0.4,
    // and `una migrate` applies it. The long call is canonical in edition 0.3, on one line, and
    // wraps after the migration, which leaves nothing for U7001.
    let declarations = "module demo\n\npub add(first: Int, second: Int, third: Int, fourth: Int) -> Int = first + second + third + fourth\n\n";
    let long = "pub total(quantity: Int) -> Int = add(quantity, second: quantity + 1000, third: quantity + 2000, fourth: quantity + 3000)\n";
    let wrapped = "pub total(quantity: Int) -> Int = add(quantity, second: quantity + 1000, third: quantity + 2000,\n  fourth: quantity + 3000\n)\n";
    let dir = toml_package("migrate-wrap", "0.3", &format!("{declarations}{long}"));
    let before = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    assert_eq!(before.diagnostics, []);
    let (code, out, err) = una(&["una", "migrate", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "wrote ita.una, ita.lock, src/demo.una; removed una.toml, verification.lock\nname: demo-pkg -> demo_pkg\nedition 0.3 -> 0.4: 0 diagnostics\n"
    );
    let migrated = format!("{declarations}{wrapped}");
    assert_eq!(
        std::fs::read_to_string(dir.join("src/demo.una")).unwrap(),
        migrated
    );
    let after = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    assert_eq!(after.diagnostics, []);
    let (code, _, err) = una(&["una", "fmt", "--check", path(&dir)]);
    assert_eq!((code, err.as_str()), (0, ""));
    // Migrating twice changes nothing: the package is already of edition 0.4.
    let (code, _, err) = una(&["una", "migrate", path(&dir)]);
    assert_eq!(code, 2);
    assert!(err.contains("already has ita.una"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.join("src/demo.una")).unwrap(),
        migrated
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        "ita demo_pkg\n  version 0.1.0\n  edition 0.4\n"
    );
}

#[test]
fn migrate_rewrites_an_edition_0_5_package_with_the_keywords_of_0_5() {
    // #1744: a `una.toml` of edition 0.5 migrates to `edition 0.5`, and its modules are read
    // with the keywords of 0.5 for the canonical form. `simd` is a name there and reserved in
    // 0.4 (ADR 0139), so with the keywords of 0.4 this module would have a syntax error, no
    // canonical form, and its long line would be left for U7001.
    let declarations = "module demo\n\npub add(first: Int, second: Int, third: Int, fourth: Int) -> Int = first + second + third + fourth\n\n";
    let long = "pub total(simd: Int) -> Int = add(simd, second: simd + 1000000, third: simd + 2000000, fourth: simd + 3000000)\n";
    let wrapped = "pub total(simd: Int) -> Int = add(simd, second: simd + 1000000, third: simd + 2000000,\n  fourth: simd + 3000000\n)\n";
    let dir = toml_package("migrate-wrap-0-5", "0.5", &format!("{declarations}{long}"));
    let (code, out, err) = una(&["una", "migrate", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "wrote ita.una, ita.lock, src/demo.una; removed una.toml, verification.lock\nname: demo-pkg -> demo_pkg\nedition 0.5 -> 0.5: 0 diagnostics\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("src/demo.una")).unwrap(),
        format!("{declarations}{wrapped}")
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        "ita demo_pkg\n  version 0.1.0\n  edition 0.5\n"
    );
    let after = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    assert_eq!(after.diagnostics, []);
    let (code, _, err) = una(&["una", "fmt", "--check", path(&dir)]);
    assert_eq!((code, err.as_str()), (0, ""));
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
        err.contains("U7009") && err.contains("--accept-authority util`"),
        "{err}"
    );
    assert!(!dir.join("ita.lock").exists());
    let (code, out, err) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        path(&dir),
    ]);
    assert_eq!(code, 0, "{err}");
    // The review comes first: the package, its digest, its ceiling and what its code uses.
    assert!(
        out.starts_with("add util 0.2.0 (path deps/util): digest sha256:")
            && out.ends_with("; may Console[print]; uses Console[print]\nwrote ita.lock\n"),
        "{out}"
    );
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
    let (code, _, _) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        path(&dir),
    ]);
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
    let (code, _, _) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        path(&dir),
    ]);
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

#[test]
fn a_ceiling_message_names_the_manifest_of_the_package() {
    // #1084: in edition 0.4 a dependency's ceiling is written in `ita.una`, so the message of a
    // grant beyond it names `ita.una`, never `una.toml`.
    let case = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/project/ita_path_dep_beyond_ceiling");
    let (code, _, stderr) = una(&["una", "check", path(&case)]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("[U3021]"), "{stderr}");
    assert!(
        stderr.contains("whose ceiling in `ita.una` allows"),
        "{stderr}"
    );
    assert!(!stderr.contains("una.toml"), "{stderr}");
}

#[test]
fn tree_prints_each_dependency_with_its_ceiling_and_authority() {
    let dir = with_dependency("tree", "    may Console[print]\n");
    let (code, _, _) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        path(&dir),
    ]);
    assert_eq!(code, 0);
    let (code, out, err) = una(&["una", "ita", "tree", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "demo 0.1.0\n  util 0.2.0 (path deps/util): may Console[print] (by demo); uses Console[print]; trust none\n"
    );
    let (_, out, _) = una(&["una", "--json", "ita", "tree", path(&dir)]);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["schema"], "una.ita/1");
    let util = &document["packages"][0];
    assert_eq!(util["package"], "util");
    assert_eq!(
        util["effective_ceiling"],
        serde_json::json!(["Console[print]"])
    );
    assert_eq!(util["granted_by"][0]["package"], "demo");
    assert!(util["digest"].as_str().unwrap().starts_with("sha256:"));
}

#[test]
fn remove_takes_the_need_out_and_relocks() {
    let dir = with_dependency("remove", "");
    let (code, _, _) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0);
    std::fs::write(
        dir.join("src/demo.una"),
        "module demo\n\npub one() -> Int = 1\n",
    )
    .unwrap();
    let (code, out, err) = una(&["una", "ita", "remove", "util", path(&dir)]);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "drop util 0.2.0 (path deps/util)\nwrote ita.lock\nwrote ita.una\n"
        ),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        "ita demo\n  version 0.1.0\n  edition 0.4\n"
    );
    assert!(
        !std::fs::read_to_string(dir.join("ita.lock"))
            .unwrap()
            .contains("package util")
    );
    let (code, _, err) = una(&["una", "ita", "remove", "util", path(&dir)]);
    assert_eq!(code, 2);
    assert!(err.contains("does not need `util`"), "{err}");
}

/// A package needing two path dependencies, `util` and `tool`, with `ita.una` written with
/// `line_end` after every line.
fn with_two_dependencies(name: &str, line_end: &str) -> PathBuf {
    let manifest = [
        "ita demo",
        "  version 0.1.0",
        "  edition 0.4",
        "",
        "needs",
        "  util from path \"deps/util\"",
        "    accept name",
        "  tool from path \"deps/tool\"",
        "    accept name",
    ]
    .iter()
    .map(|line| format!("{line}{line_end}"))
    .collect::<String>();
    let dir = package(name, &manifest);
    for (dep, version) in [("util", "0.2.0"), ("tool", "0.1.0")] {
        std::fs::create_dir_all(dir.join(format!("deps/{dep}/src"))).unwrap();
        std::fs::write(
            dir.join(format!("deps/{dep}/ita.una")),
            format!("ita {dep}\n  version {version}\n  edition 0.4\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join(format!("deps/{dep}/src/{dep}.una")),
            format!("module {dep}\n\npub one() -> Int = 1\n"),
        )
        .unwrap();
    }
    dir
}

#[test]
fn remove_edits_an_ita_una_with_crlf_line_ends() {
    let dir = with_two_dependencies("remove-crlf", "\r\n");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = una(&["una", "ita", "remove", "util", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("drop util 0.2.0 (path deps/util)\n"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        "ita demo\n  version 0.1.0\n  edition 0.4\n\nneeds\n  tool from path \"deps/tool\"\n    accept name\n"
    );
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(
        lock.contains("\npackage tool 0.1.0\n") && !lock.contains("package util"),
        "{lock}"
    );
}

/// A lone `\r` just before a `\r\n` decodes to a `\r\n` that is the lone `\r`, never one
/// more line end: `ita.una` is decoded once (#2252), so `una check` and `una fmt` report it
/// (U7008) at the `\r`, and `una ita remove` refuses the file and writes nothing.
#[test]
fn a_lone_carriage_return_before_a_line_end_is_reported_everywhere() {
    let dir = with_two_dependencies("lone-cr", "\r\n");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    let manifest = std::fs::read_to_string(dir.join("ita.una"))
        .unwrap()
        .replacen("\r\n", "\r\r\n", 1);
    std::fs::write(dir.join("ita.una"), &manifest).unwrap();
    let lone = |out: &str| {
        let document: serde_json::Value = serde_json::from_str(out).unwrap();
        let diagnostics = document["diagnostics"].as_array().unwrap().clone();
        assert!(
            diagnostics.iter().any(|d| d["code"] == "U7008"
                && d["message"] == "carriage return not followed by a line feed"
                && d["primary"]["start"] == serde_json::json!({"line": 1, "col": 9})
                && d["primary"]["end"] == serde_json::json!({"line": 1, "col": 10})),
            "{out}"
        );
    };
    let (code, out, _) = una(&["una", "--json", "check", path(&dir)]);
    assert_eq!(code, 1, "{out}");
    lone(&out);
    let file = dir.join("ita.una");
    let (code, out, _) = una(&["una", "--json", "fmt", file.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    lone(&out);
    let (code, _, err) = una(&["una", "ita", "remove", "util", path(&dir)]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("does not parse"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), manifest);
}

/// Appends `-- x\r\n` comment lines to `text` until it holds more than `bound` bytes while its
/// decoded text, each `\r\n` read as `\n`, stays within `bound`.
fn past_in_bytes_only(mut text: String, bound: usize) -> String {
    while text.len() <= bound {
        text.push_str("-- x\r\n");
    }
    assert!(text.len() - text.matches("\r\n").count() <= bound);
    text
}

/// An `ita.una` of more than 1 MiB of bytes whose `\r\n` line ends bring its decoded text under
/// 1 MiB is past its bound (`spec/30 §1` rule 4, #2282): every command measures the bytes and
/// refuses it, with the one message that states them, and writes nothing.
#[test]
fn a_manifest_past_its_bound_in_bytes_only_is_refused_everywhere() {
    let dir = with_two_dependencies("crlf-bound", "\r\n");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    let file = dir.join("ita.una");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let manifest = past_in_bytes_only(std::fs::read_to_string(&file).unwrap(), 1 << 20);
    std::fs::write(&file, &manifest).unwrap();
    let message = format!(
        "the file holds {} bytes, more than the 1048576 a manifest may hold",
        manifest.len()
    );
    let refused = |out: &str| {
        let document: serde_json::Value = serde_json::from_str(out).unwrap();
        let diagnostics = document["diagnostics"].as_array().unwrap().clone();
        assert!(
            diagnostics
                .iter()
                .any(|d| d["code"] == "U7008" && d["message"] == message.as_str()),
            "{out}"
        );
    };
    let (code, out, _) = una(&["una", "--json", "check", path(&dir)]);
    assert_eq!(code, 1, "{out}");
    refused(&out);
    let (code, out, _) = una(&["una", "--json", "fmt", file.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    refused(&out);
    let (code, out, _) = una(&["una", "--json", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1, "{out}");
    refused(&out);
    let (code, _, err) = una(&["una", "ita", "remove", "util", path(&dir)]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("does not parse"), "{err}");
    let (code, _, err) = una_at(&dir, &["ita", "update", "util"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("does not parse"), "{err}");
    let (code, _, err) = una_at(&dir, &["ita", "add", "json", "2.1"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("does not parse"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), manifest);
    assert_eq!(std::fs::read_to_string(dir.join("ita.lock")).unwrap(), lock);
}

/// An `ita.lock` of more than 16 MiB of bytes whose `\r\n` line ends bring its decoded text
/// under 16 MiB is past its bound (`spec/30 §1` rule 4, #2282), and `una check` refuses it
/// (U7009) with the message that states its bytes.
#[test]
fn a_lock_past_its_bound_in_bytes_only_is_refused() {
    let dir = with_two_dependencies("crlf-lock-bound", "\n");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock"))
        .unwrap()
        .replace('\n', "\r\n");
    let lock = past_in_bytes_only(lock, 16 << 20);
    std::fs::write(dir.join("ita.lock"), &lock).unwrap();
    let (code, out, _) = una(&["una", "--json", "check", path(&dir)]);
    assert_eq!(code, 1, "{out}");
    let message = format!(
        "the file holds {} bytes, more than the 16777216 a lock may hold",
        lock.len()
    );
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "U7009" && d["message"] == message.as_str()),
        "{out}"
    );
}

/// A project file far past its bound is refused by the length it holds, read no further than
/// one byte past the bound (#2282).
#[test]
fn a_manifest_far_past_its_bound_is_refused_by_its_length() {
    let dir = package("far-past-bound", MANIFEST);
    let file = dir.join("ita.una");
    let handle = std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap();
    // A sparse file: 64 MiB of length, none of it written.
    handle.set_len(64 << 20).unwrap();
    let (code, out, _) = una(&["una", "--json", "check", path(&dir)]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("the file holds 67108864 bytes, more than the 1048576 a manifest may hold"),
        "{out}"
    );
}

/// Runs `una` in `dir` with `args`, and fails rather than waits if it has not ended within
/// 20 seconds: the exit code and stderr.
#[cfg(unix)]
fn una_ends(dir: &Path, args: &[&str]) -> (u8, String) {
    una_ends_within(dir, args, 20)
}

/// [`una_ends`] with a limit of `seconds`.
#[cfg(unix)]
fn una_ends_within(dir: &Path, args: &[&str], seconds: u64) -> (u8, String) {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_una"))
        .current_dir(dir)
        .args(args)
        .env_remove("UNA_ITAOCA")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        let blocked = start.elapsed() > std::time::Duration::from_secs(seconds);
        if blocked {
            let _ = child.kill();
        }
        assert!(!blocked, "`una {}` blocked on a FIFO", args.join(" "));
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut err = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err).unwrap();
    (u8::try_from(status.code().unwrap()).unwrap(), err)
}

/// A FIFO at `ita.una` is not a regular file: every command that reads it refuses it at once,
/// before reading, and none blocks waiting for a writer (#2313).
#[cfg(unix)]
#[test]
fn a_fifo_at_ita_una_is_refused_and_never_blocks() {
    let dir = package("fifo-manifest", MANIFEST);
    let manifest = dir.join("ita.una");
    std::fs::remove_file(&manifest).unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(&manifest)
        .status()
        .unwrap();
    assert!(made.success());
    for args in [
        &["ita", "remove", "util"][..],
        &["ita", "add", "json", "2.1"],
        &["ita", "update", "util"],
    ] {
        let (code, err) = una_ends(&dir, args);
        assert_ne!(code, 0, "`una {}`: {err}", args.join(" "));
        assert!(
            err.contains("is not a regular file"),
            "`una {}`: {err}",
            args.join(" ")
        );
    }
    // `yank` and `fmt` look for a regular `ita.una` before they read one, as `una check` does.
    for args in [&["ita", "yank", "0.1.0"][..], &["fmt", "ita.una"]] {
        let (code, err) = una_ends(&dir, args);
        assert_eq!(code, 2, "`una {}`: {err}", args.join(" "));
    }
}

/// A FIFO or a directory at `ita.lock` is not a regular file: `una check` and `una ita verify`
/// refuse it as such, at once, and never report it as a missing lock (#2348).
#[cfg(unix)]
#[test]
fn an_ita_lock_that_is_not_a_regular_file_is_refused_not_missing() {
    let dir = package("fifo-lock", MANIFEST);
    let lock = dir.join("ita.lock");
    let commands = [&["check", "."][..], &["ita", "verify", "."]];
    // With no lock at all, `verify` reports the missing lock.
    let (code, err) = una_ends(&dir, &["ita", "verify", "."]);
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("has no ita.lock"), "{err}");
    let made = std::process::Command::new("mkfifo")
        .arg(&lock)
        .status()
        .unwrap();
    assert!(made.success());
    let refused = |what: &str| {
        for args in commands {
            let (code, err) = una_ends(&dir, args);
            let shown = format!("`una {}` with a {what} at ita.lock: {err}", args.join(" "));
            assert_ne!(code, 0, "{shown}");
            assert!(err.contains("is not a regular file"), "{shown}");
            assert!(!err.contains("has no ita.lock"), "{shown}");
        }
    };
    refused("FIFO");
    std::fs::remove_file(&lock).unwrap();
    std::fs::create_dir(&lock).unwrap();
    refused("directory");
}

/// A module under `src/` that is a FIFO, or a link to `/dev/zero`, is refused by `una check` as
/// not a regular file, at once: it never waits for a writer, and never reads an endless device
/// into memory (#2392). The limit on `/dev/zero` is short, so a regression is killed before it
/// holds much.
#[cfg(unix)]
#[test]
fn a_module_that_is_not_a_regular_file_is_refused_by_check() {
    let dir = package("fifo-module", MANIFEST);
    let module = dir.join("src/x.una");
    let made = std::process::Command::new("mkfifo")
        .arg(&module)
        .status()
        .unwrap();
    assert!(made.success());
    let (code, err) = una_ends(&dir, &["check", "."]);
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("is not a regular file"), "{err}");
    std::fs::remove_file(&module).unwrap();
    std::os::unix::fs::symlink("/dev/zero", &module).unwrap();
    let (code, err) = una_ends_within(&dir, &["check", "."], 3);
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("is not a regular file"), "{err}");
}

/// A link at `ita.lock` that names nothing is refused as a broken link by `una check` and `una
/// ita verify`, never reported as a missing lock with the advice to write one (#2395).
#[cfg(unix)]
#[test]
fn a_dangling_link_at_ita_lock_is_refused_as_broken() {
    let dir = package("dangling-lock", MANIFEST);
    std::os::unix::fs::symlink(dir.join("moved/ita.lock"), dir.join("ita.lock")).unwrap();
    for args in [&["check", "."][..], &["ita", "verify", "."]] {
        let (code, err) = una_ends(&dir, args);
        let shown = format!("`una {}`: {err}", args.join(" "));
        assert_ne!(code, 0, "{shown}");
        assert!(
            err.contains("is a symbolic link that names nothing"),
            "{shown}"
        );
        assert!(!err.contains("una ita lock"), "{shown}");
    }
}

/// An ill-formed UTF-8 sequence in `ita.una` is reported (U7008) where it is: the file is
/// decoded once, by the source database, and read as decoded (#2252).
#[test]
fn an_ill_formed_sequence_in_ita_una_is_reported() {
    let dir = package("ill-formed", "");
    std::fs::write(
        dir.join("ita.una"),
        b"ita demo\n  version 0.1.0\n  edition 0.4\n  about \"a\xffb\"\n",
    )
    .unwrap();
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1, "{err}");
    let (code, out, _) = una(&["una", "--json", "check", path(&dir)]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("invalid UTF-8 byte sequence") && out.contains("U7008"),
        "{out}"
    );
}

/// A write that fails exits with code 1 and a message, never a usage error, and changes
/// neither file.
#[cfg(unix)]
#[test]
fn a_write_that_fails_exits_1_and_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let dir = with_two_dependencies("write-fails", "\n");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    // A user the permissions do not bind, such as root, can still write: nothing to test then.
    let probe = dir.join("probe");
    let bound = std::fs::write(&probe, b"x").is_err();
    let _ = std::fs::remove_file(&probe);
    let (code, out, err) = una(&["una", "ita", "remove", "util", path(&dir)]);
    let (json_code, json, _) = una(&["una", "--json", "ita", "remove", "util", path(&dir)]);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    if bound {
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("cannot write"), "{err}");
        assert!(!out.contains("wrote"), "{out}");
        // With `--json`, one document with the error.
        assert_eq!(json_code, 1);
        let document: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(document["schema"], "una.ita/1");
        assert_eq!(document["command"], "remove");
        assert_eq!(document["written"], serde_json::json!([]));
        assert!(
            document["error"].as_str().unwrap().contains("cannot write"),
            "{json}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("ita.una")).unwrap(),
            manifest
        );
        assert_eq!(std::fs::read_to_string(dir.join("ita.lock")).unwrap(), lock);
    }
}

/// When the lock is written and `ita.una` then cannot be, the lock is put back, byte for byte,
/// and the command exits with code 1. `ita.una` is a link to a file in a directory of the
/// package that cannot be written, so its write fails whether the writer follows a link inside
/// the package (it cannot write beside the target) or refuses every link (it refuses this
/// one), while the lock, in the package's own directory, is written. Only a user the
/// permissions do not bind, such as root, with a writer that follows the link, writes it; the
/// test then has nothing to check.
#[cfg(unix)]
#[test]
fn the_lock_is_put_back_when_ita_una_cannot_be_written() {
    use std::os::unix::fs::PermissionsExt;
    let dir = with_two_dependencies("restore", "\n");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    // The lock with bytes `una ita lock` would not write, a comment, which must come back.
    let lock = format!(
        "-- kept as written\n{}",
        std::fs::read_to_string(dir.join("ita.lock")).unwrap()
    );
    std::fs::write(dir.join("ita.lock"), &lock).unwrap();
    let held = dir.join("held");
    std::fs::create_dir_all(&held).unwrap();
    std::fs::rename(dir.join("ita.una"), held.join("ita.una")).unwrap();
    std::os::unix::fs::symlink(held.join("ita.una"), dir.join("ita.una")).unwrap();
    let manifest = std::fs::read_to_string(held.join("ita.una")).unwrap();
    std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = held.join("probe");
    let bound = std::fs::write(&probe, b"x").is_err();
    let _ = std::fs::remove_file(&probe);
    let (code, _, remove_err) = una(&["una", "ita", "remove", "util", path(&dir)]);
    std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o755)).unwrap();
    if code == 0 {
        // Only a user the permissions do not bind, such as root, writes through the link.
        assert!(!bound, "{remove_err}");
        return;
    }
    assert_eq!(code, 1, "{remove_err}");
    assert!(
        remove_err.contains("cannot write") && remove_err.contains("was put back as it was"),
        "{remove_err}"
    );
    assert_eq!(std::fs::read_to_string(dir.join("ita.lock")).unwrap(), lock);
    assert_eq!(
        std::fs::read_to_string(held.join("ita.una")).unwrap(),
        manifest
    );
}

#[test]
fn update_raises_only_the_named_needs() {
    let dir = with_two_dependencies("update-named", "\n");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    for (dep, version) in [("util", "0.2.1"), ("tool", "0.1.1")] {
        std::fs::write(
            dir.join(format!("deps/{dep}/ita.una")),
            format!("ita {dep}\n  version {version}\n  edition 0.4\n"),
        )
        .unwrap();
    }
    // `update util` locks util again and keeps tool as the lock records it.
    let (code, out, err) = una_at(&dir, &["ita", "update", "util"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("change util 0.2.0 -> 0.2.1 (path deps/util)") && !out.contains("tool"),
        "{out}"
    );
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(
        lock.contains("\npackage util 0.2.1\n") && lock.contains("\npackage tool 0.1.0\n"),
        "{lock}"
    );
    // Every need: tool too.
    let (code, out, err) = una_at(&dir, &["ita", "update"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("change tool 0.1.0 -> 0.1.1"), "{out}");
    // Authority is accepted per package: a widened tool ceiling is not accepted by naming
    // util, and `update util` does not touch tool's.
    let manifest = std::fs::read_to_string(dir.join("ita.una"))
        .unwrap()
        .replace(
            "  tool from path \"deps/tool\"\n",
            "  tool from path \"deps/tool\"\n    may Console[print]\n",
        );
    std::fs::write(dir.join("ita.una"), manifest).unwrap();
    let (code, _, err) = una_at(&dir, &["ita", "update", "--accept-authority", "util"]);
    assert_eq!(code, 1);
    assert!(err.contains("--accept-authority tool`"), "{err}");
    let (code, out, _) = una_at(&dir, &["ita", "update", "util"]);
    assert_eq!((code, out.as_str()), (0, "ita.lock is up to date\n"));
    let (code, _, err) = una_at(&dir, &["ita", "update", "--accept-authority", "tool"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        std::fs::read_to_string(dir.join("ita.lock"))
            .unwrap()
            .contains("  granted by demo may Console[print]\n")
    );
}

#[test]
fn trust_is_consented_with_accept_trust_and_pinned_to_the_content() {
    let dir = with_dependency("trust", "    may Console[print]\n");
    let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!("{manifest}\ntrust\n  util modules keys\n    may Console[print]\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("deps/util/ita.una"),
        "ita util\n  version 0.2.0\n  edition 0.4\n\ntrusted\n  keys\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps/util/src/keys.una"),
        "trusted module keys\n\npub policy reveal\n  from secret Text\n  to Text\n  via shown\n  purpose \"show a token\"\n  owner team\n\nshown(t: secret Text) -> Text = t\n",
    )
    .unwrap();
    // No consent yet: refused, and nothing written; the refusal names the flag to copy, with
    // the digest of the content to consent to.
    let (code, out, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1);
    assert!(
        err.contains("U7012") && err.contains("--accept-trust util@sha256:"),
        "{err}"
    );
    assert!(
        out.contains("to accept, after reviewing: una ita lock "),
        "{out}"
    );
    assert!(!dir.join("ita.lock").exists());
    let flag = trust_flag(&err);
    // Consent to a package the root does not need is a usage error, and so is consent to two
    // contents of one package.
    let (code, _, err) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        "--accept-trust",
        &format!("other@sha256:{}", "0".repeat(64)),
        path(&dir),
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("names no need"), "{err}");
    let (code, _, err) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-trust",
        &flag,
        "--accept-trust",
        &format!("util@sha256:{}", "0".repeat(64)),
        path(&dir),
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("names `util` twice"), "{err}");
    assert!(!dir.join("ita.lock").exists());
    // A flag that names no digest is a usage error.
    let (code, _, err) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        "--accept-trust",
        "util",
        path(&dir),
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("<package>@sha256:<hex>"), "{err}");
    let (code, out, err) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        "--accept-trust",
        &flag,
        path(&dir),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.ends_with("\nwrote ita.lock\n"), "{out}");
    assert!(out.contains("; trust keys; consented sha256:"), "{out}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(
        lock.contains("  consented \"sha256:") && lock.contains("  trusted modules keys\n"),
        "{lock}"
    );
    // The consent holds while the content is the same.
    let (code, out, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(
        (code, out.as_str()),
        (0, "ita.lock is up to date\n"),
        "{err}"
    );
    let (code, _, err) = una(&["una", "check", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    // Any byte of the package changed, even outside the trusted module: consent again.
    std::fs::write(
        dir.join("deps/util/src/say.una"),
        "module say\n\npub say(line: Text) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(line)\n\npub two() -> Int = 2\n",
    )
    .unwrap();
    let (code, _, err) = una(&["una", "check", path(&dir)]);
    assert_eq!(code, 1);
    assert!(err.contains("U7012"), "{err}");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1);
    assert!(err.contains("--accept-trust util@sha256:"), "{err}");
    // The consent to the old content is stale: refused, and said so.
    let (code, _, err) = una(&["una", "ita", "lock", "--accept-trust", &flag, path(&dir)]);
    assert_eq!(code, 1);
    assert!(err.contains("consents to other content"), "{err}");
    let fresh = trust_flag(&err);
    assert_ne!(fresh, flag);
    let (code, _, err) = una(&["una", "ita", "lock", "--accept-trust", &fresh, path(&dir)]);
    assert_eq!(code, 0, "{err}");
    // A change of the consent alone is a change of the review: the lock's consent lines
    // removed by hand are written again, and the review says so.
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let stripped: String = lock
        .lines()
        .filter(|line| !line.starts_with("  consented") && !line.starts_with("  trusted modules"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(dir.join("ita.lock"), stripped).unwrap();
    let (code, out, err) = una(&["una", "ita", "lock", "--accept-trust", &fresh, path(&dir)]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("change util 0.2.0 (path deps/util): digest sha256:")
            && out.contains("; trust none -> keys; consented none -> sha256:"),
        "{out}"
    );
    assert_eq!(std::fs::read_to_string(dir.join("ita.lock")).unwrap(), lock);
}

/// `una ita tree` reports the effective ceiling of a package with trusted modules as its
/// ceiling meets the trust line's, the package's own profile, and the digest of the bytes
/// analysed, which is not the lock's once the files change.
#[test]
fn tree_reports_the_effective_ceiling_profile_and_digest_analysed() {
    let dir = with_dependency("tree-trust", "    may Console[print], Clock[now]\n");
    let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!("{manifest}\ntrust\n  util modules keys\n    may Console[print]\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("deps/util/ita.una"),
        "ita util\n  version 0.2.0\n  edition 0.4\n  profile app\n\ntrusted\n  keys\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps/util/src/keys.una"),
        "trusted module keys\n\npub policy reveal\n  from secret Text\n  to Text\n  via shown\n  purpose \"show a token\"\n  owner team\n\nshown(t: secret Text) -> Text = t\n",
    )
    .unwrap();
    let (_, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    let flag = trust_flag(&err);
    let (code, _, err) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        "--accept-trust",
        &flag,
        path(&dir),
    ]);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = una(&["una", "--json", "ita", "tree", path(&dir)]);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    let util = &document["packages"][0];
    assert_eq!(
        util["effective_ceiling"],
        serde_json::json!(["Console[print]"])
    );
    assert_eq!(
        util["granted_by"][0]["may"],
        serde_json::json!(["Console[print]", "Clock[now]"])
    );
    assert_eq!(util["profile"], "app");
    let digest = flag.trim_start_matches("util@").to_owned();
    assert_eq!(util["digest"], serde_json::json!(digest));
    // A file changed after the lock: the digest is of what was analysed, not the lock's.
    std::fs::write(
        dir.join("deps/util/src/say.una"),
        "module say\n\npub say(line: Text) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(line)\n\npub two() -> Int = 2\n",
    )
    .unwrap();
    let (_, out, _) = una(&["una", "--json", "ita", "tree", path(&dir)]);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    let changed = document["packages"][0]["digest"].as_str().unwrap();
    assert!(changed.starts_with("sha256:") && changed != digest, "{out}");
}

/// The `--accept-trust` flag a refusal names, `util@sha256:<hex>`.
fn trust_flag(text: &str) -> String {
    let at = text.rfind("--accept-trust util@sha256:").unwrap() + "--accept-trust ".len();
    text[at..at + "util@sha256:".len() + 64].to_owned()
}

#[test]
fn consent_is_reviewed_before_it_is_given_and_names_each_package() {
    let dir = with_dependency("consent-review", "    may Console[print]\n");
    // `--dry-run` without consent: the review is printed with the flag to copy, and the
    // command refuses and writes nothing.
    let (code, out, err) = una(&["una", "ita", "lock", "--dry-run", path(&dir)]);
    assert_eq!(code, 1);
    assert!(
        out.starts_with("add util 0.2.0 (path deps/util): digest sha256:")
            && out.contains("\nto accept, after reviewing: una ita lock --accept-authority util\n"),
        "{out}"
    );
    assert!(err.contains("U7009"), "{err}");
    assert!(!dir.join("ita.lock").exists());
    // The JSON document holds the review too.
    let (code, out, _) = una(&["una", "--json", "ita", "lock", "--dry-run", path(&dir)]);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["changes"][0]["package"], "util");
    assert_eq!(document["written"], serde_json::json!([]));
    // Accepting the authority of a package the root does not need is a usage error.
    let (code, out, err) = una(&[
        "una",
        "--json",
        "ita",
        "lock",
        "--accept-authority",
        "other",
        path(&dir),
    ]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(
        err.contains("`--accept-authority other` names no need"),
        "{err}"
    );
    assert!(!dir.join("ita.lock").exists());
    // A bare flag is refused: it must name the package.
    let (code, _, err) = una(&["una", "ita", "lock", "--accept-authority"]);
    assert_eq!(code, 2);
    assert!(err.contains("PACKAGE"), "{err}");
}

/// Runs `una` with `args` and Itaoca at `itaoca`: the exit code, stdout and stderr.
fn una_in(args: &[&str], itaoca: &Path) -> (u8, String, String) {
    let store = itaoca.to_str().unwrap().to_owned();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = una_cli::run(
        args.iter().copied(),
        &mut stdout,
        &mut stderr,
        false,
        &|name| Ok((name == "UNA_ITAOCA").then(|| store.clone())),
        una_cli::Heap::unmeasured(),
    );
    (
        status.code(),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

/// A test registry `acme` with one release, `acme/ledger 0.3.1`, signed by its own root,
/// log, witnesses and publisher (ADR 0122 rules 35 to 38).
mod registry {
    use ed25519_dalek::{Signer, SigningKey};
    use una_project::content::{File, archive, digest_of};
    use una_project::provenance::{Format, Provenance, sha256_digest, signed_name};
    use una_project::tlog::merkle::{inclusion_proof, leaf_hash, root as tree_root};
    use una_project::tlog::note::{Key, encode_base64};

    const LOG: &str = "acme.example/log";

    fn keypair(seed: u8, name: &str) -> (SigningKey, Key) {
        let pair = SigningKey::from_bytes(&[seed; 32]);
        let public: [u8; 32] = pair.verifying_key().to_bytes();
        (
            pair,
            Key {
                name: name.to_owned(),
                public,
            },
        )
    }

    fn sign_line(pair: &SigningKey, key: &Key, text: &str) -> String {
        let mut bytes = key.id().to_vec();
        bytes.extend_from_slice(&pair.sign(text.as_bytes()).to_bytes());
        format!("\u{2014} {} {}\n", key.name, encode_base64(&bytes))
    }

    fn cosign_line(pair: &SigningKey, key: &Key, text: &str, time: u64) -> String {
        let message = format!("cosignature/v1\ntime {time}\n{text}");
        let mut bytes = key.cosignature_id().to_vec();
        bytes.extend_from_slice(&time.to_be_bytes());
        bytes.extend_from_slice(&pair.sign(message.as_bytes()).to_bytes());
        format!("\u{2014} {} {}\n", key.name, encode_base64(&bytes))
    }

    fn key_line(key: &Key) -> String {
        format!("key {} {}\n", key.name, encode_base64(&key.public))
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The release: its archive, provenance, root digest, and the lines a lock writes for it.
    pub struct Release {
        pub archive: Vec<u8>,
        pub provenance: Vec<u8>,
        pub root: String,
        pub lock: String,
    }

    pub fn release() -> Release {
        let mut files = vec![
            File {
                path: "ita.una".to_owned(),
                bytes: b"ita acme/ledger\n  version 0.3.1\n  edition 0.4\n".to_vec(),
            },
            File {
                path: "src/ledger.una".to_owned(),
                bytes: b"module ledger\n\npub total() -> Int = 3\n".to_vec(),
            },
        ];
        let archive = archive(&mut files).unwrap();
        let digest = digest_of(&mut files).unwrap();
        let (r1, k1) = keypair(10, "root.a");
        let (log_pair, log) = keypair(20, LOG);
        let (w1, wa) = keypair(30, "witness.a");
        let (_, security) = keypair(40, "security.a");
        let (_, itauna) = keypair(41, "itauna.a");
        let mut text =
            "ita-root/1\nsource acme\nversion 1\nexpires 4000000000\nroot 1\n".to_owned();
        text.push_str(&key_line(&k1));
        text.push_str(&format!(
            "log {} {}\n",
            log.name,
            encode_base64(&log.public)
        ));
        text.push_str("witnesses 1\n");
        text.push_str(&key_line(&wa));
        text.push_str("security 1\n");
        text.push_str(&key_line(&security));
        text.push_str("itauna 1\n");
        text.push_str(&key_line(&itauna));
        let root_document = format!("{text}\n{}", sign_line(&r1, &k1, &text));
        let manifest = sha256_digest(&files.iter().find(|f| f.path == "ita.una").unwrap().bytes);
        let statement_text = format!(
            "ita-release/1\nsource acme\nname acme/ledger\nversion 0.3.1\nprevious none\ndigest {digest}\nmanifest {manifest}\nedition 0.4\nauthority none\ntrust none\n"
        );
        let (publisher_pair, publisher) = keypair(50, "acme.publisher");
        let statement = format!(
            "{statement_text}\n{}",
            sign_line(&publisher_pair, &publisher, &statement_text)
        );
        let leaves = [
            leaf_hash(b"an earlier entry"),
            leaf_hash(statement.as_bytes()),
        ];
        let top = tree_root(&leaves);
        let checkpoint_text = format!(
            "{LOG}\n2\n{}\nita-root 1 {}\n",
            encode_base64(&top),
            una_project::tlog::root::digest(&root_document)
        );
        let checkpoint = format!(
            "{checkpoint_text}\n{}{}",
            sign_line(&log_pair, &log, &checkpoint_text),
            cosign_line(&w1, &wa, &checkpoint_text, 1_900_000_000)
        );
        let root = una_project::tlog::root::digest(&root_document);
        let provenance = Provenance {
            format: Format::V2,
            roots: vec![root_document],
            checkpoint,
            latest: None,
            statement,
            index: 1,
            proof: inclusion_proof(1, &leaves),
            key: publisher.clone(),
        };
        let lock = format!(
            "registry acme\n  root \"{root}\"\n  checkpoint 2 \"sha256:{}\"\n  last root 1 \"{root}\"\n\npackage acme/ledger 0.3.1\n  from acme\n  digest \"{digest}\"\n  signed \"{}\"\n  logged 1\n",
            hex(&top),
            signed_name("acme/ledger", &publisher.public)
        );
        Release {
            archive,
            provenance: provenance.encode(),
            root,
            lock,
        }
    }
}

const SOLVER: &str = "solver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n";

/// A package whose lock holds the registry release, with Itaoca beside it holding the release
/// when `stored`.
fn with_registry(name: &str, stored: bool) -> (PathBuf, PathBuf) {
    let release = registry::release();
    let manifest = format!(
        "{MANIFEST}\nsources\n  acme \"https://ita.acme.example\"\n    root \"{}\"\n    owners acme\n",
        release.root
    );
    let dir = package(name, &manifest);
    std::fs::write(
        dir.join("ita.lock"),
        format!("lock 1\n\n{SOLVER}\n{}", release.lock),
    )
    .unwrap();
    let itaoca = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("ita-store")
        .join(name);
    let _ = std::fs::remove_dir_all(&itaoca);
    std::fs::create_dir_all(&itaoca).unwrap();
    if stored {
        let store = una_project::itaoca::Itaoca {
            root: itaoca.clone(),
        };
        store.store(&release.archive).unwrap();
        store
            .store_provenance("acme", "acme/ledger", "0.3.1", 2, &release.provenance)
            .unwrap();
    }
    (dir, itaoca)
}

/// The codes of `una --json check` with Itaoca at `itaoca`.
fn codes_in(dir: &Path, itaoca: &Path) -> Vec<String> {
    let (_, out, _) = una_in(&["una", "--json", "check", path(dir)], itaoca);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

/// The codes of `una check` with no Itaoca: the vendor directory only.
fn codes(dir: &Path) -> Vec<String> {
    let checked = una_cli::check(dir, una_cli::Tier::Zero).unwrap();
    checked
        .diagnostics
        .iter()
        .map(|d| d.code.to_string())
        .collect()
}

#[test]
fn vendor_copies_each_registry_package_with_its_provenance() {
    let (dir, itaoca) = with_registry("vendor-copies", true);
    // Before: in Itaoca, so present, and verified there; absent with no Itaoca.
    assert!(
        !codes_in(&dir, &itaoca)
            .iter()
            .any(|c| c == "U7010" || c == "U7011")
    );
    assert!(
        codes(&dir).iter().any(|c| c == "U7011"),
        "{:?}",
        codes(&dir)
    );
    let (code, out, err) = una_in(&["una", "ita", "vendor", path(&dir)], &itaoca);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "vendored acme/ledger 0.3.1\n", "")
    );
    let vendored = dir.join("vendor/ita/acme/acme+ledger-0.3.1");
    assert_eq!(
        std::fs::read_to_string(vendored.join("src/ledger.una")).unwrap(),
        "module ledger\n\npub total() -> Int = 3\n"
    );
    assert!(
        dir.join("vendor/ita/acme/acme+ledger-0.3.1.provenance")
            .is_file()
    );
    // Now it builds from the vendor directory alone: no U7010 or U7011, with no Itaoca.
    let found = codes(&dir);
    assert!(
        !found.iter().any(|c| c == "U7010" || c == "U7011"),
        "{found:?}"
    );
    // Again: up to date, with `written` empty.
    let (code, out, _) = una_in(&["una", "--json", "ita", "vendor", path(&dir)], &itaoca);
    assert_eq!(code, 0);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["command"], "vendor");
    assert_eq!(document["written"], serde_json::json!([]));
    // A vendored file changed: U7010 on every load, even with Itaoca holding the release.
    std::fs::write(
        vendored.join("src/ledger.una"),
        "module ledger\n\npub total() -> Int = 4\n",
    )
    .unwrap();
    assert!(codes(&dir).iter().any(|c| c == "U7010"));
    assert!(codes_in(&dir, &itaoca).iter().any(|c| c == "U7010"));
    let (code, _, err) = una_in(&["una", "ita", "vendor", path(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010"), "{err}");
    // The digest and the lock edited together: the stored statement still names the old
    // digest, so it does not load.
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let mut files = vec![
        una_project::content::File {
            path: "ita.una".to_owned(),
            bytes: b"ita acme/ledger\n  version 0.3.1\n  edition 0.4\n".to_vec(),
        },
        una_project::content::File {
            path: "src/ledger.una".to_owned(),
            bytes: b"module ledger\n\npub total() -> Int = 4\n".to_vec(),
        },
    ];
    let edited = una_project::content::digest_of(&mut files).unwrap();
    let old = lock
        .lines()
        .find(|line| line.starts_with("  digest"))
        .unwrap()
        .to_owned();
    std::fs::write(
        dir.join("ita.lock"),
        lock.replacen(&old, &format!("  digest \"{edited}\""), 1),
    )
    .unwrap();
    assert!(codes(&dir).iter().any(|c| c == "U7010"));
}

#[test]
fn vendor_reports_a_package_that_itaoca_does_not_hold() {
    let (dir, itaoca) = with_registry("vendor-absent", false);
    let (code, out, _) = una_in(&["una", "--json", "ita", "vendor", path(&dir)], &itaoca);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["diagnostics"][0]["code"], "U7011");
    assert_eq!(document["written"], serde_json::json!([]));
    assert!(!dir.join("vendor").exists());
    assert!(codes_in(&dir, &itaoca).iter().any(|c| c == "U7011"));
    // A vendor directory reached through a link is refused, and nothing is written there.
    #[cfg(unix)]
    {
        let (dir, itaoca) = with_registry("vendor-through-link", true);
        let away = dir.join("away");
        std::fs::create_dir_all(&away).unwrap();
        std::os::unix::fs::symlink(&away, dir.join("vendor")).unwrap();
        let (code, _, err) = una_in(&["una", "ita", "vendor", path(&dir)], &itaoca);
        assert_eq!(code, 1);
        assert!(err.contains("U7010"), "{err}");
        assert_eq!(std::fs::read_dir(&away).unwrap().count(), 0);
    }
}

/// Rule 31, structurally: the crate that loads packages, and the code of `una` that loads them
/// and runs `una ita`, name no networking API and start no program, and the only files of the
/// command-line crate that name the HTTP client are those that configure the `Http`
/// capability of a program run with an `http` grant. A change that lets another command open
/// a connection fails here.
#[test]
fn no_command_but_una_ita_opens_a_connection() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let network = [
        "std::net",
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "ToSocketAddrs",
        "una_http",
        "rustls",
        "process::Command",
        "Command::new",
    ];
    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut project = Vec::new();
    sources(&crates.join("una-project/src"), &mut project);
    assert!(project.len() > 10);
    for file in &project {
        let text = std::fs::read_to_string(file).unwrap();
        for word in network {
            assert!(!text.contains(word), "{} names {word}", file.display());
        }
    }
    let manifest = std::fs::read_to_string(crates.join("una-project/Cargo.toml")).unwrap();
    for word in ["una-http", "rustls", "webpki"] {
        assert!(!manifest.contains(word), "una-project depends on {word}");
    }
    for file in [
        "package.rs",
        "ita.rs",
        "migrate.rs",
        "lsp.rs",
        "verification.rs",
    ] {
        let text = std::fs::read_to_string(crates.join("una-cli/src").join(file)).unwrap();
        for word in network {
            assert!(!text.contains(word), "una-cli/src/{file} names {word}");
        }
    }
    let mut cli = Vec::new();
    sources(&crates.join("una-cli/src"), &mut cli);
    let mut naming: Vec<String> = cli
        .iter()
        .filter(|file| {
            let text = std::fs::read_to_string(file).unwrap();
            text.contains("una_http") || text.contains("std::net") || text.contains("TcpStream")
        })
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    naming.sort();
    assert_eq!(naming, ["args.rs", "lib.rs", "runtime.rs"]);
}

/// Runs the `una` binary in `dir`, as `una ita add` and `update` work on the package in the
/// current directory: the exit code, stdout and stderr.
fn una_at(dir: &Path, args: &[&str]) -> (u8, String, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_una"))
        .current_dir(dir)
        .args(args)
        .env_remove("UNA_ITAOCA")
        .output()
        .unwrap();
    (
        u8::try_from(output.status.code().unwrap()).unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn a_changed_dependency_is_reviewed_and_dry_run_writes_nothing() {
    let dir = with_dependency("review", "    may Console[print]\n");
    let (code, _, _) = una(&[
        "una",
        "ita",
        "lock",
        "--accept-authority",
        "util",
        path(&dir),
    ]);
    assert_eq!(code, 0);
    std::fs::write(
        dir.join("deps/util/ita.una"),
        "ita util\n  version 0.3.0\n  edition 0.4\n",
    )
    .unwrap();
    let before = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let (code, out, err) = una(&["una", "ita", "lock", "--dry-run", path(&dir)]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("change util 0.2.0 -> 0.3.0 (path deps/util): digest sha256:")
            && out.contains(" -> sha256:")
            && out.ends_with(
                "; may Console[print]; uses Console[print]\n--dry-run: nothing written\n"
            ),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.lock")).unwrap(),
        before
    );
    let (code, out, _) = una(&["una", "--json", "ita", "lock", "--dry-run", path(&dir)]);
    assert_eq!(code, 0);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(out.starts_with("{\"schema\":\"una.ita/1\""), "{out}");
    assert_eq!(document["written"], serde_json::json!([]));
    let change = &document["changes"][0];
    assert_eq!(change["change"], "change");
    assert_eq!(change["package"], "util");
    assert_eq!(change["origin"], "path deps/util");
    assert_eq!(change["version_from"], "0.2.0");
    assert_eq!(change["version_to"], "0.3.0");
    assert_eq!(change["authority"], serde_json::json!(["Console[print]"]));
    let (code, out, _) = una(&["una", "--json", "ita", "lock", path(&dir)]);
    assert_eq!(code, 0);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["written"], serde_json::json!(["ita.lock"]));
    assert!(
        std::fs::read_to_string(dir.join("ita.lock"))
            .unwrap()
            .contains("\npackage util 0.3.0\n")
    );
}

#[test]
fn an_avoided_version_with_no_later_release_has_no_resolution() {
    let dir = with_dependency("avoid", "    avoid 0.2.0\n");
    let (code, _, err) = una(&["una", "ita", "lock", path(&dir)]);
    assert_eq!(code, 1);
    assert!(
        err.contains("U7013") && err.contains("no later release qualifies"),
        "{err}"
    );
    assert!(!dir.join("ita.lock").exists());
}

#[test]
fn update_locks_path_packages_again_and_refuses_registry_needs() {
    let dir = with_dependency("update", "");
    let (code, out, err) = una_at(&dir, &["ita", "update"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.ends_with("wrote ita.lock\n"), "{out}");
    assert!(
        std::fs::read_to_string(dir.join("ita.lock"))
            .unwrap()
            .contains("\npackage util 0.2.0\n")
    );
    let (code, out, _) = una_at(&dir, &["ita", "update", "util"]);
    assert_eq!((code, out.as_str()), (0, "ita.lock is up to date\n"));
    let (code, _, err) = una_at(&dir, &["ita", "update", "nothing"]);
    assert_eq!(code, 2);
    assert!(err.contains("does not need `nothing`"), "{err}");
    // `--cooldown` and `--now` are one choice.
    let (code, _, _) = una_at(&dir, &["ita", "update", "--now", "--cooldown", "1"]);
    assert_eq!(code, 2);
    let registry = package(
        "update-registry",
        &format!("{MANIFEST}\nneeds\n  json 2.0\n"),
    );
    let (code, out, _) = una_at(&registry, &["--json", "ita", "update", "json"]);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["command"], "update");
    assert_eq!(document["diagnostics"][0]["code"], "U7013");
    assert!(!registry.join("ita.lock").exists());
}

/// The line of the first U7013 of a `una.ita/1` document.
fn u7013_line(out: &str) -> u64 {
    let document: serde_json::Value = serde_json::from_str(out).unwrap();
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "U7013")
        .map(|d| d["primary"]["start"]["line"].as_u64().unwrap())
        .unwrap()
}

#[test]
fn add_writes_nothing_when_the_need_has_no_resolution() {
    let dir = package("add", MANIFEST);
    // A registry need: no resolution offline, so neither file is written. The need is read
    // where `add` puts it: a new `needs` section after the head.
    let (code, out, _) = una_at(&dir, &["--json", "ita", "add", "json", "2.1"]);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["command"], "add");
    assert_eq!(document["written"], serde_json::json!([]));
    assert_eq!(u7013_line(&out), 6);
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        MANIFEST
    );
    assert!(!dir.join("ita.lock").exists());
    // At the end of an existing `needs` section, and before a section that follows `needs`.
    let needs = with_dependency("add-needs", "");
    let (_, out, _) = una_at(
        &needs,
        &["--json", "ita", "add", "acme/json", "2.1", "--as", "js"],
    );
    assert_eq!(u7013_line(&out), 8);
    let before = package(
        "add-before",
        &format!("{MANIFEST}\ntrust\n  util modules keys\n"),
    );
    let (_, out, _) = una_at(&before, &["--json", "ita", "add", "json", "2.1"]);
    assert_eq!(u7013_line(&out), 6);
    // With no version, the newest release cannot be found offline.
    let (code, _, err) = una_at(&dir, &["ita", "add", "json"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("U7013") && err.contains("newest release"),
        "{err}"
    );
    // Every word is checked before it is written.
    for args in [
        ["ita", "add", "Json", "2.1"],
        ["ita", "add", "json", "2.1\ntrust"],
        ["ita", "add", "json", "two"],
        ["ita", "add", "json/", "2.1"],
    ] {
        let (code, _, _) = una_at(&dir, &args);
        assert_eq!(code, 2, "{args:?}");
    }
    let (code, _, _) = una_at(&dir, &["ita", "add", "json", "2.1", "--from", "x y"]);
    assert_eq!(code, 2);
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        MANIFEST
    );
}

#[test]
fn verify_checks_each_registry_package_wherever_it_is() {
    let (dir, itaoca) = with_registry("verify", true);
    // Every copy matches the lock, and still nothing is reported verified: the owner's key
    // chain is checked when a release is read but not stored beside the package, so it is
    // unavailable (U7013), exit code 1.
    let (code, out, err) = una_in(&["una", "ita", "verify", path(&dir)], &itaoca);
    assert_eq!((code, out.as_str()), (1, ""), "{err}");
    assert!(
        err.contains("U7013")
            && err.contains("`acme/ledger 0.3.1` is not verified")
            && err.contains("in Itaoca match the lock")
            && err.contains("key chain"),
        "{err}"
    );
    let (code, _, _) = una_in(&["una", "ita", "vendor", path(&dir)], &itaoca);
    assert_eq!(code, 0);
    let (code, out, _) = una_in(&["una", "--json", "ita", "verify", path(&dir)], &itaoca);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["diagnostics"][0]["code"], "U7013");
    assert!(
        document["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("in vendor/ita and Itaoca match the lock"),
        "{out}"
    );
    // A vendored file changed: U7010, though Itaoca's copy verifies.
    std::fs::write(
        dir.join("vendor/ita/acme/acme+ledger-0.3.1/src/ledger.una"),
        "module ledger\n\npub total() -> Int = 4\n",
    )
    .unwrap();
    let (code, out, _) = una_in(&["una", "--json", "ita", "verify", path(&dir)], &itaoca);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(document["command"], "verify");
    assert_eq!(document["diagnostics"][0]["code"], "U7010");
    // In neither place: U7011.
    let (dir, itaoca) = with_registry("verify-absent", false);
    let (code, _, err) = una_in(&["una", "ita", "verify", path(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7011"), "{err}");
    // With no registry package, there is nothing to verify.
    let plain = package("verify-plain", MANIFEST);
    let (code, _, _) = una(&["una", "ita", "lock", path(&plain)]);
    assert_eq!(code, 0);
    let (code, out, _) = una(&["una", "ita", "verify", path(&plain)]);
    assert_eq!((code, out.as_str()), (0, "no registry package is locked\n"));
}
