//! Project files and package discovery (`spec/13 §7`, `spec/12 §9`, ADR 0021).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use una_project::{
    DEFAULT_SOLVER_VERSION, LoadError, Lock, MAX_MEMORY_CAP, Profile, Role, discover, is_grant,
    is_module_path, parse_lock, parse_manifest,
};
use una_syntax::Unread;

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
        manifest_errors("[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.6\"\n"),
        [
            "U7002 4:11 edition `0.6` is not supported; the editions are \"0.1\", \"0.2\", \"0.3\", \"0.4\" and \"0.5\""
        ]
    );
    // Edition 0.5, as 0.4, is described by `ita.una` (ADR 0122 Question 8.1, ADR 0139).
    let (manifest, errors) =
        parse_manifest("[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.5\"\n");
    assert_eq!(manifest.edition, una_project::Edition::V0_5);
    assert_eq!(
        errors
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>(),
        ["edition 0.5 is described by ita.una, not una.toml; run `una migrate`"]
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
fn serve_grants_are_refused_in_una_toml() {
    for edition in ["0.1", "0.3"] {
        assert_eq!(
            manifest_errors(&format!(
                "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n[run]\ngrants = [\"console\", \"serve:http://127.0.0.1:8080\"]\n"
            )),
            [
                "U7002 6:22 a `serve:` grant needs edition 0.4, whose grants are written in ita.una; una.toml cannot hold one"
            ]
        );
    }
    assert!(!is_grant("serve:http://127.0.0.1:8080"));
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

/// `ita.una` and `ita.lock` are read within their bounds (`spec/30 §1` rule 4, #2282): a file
/// past its bound is left unread, its bytes never held, and keeps its length, which its reader
/// refuses; a file within it is read whole. The bound is on the bytes, so `\r\n` counts two.
#[test]
fn project_files_are_read_within_their_bounds() {
    let root = scratch("discover-bounds");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.una"), "module a\n").unwrap();
    let manifest = "ita a\r\n  version 0.1.0\r\n  edition 0.4\r\n".to_owned()
        + &"-- x\r\n".repeat((1 << 20) / 6 + 1);
    std::fs::write(root.join("ita.una"), &manifest).unwrap();
    // A sparse lock of 32 MiB, none of it written.
    let lock = std::fs::File::create(root.join("ita.lock")).unwrap();
    lock.set_len(32 << 20).unwrap();
    let loaded = discover(&root).unwrap();
    let sizes: Vec<(&str, usize, Option<Unread>)> = loaded
        .files
        .iter()
        .map(|file| (file.name.as_str(), file.bytes.len(), file.unread))
        .collect();
    assert_eq!(
        sizes,
        [
            ("ita.una", 0, Some(Unread::Holds(manifest.len()))),
            ("ita.lock", 0, Some(Unread::Holds(32 << 20))),
            ("src/a.una", 9, None),
        ]
    );
    let decoded = loaded.files[0].decode();
    assert_eq!(decoded.raw_len(), manifest.len());
    let (_, errors) = una_project::ita::parse_ita_manifest(&decoded);
    assert_eq!(
        errors[0].message,
        format!(
            "the file holds {} bytes, more than the 1048576 a manifest may hold",
            manifest.len()
        )
    );
    let file =
        una_project::read_project_file(&root.join("ita.lock"), "ita.lock", 16 << 20).unwrap();
    assert_eq!(file.text(), "");
    assert_eq!(file.unread_len(), Some(Unread::Holds(32 << 20)));
    // Within the bound, the file is read whole and decoded once.
    std::fs::write(
        root.join("ita.una"),
        "ita a\r\n  version 0.1.0\r\n  edition 0.4\r\n",
    )
    .unwrap();
    let file = una_project::read_project_file(&root.join("ita.una"), "ita.una", 1 << 20).unwrap();
    assert_eq!(file.text(), "ita a\n  version 0.1.0\n  edition 0.4\n");
    assert_eq!(file.raw_len(), 39);
    assert_eq!(file.unread_len(), None);
    assert_eq!(una_project::ita::parse_ita_manifest(&file).1, vec![]);
}

/// A manifest of exactly its bound on disk, `\r\n` line ends and all, is read whole, by
/// [`discover`] and by `read_project_file`; one byte more is past it (#2310).
#[test]
fn a_project_file_of_exactly_its_bound_on_disk_is_read() {
    let root = scratch("discover-exact-bound");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.una"), "module a\n").unwrap();
    let mut manifest = "ita a\r\n  version 0.1.0\r\n  edition 0.4\r\n".to_owned();
    while manifest.len() + 6 <= 1 << 20 {
        manifest.push_str("-- x\r\n");
    }
    manifest.push_str(&"-".repeat((1 << 20) - manifest.len()));
    assert_eq!(manifest.len(), 1 << 20);
    std::fs::write(root.join("ita.una"), &manifest).unwrap();
    let loaded = discover(&root).unwrap();
    assert_eq!(loaded.files[0].unread, None);
    assert_eq!(loaded.files[0].bytes.len(), 1 << 20);
    let file = una_project::read_project_file(&root.join("ita.una"), "ita.una", 1 << 20).unwrap();
    assert_eq!((file.unread_len(), file.raw_len()), (None, 1 << 20));
    let (_, errors) = una_project::ita::parse_ita_manifest(&file);
    assert!(
        errors.iter().all(|error| !error.message.contains("bytes")),
        "{errors:?}"
    );
    manifest.push('-');
    std::fs::write(root.join("ita.una"), &manifest).unwrap();
    let file = una_project::read_project_file(&root.join("ita.una"), "ita.una", 1 << 20).unwrap();
    assert_eq!(file.unread_len(), Some(Unread::Holds((1 << 20) + 1)));
}

/// A project file that is not a regular file is refused before anything is read, and the open
/// never blocks: a link to `/dev/zero`, which would never end, a FIFO with no writer, which
/// would block the open, and a directory (#2310, #2313). A link to a regular file is read.
#[cfg(unix)]
#[test]
fn a_project_file_that_is_not_a_regular_file_is_refused_before_it_is_read() {
    let root = scratch("project-file-kinds");
    let refused = |path: &Path| {
        let (done, wait) = std::sync::mpsc::channel();
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let _ = done.send(una_project::read_project_file(&path, "ita.una", 1 << 20));
        });
        let read = wait
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the read blocked");
        let error = read.expect_err("a file that is not regular was read");
        assert!(
            error.to_string().contains("is not a regular file"),
            "{error}"
        );
    };
    std::os::unix::fs::symlink("/dev/zero", root.join("zero")).unwrap();
    refused(&root.join("zero"));
    let fifo = root.join("fifo");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    refused(&fifo);
    refused(&root);
    std::fs::write(root.join("real"), "ita a\n").unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
    let file = una_project::read_project_file(&root.join("link"), "ita.una", 1 << 20).unwrap();
    assert_eq!(file.text(), "ita a\n");
}

