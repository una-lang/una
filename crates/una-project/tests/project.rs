//! Project files and package discovery (`spec/13 §7`, `spec/12 §9`, ADR 0021).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use una_project::{
    DEFAULT_SOLVER_VERSION, LoadError, Lock, Role, discover, is_grant, is_module_path, parse_lock,
    parse_manifest,
};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The errors of `text` as "code line:col message", in output order (`spec/13 §1`).
fn located(text: &str, errors: &[una_project::ProjectError]) -> Vec<String> {
    let mut errors = errors.to_vec();
    errors.sort_by_key(|error| (error.range.start, error.code));
    errors
        .iter()
        .map(|error| {
            let before = &text[..error.range.start];
            let line = before.matches('\n').count() + 1;
            let col = before.rsplit('\n').next().unwrap().chars().count() + 1;
            format!("{} {line}:{col} {}", error.code, error.message)
        })
        .collect()
}

fn manifest_errors(text: &str) -> Vec<String> {
    located(text, &parse_manifest(text).1)
}

fn lock_errors(text: &str) -> Vec<String> {
    located(text, &parse_lock(text).1)
}

const PACKAGE: &str = "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

#[test]
fn the_specification_example_and_the_examples_are_valid() {
    let spec = "[package]\nname = \"events-example\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n\n[labels]\ncategories = [\"PII\", \"Location\"]\n\n[trusted]\nmodules = [\"events.keys\"]\n\n[escape_budget]\nchecked_per_module = 1        # default 0\n\n[run]\nentry = \"events.main\"\ngrants = [\"console\"]\n";
    let (manifest, errors) = parse_manifest(spec);
    assert_eq!(errors, vec![]);
    assert_eq!(manifest.checked_per_module, 1);
    assert_eq!(manifest.trusted[0].value, "events.keys");
    assert_eq!(manifest.entry.as_ref().unwrap().value, "events.main");
    let categories: Vec<&str> = manifest
        .categories
        .iter()
        .map(|c| c.value.as_str())
        .collect();
    assert_eq!(categories, ["PII", "Location"]);
    for example in ["examples/events/una.toml", "examples/hello/una.toml"] {
        let text = std::fs::read_to_string(repository().join(example)).unwrap();
        assert_eq!(parse_manifest(&text).1, vec![], "{example}");
    }
}

#[test]
fn manifest_errors_are_located() {
    assert_eq!(
        manifest_errors(&format!("{PACKAGE}license = \"MIT\"\n")),
        ["U7002 5:1 unknown key `license` in [package]"]
    );
    assert_eq!(
        manifest_errors(&format!("{PACKAGE}[tests]\nfast = true\n")),
        ["U7002 5:2 unknown table [tests] in una.toml"]
    );
    assert_eq!(
        manifest_errors("[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.5\"\n"),
        [
            "U7002 4:11 edition `0.5` is not supported; the editions are \"0.1\", \"0.2\", \"0.3\" and \"0.4\""
        ]
    );
    // Edition 0.2 opens the constructs of spec/17 and spec/18 (ADR 0056, ADR 0057).
    assert!(
        manifest_errors("[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n")
            .is_empty()
    );
    assert_eq!(
        manifest_errors("[package]\nname = \"\"\nedition = \"0.1\"\n"),
        [
            "U7002 1:2 [package] has no `version`",
            "U7002 2:8 `name` must not be empty",
        ]
    );
    assert_eq!(
        manifest_errors("[run]\nentry = \"main\"\n"),
        ["U7002 1:1 una.toml has no [package] table"]
    );
    assert_eq!(
        manifest_errors(&format!(
            "{PACKAGE}[labels]\ncategories = [\"PII\", \"health_data\", \"PII\"]\n"
        )),
        [
            "U1005 6:22 label category `health_data` must be UpperCamelCase, as in `HealthData`",
            "U1002 6:37 label category `PII` is declared twice",
        ]
    );
    assert_eq!(
        manifest_errors(&format!(
            "{PACKAGE}[trusted]\nmodules = [\"events keys\", 3]\n"
        )),
        [
            "U7002 6:12 `events keys` is not a module path",
            "U7002 6:27 `modules` must be a string, not an integer",
        ]
    );
    assert_eq!(
        manifest_errors(&format!(
            "{PACKAGE}[escape_budget]\nchecked_per_module = -1\n"
        )),
        ["U7002 6:22 `checked_per_module` must be 0 or more"]
    );
    assert_eq!(
        manifest_errors(&format!(
            "{PACKAGE}[run]\nentry = \"Main\"\ngrants = [\"network\"]\n"
        )),
        [
            "U7002 6:9 `Main` is not a module path",
            "U7002 7:11 `network` is not a grant; grants are console, clock, env, keys:<file>, files:<path>:<access> and http:<origin>[:<methods>]",
        ]
    );
    assert_eq!(
        manifest_errors("[package]\nname = \"p\n"),
        ["U7002 2:10 invalid TOML: invalid basic string, expected `\"`"]
    );
    assert_eq!(
        manifest_errors("package = 1\n"),
        [
            "U7002 1:1 una.toml has no [package] table",
            "U7002 1:11 `package` must be a table, not an integer",
        ]
    );
}

#[test]
fn grants_and_module_paths() {
    for grant in [
        "console",
        "clock",
        "env",
        "keys:./keys.toml",
        "files:data:read",
        "files:out:write",
        "files:/tmp/x:read,write",
    ] {
        assert!(is_grant(grant), "{grant}");
    }
    for grant in [
        "network",
        "keys:",
        "files:data",
        "files::read",
        "files:data:execute",
    ] {
        assert!(!is_grant(grant), "{grant}");
    }
    for path in ["a", "events.keys", "a1.b_2"] {
        assert!(is_module_path(path), "{path}");
    }
    for path in ["", "A", "a..b", "a.", ".a", "a-b", "1a"] {
        assert!(!is_module_path(path), "{path}");
    }
}