/// The open flags of `tree` are known on the platforms this is tested on, and they are the ones
/// that keep an open from blocking: a FIFO with no writer, which a plain open waits on, opens
/// at once with them; a project file read there is refused as not a regular file (#2350).
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn opening_a_fifo_with_the_open_flags_never_waits() {
    use std::os::unix::fs::OpenOptionsExt;
    let flags = una_project::tree::OPEN_FLAGS;
    assert_ne!(flags, 0);
    let root = scratch("fifo-open-flags");
    let fifo = root.join("ita.una");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let opened = |flags: i32| {
        let (done, wait) = std::sync::mpsc::channel();
        let fifo = fifo.clone();
        std::thread::spawn(move || {
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(flags)
                .open(&fifo);
            let _ = done.send(opened.is_ok());
        });
        wait.recv_timeout(std::time::Duration::from_millis(500))
    };
    assert_eq!(opened(flags), Ok(true), "the open waited");
    // Without them the open waits for a writer; one opened here releases it.
    assert!(opened(0).is_err(), "a plain open of a FIFO did not wait");
    drop(std::fs::OpenOptions::new().write(true).open(&fifo).unwrap());
    let error = una_project::read_project_file(&fifo, "ita.una", 1 << 20).unwrap_err();
    assert!(
        error.to_string().contains("is not a regular file"),
        "{error}"
    );
}