#[test]
fn the_repository_lock_is_the_default_lock() {
    let text = std::fs::read_to_string(repository().join("verification.lock")).unwrap();
    let (lock, errors) = parse_lock(&text);
    assert_eq!(errors, vec![]);
    assert_eq!(lock, Lock::default());
    assert_eq!(lock.solver_version, DEFAULT_SOLVER_VERSION);
}

#[test]
fn lock_errors_are_located() {
    let valid = "encoding_version = \"una-smt-1\"\nsolver = \"z3\"\nsolver_version = \"5.1.0\"\nlogic = \"QF_UFLIA\"\nstep_budget = 5000000\nrandom_seed = 0\n";
    assert_eq!(lock_errors(valid), Vec::<String>::new());
    assert_eq!(
        lock_errors(&valid.replace("una-smt-1", "una-smt-0")),
        ["U7004 1:20 `encoding_version` must be \"una-smt-1\" in v0.1, not \"una-smt-0\""]
    );
    assert_eq!(
        lock_errors(&valid.replace("step_budget = 5000000", "step_budget = 0")),
        ["U7004 5:15 `step_budget` must be 1 or more"]
    );
    assert_eq!(
        lock_errors(&valid.replace("random_seed = 0\n", "seed = 0\n")),
        [
            "U7004 1:1 verification.lock has no `random_seed`",
            "U7004 6:1 unknown key `seed` in verification.lock",
        ]
    );
    assert_eq!(
        lock_errors(&valid.replace("\"5.1.0\"", "5")),
        ["U7004 3:18 `solver_version` must be a string, not an integer"]
    );
}

/// A fresh directory for a discovery test.
fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn a_package_is_a_directory_with_una_toml_and_src() {
    let root = scratch("discover-package");
    std::fs::create_dir_all(root.join("src/a/.cache")).unwrap();
    for (path, text) in [
        ("una.toml", PACKAGE),
        ("verification.lock", "x"),
        ("src/b.una", "module b\n"),
        ("src/a/c.una", "module a.c\n"),
        ("src/a/.#c.una", "lock"),
        ("src/a/.cache/d.una", "hidden"),
        ("src/notes.txt", "not a module"),
    ] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let loaded = discover(&root).unwrap();
    assert!(!loaded.single);
    let files: Vec<(&str, Role)> = loaded
        .files
        .iter()
        .map(|file| (file.name.as_str(), file.role))
        .collect();
    assert_eq!(
        files,
        [
            ("una.toml", Role::Manifest),
            ("verification.lock", Role::Lock),
            ("src/a/c.una", Role::Module),
            ("src/b.una", Role::Module),
        ]
    );
    std::fs::remove_file(root.join("verification.lock")).unwrap();
    assert_eq!(discover(&root).unwrap().files.len(), 3);
    std::fs::remove_dir_all(root.join("src")).unwrap();
    assert_eq!(
        discover(&root),
        Err(LoadError::NoSourceDirectory(root.clone()))
    );
    std::fs::remove_file(root.join("una.toml")).unwrap();
    assert_eq!(discover(&root), Err(LoadError::NotAPackage(root.clone())));
    let missing = root.join("nowhere");
    assert_eq!(discover(&missing), Err(LoadError::Missing(missing.clone())));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_file_is_a_package_of_one_module() {
    let root = scratch("discover-file");
    let file = root.join("one.una");
    std::fs::write(&file, "module one\n").unwrap();
    let loaded = discover(&file).unwrap();
    assert!(loaded.single);
    assert_eq!(loaded.files.len(), 1);
    assert_eq!(loaded.files[0].name, "one.una");
    assert_eq!(loaded.files[0].role, Role::Module);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn boundaries_are_a_table_of_edition_0_3() {
    // ADR 0093: editions 0.1 and 0.2 keep [boundaries] unknown, and keep no boundary.
    let older = "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[boundaries]\n\"a.b\" = [\"Files[read]\"]\n";
    assert_eq!(
        manifest_errors(older),
        ["U7002 6:2 unknown table [boundaries] in una.toml"]
    );
    assert!(parse_manifest(older).0.boundaries.is_empty());

    let text = "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[boundaries]\n\"a.b\" = [\"Files[read, write]\", \"Clock[now]\"]\n\"c\" = []\n";
    let (manifest, errors) = parse_manifest(text);
    assert_eq!(located(text, &errors), Vec::<String>::new());
    let ceilings: Vec<String> = manifest
        .boundaries
        .iter()
        .map(|b| {
            let ceiling: Vec<String> = b
                .ceiling
                .iter()
                .map(|c| format!("{}[{}]", c.capability, c.ops.join(", ")))
                .collect();
            format!("{} = {}", b.module.value, ceiling.join(" "))
        })
        .collect();
    assert_eq!(ceilings, ["a.b = Files[read, write] Clock[now]", "c = "]);
    // ADR 0093 rule 9: a key covers its module and the modules below it, by whole segments.
    let extents = manifest.extents();
    let key = |module: &str| {
        extents
            .of(module)
            .map(|i| manifest.boundaries[i].module.value.as_str())
    };
    assert_eq!(key("a.b.tiles"), Some("a.b"));
    assert_eq!(key("a.b"), Some("a.b"));
    assert_eq!(key("a.bx"), None);
    assert_eq!(key("a"), None);
    assert_eq!(key("c.d"), Some("c"));
    assert_eq!(key("c"), Some("c"));
}