/// An `ita.lock` that is there but is not a regular file, a FIFO with no writer, a link to a
/// device, or a directory, refuses the package as not a regular file, before anything is read
/// and without blocking, rather than leave it with no lock (#2348). A missing lock, and a link
/// that names nothing, are no lock; a link to a regular lock is read.
#[cfg(unix)]
#[test]
fn an_ita_lock_that_is_not_a_regular_file_is_refused_not_missing() {
    let root = scratch("discover-lock-kinds");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.una"), "module a\n").unwrap();
    std::fs::write(
        root.join("ita.una"),
        "ita a\n  version 0.1.0\n  edition 0.4\n",
    )
    .unwrap();
    let lock = root.join("ita.lock");
    let discovered = |root: &Path| {
        let (done, wait) = std::sync::mpsc::channel();
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let _ = done.send(discover(&root));
        });
        wait.recv_timeout(std::time::Duration::from_secs(10))
            .expect("discover blocked")
    };
    let has_lock =
        |loaded: &una_project::Loaded| loaded.files.iter().any(|file| file.role == Role::Lock);
    // No lock: the package has none.
    assert!(!has_lock(&discovered(&root).unwrap()));
    let refused = |root: &Path| match discovered(root) {
        Err(LoadError::Unreadable { path, reason }) => {
            assert_eq!(path, root.join("ita.lock"));
            assert!(reason.contains("is not a regular file"), "{reason}");
        }
        other => panic!("a lock that is not a regular file was not refused: {other:?}"),
    };
    let made = std::process::Command::new("mkfifo")
        .arg(&lock)
        .status()
        .unwrap();
    assert!(made.success());
    refused(&root);
    std::fs::remove_file(&lock).unwrap();
    std::os::unix::fs::symlink("/dev/null", &lock).unwrap();
    refused(&root);
    std::fs::remove_file(&lock).unwrap();
    std::fs::create_dir(&lock).unwrap();
    refused(&root);
    std::fs::remove_dir(&lock).unwrap();
    // A link that names nothing is no lock.
    std::os::unix::fs::symlink(root.join("gone"), &lock).unwrap();
    assert!(!has_lock(&discovered(&root).unwrap()));
    std::fs::remove_file(&lock).unwrap();
    // A link to a regular lock is read.
    std::fs::write(root.join("real.lock"), "lock\n").unwrap();
    std::os::unix::fs::symlink(root.join("real.lock"), &lock).unwrap();
    let loaded = discovered(&root).unwrap();
    let read = loaded.files.iter().find(|file| file.role == Role::Lock);
    assert_eq!(read.map(|file| file.bytes.as_slice()), Some(&b"lock\n"[..]));
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

/// A manifest of `edition` with `package` as line 5, under `[package]`, and `run` as line 7,
/// under `[run]`.
fn profile_manifest(edition: &str, package: &str, run: &str) -> String {
    format!(
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n{package}\n[run]\n{run}\n"
    )
}

#[test]
fn a_package_declares_its_profile_in_edition_0_3() {
    // ADR 0109 rule 1: `app` by default, `systems` declared; any other value is U7002.
    let (manifest, errors) = parse_manifest(&profile_manifest("0.3", "", ""));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(manifest.profile, Profile::App);
    assert_eq!(manifest.max_memory, None);
    let text = profile_manifest("0.3", "profile = \"systems\"", "max_memory = 256");
    let (manifest, errors) = parse_manifest(&text);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(manifest.profile, Profile::Systems);
    assert_eq!(manifest.max_memory, Some(256));
    let text = profile_manifest("0.3", "profile = \"app\"", "");
    assert_eq!(parse_manifest(&text).0.profile, Profile::App);
    assert_eq!(
        manifest_errors(&profile_manifest("0.3", "profile = \"kernel\"", "")),
        ["U7002 5:11 unknown profile `kernel`; the profiles are \"app\" and \"systems\""]
    );
    assert_eq!(Profile::parse("systems"), Some(Profile::Systems));
    assert_eq!(Profile::Systems.name(), "systems");
    assert_eq!(Profile::App.name(), "app");
}

#[test]
fn the_profile_and_the_budget_are_unknown_before_edition_0_3() {
    // ADR 0109 rules 1 and 4: editions 0.1 and 0.2 keep both keys unknown, and their packages
    // are `app`, with no declared budget.
    for edition in ["0.1", "0.2"] {
        let text = profile_manifest(edition, "profile = \"systems\"", "max_memory = 256");
        assert_eq!(
            manifest_errors(&text),
            [
                "U7002 5:1 unknown key `profile` in [package]",
                "U7002 7:1 unknown key `max_memory` in [run]"
            ],
            "{edition}"
        );
        let (manifest, _) = parse_manifest(&text);
        assert_eq!(manifest.profile, Profile::App);
        assert_eq!(manifest.max_memory, None);
    }
}

#[test]
fn a_declared_budget_is_from_1_to_the_cap() {
    // ADR 0109 rule 4: from 1 to 4096 MiB while edition 0.3 is a preview.
    assert_eq!(MAX_MEMORY_CAP, 4096);
    for good in [1, 4096] {
        let text = profile_manifest("0.3", "", &format!("max_memory = {good}"));
        let (manifest, errors) = parse_manifest(&text);
        assert!(errors.is_empty(), "{good}: {errors:?}");
        assert_eq!(manifest.max_memory, Some(good));
    }
    for bad in ["0", "4097", "-1"] {
        assert_eq!(
            manifest_errors(&profile_manifest("0.3", "", &format!("max_memory = {bad}"))),
            [format!(
                "U7002 7:14 [run] max_memory is a number of MiB from 1 to 4096, not {bad}"
            )]
        );
    }
    let errors = manifest_errors(&profile_manifest("0.3", "", "max_memory = \"256\""));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("U7002 7:14"), "{errors:?}");
}

#[test]
fn a_systems_package_that_runs_declares_its_budget() {
    // ADR 0109 rule 4: `systems`, an entry and no budget is U7002 at the entry; without an
    // entry, or as `app`, no budget is needed.
    assert_eq!(
        manifest_errors(&profile_manifest(
            "0.3",
            "profile = \"systems\"",
            "entry = \"app.main\""
        )),
        [
            "U7002 7:9 a `systems` package with a [run] entry declares [run] max_memory, its memory budget in MiB"
        ]
    );
    // A budget out of range is reported at its value only, not again as missing.
    assert_eq!(
        manifest_errors(&profile_manifest(
            "0.3",
            "profile = \"systems\"",
            "entry = \"app.main\"\nmax_memory = 0"
        )),
        ["U7002 8:14 [run] max_memory is a number of MiB from 1 to 4096, not 0"]
    );
    for (package, run) in [
        (
            "profile = \"systems\"",
            "entry = \"app.main\"\nmax_memory = 64",
        ),
        ("profile = \"systems\"", ""),
        ("", "entry = \"app.main\""),
        ("profile = \"app\"", "entry = \"app.main\"\nmax_memory = 64"),
    ] {
        let (_, errors) = parse_manifest(&profile_manifest("0.3", package, run));
        assert!(errors.is_empty(), "{package} {run}: {errors:?}");
    }
}

#[test]
fn a_listed_empty_grants_is_told_from_no_grants() {
    // #925: `grants = []` is a final set that grants nothing, which `una check` and `una build`
    // check `main` against; a `[run]` without `grants` lists none.
    let (listed, errors) = parse_manifest(&format!(
        "{PACKAGE}\n[run]\nentry = \"main\"\ngrants = []\n"
    ));
    assert_eq!(errors, vec![]);
    assert!(listed.grants_listed);
    assert!(listed.grants.is_empty());
    let (absent, errors) = parse_manifest(&format!("{PACKAGE}\n[run]\nentry = \"main\"\n"));
    assert_eq!(errors, vec![]);
    assert!(!absent.grants_listed);
    let (some, _) = parse_manifest(&format!("{PACKAGE}\n[run]\ngrants = [\"console\"]\n"));
    assert!(some.grants_listed);
    assert!(!parse_manifest(PACKAGE).0.grants_listed);
}
