//! `una ita` with a registry (ADR 0122 rules 25, 26 and 34 to 40, `spec/30 §14`): a source of
//! `sources` read from a mirror directory and verified, resolved into the lock, fetched into
//! Itaoca, and mounted in the program; a source with no mirror, a plain `http://` mirror, a
//! tampered mirror, a key change, and a release that breaks rule 26.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "../../una-project/tests/support/registry.rs"]
mod support;

use std::path::{Path, PathBuf};

use support::{TestRegistry, key_text, keypair};

const DAY: u64 = 24 * 3600;

/// The local clock, as `una ita` reads it.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A fresh directory named `name` below the tests' temporary directory.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("registry")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `una` with Itaoca at `itaoca`: the exit code, stdout and stderr.
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

/// Runs the `una` binary in `dir` with Itaoca at `itaoca`, as `una ita update` and `add` work on
/// the package in the current directory.
fn una_at(dir: &Path, args: &[&str], itaoca: &Path) -> (u8, String, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_una"))
        .current_dir(dir)
        .args(args)
        .env("UNA_ITAOCA", itaoca)
        .output()
        .unwrap();
    (
        u8::try_from(output.status.code().unwrap()).unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// A registry `acme` with the owner `acme` and releases of `acme/ledger`, logged a month ago,
/// and its latest checkpoint cosigned now. Returns it with the owner's key.
fn acme() -> (TestRegistry, (ed25519_dalek::SigningKey, una_project::tlog::note::Key)) {
    let mut registry = TestRegistry::new("acme");
    let key = keypair(60, "acme.laptop");
    registry.register("acme", &key.1);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(now() - 30 * DAY);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&key]);
    registry.checkpoint(now() - 20 * DAY);
    // Another name's entry, so the latest checkpoint is a size of its own.
    registry.append("other", "filler");
    registry.checkpoint(now() - 60);
    (registry, key)
}

/// A package that needs `acme/ledger 0.3` from the registry `acme`, whose module imports it,
/// the registry written as a mirror beside it, and an empty Itaoca. Returns the package, the
/// mirror and Itaoca.
fn project(name: &str, registry: &TestRegistry) -> (PathBuf, PathBuf, PathBuf) {
    let base = scratch(name);
    let dir = base.join("demo");
    let mirror = base.join("mirror");
    let itaoca = base.join("itaoca");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(&itaoca).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita demo\n  version 0.1.0\n  edition 0.4\n\nsources\n  acme \"https://ita.acme.example\"\n    root \"{}\"\n    owners acme\n\nneeds\n  acme/ledger 0.3 from acme\n",
            registry.pin()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/demo.una"),
        "module demo\n\nimport ledger.ledger (answer)\n\npub forty_two() -> Int = answer()\n",
    )
    .unwrap();
    registry.write(&mirror);
    (dir, mirror, itaoca)
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn mirror_flag(mirror: &Path) -> String {
    format!("acme={}", mirror.display())
}

/// The codes of `una --json check` with Itaoca at `itaoca`.
fn codes(dir: &Path, itaoca: &Path) -> Vec<String> {
    let (_, out, _) = una_in(&["una", "--json", "check", arg(dir)], itaoca);
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_registry_need_is_locked_fetched_and_mounted() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("end-to-end", &registry);
    let flag = mirror_flag(&mirror);
    let (code, out, err) = una_in(
        &["una", "ita", "lock", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
    // Minimal version selection: the least release at or above the need, never the newest.
    assert!(
        out.starts_with("add acme/ledger 0.3.1 (acme): digest sha256:"),
        "{out}"
    );
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(lock.contains("\nregistry acme\n  root \""), "{lock}");
    assert!(lock.contains("\npackage acme/ledger 0.3.1\n  from acme\n"), "{lock}");
    assert!(lock.contains("  signed \"ed25519:acme:"), "{lock}");
    assert!(lock.contains("  logged 1\n"), "{lock}");
    // The checkpoint read is stored as the registry's floor.
    assert!(itaoca.join("registry/acme/checkpoint").is_file());
    // Locked but not fetched: U7011, and nothing is mounted, so the import fails too.
    assert!(codes(&dir, &itaoca).contains(&"U7011".to_owned()));
    // Fetched at the lock's resolution point, verified, into Itaoca.
    let (code, out, err) = una_in(
        &["una", "ita", "fetch", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!((code, out.as_str()), (0, "fetched acme/ledger 0.3.1\n"), "{err}");
    let (code, out, _) = una_in(
        &["una", "ita", "fetch", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!((code, out.as_str()), (0, "acme/ledger 0.3.1 is in Itaoca\n"));
    // Mounted: the program checks, and the report names the registry.
    assert_eq!(codes(&dir, &itaoca), Vec::<String>::new());
    let (_, out, _) = una_in(&["una", "--json", "report", arg(&dir)], &itaoca);
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["dependencies"][0]["source"], "acme");
    assert_eq!(report["tcb"]["registries"][0]["source"], "acme");
    assert_eq!(report["tcb"]["registries"][0]["root"], registry.pin());
    // The lock is the one `una ita lock --check` writes at its own resolution point.
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--check", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
}

#[test]
fn a_source_is_read_only_from_a_mirror_and_never_over_plain_http() {
    let (registry, _) = acme();
    let (dir, _, itaoca) = project("no-mirror", &registry);
    // No mirror: the source is at an https URL, and this release has no https transport.
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(
        err.contains("U7013") && err.contains("no https transport"),
        "{err}"
    );
    assert!(!dir.join("ita.lock").exists());
    for refused in [
        "acme=http://mirror.example",
        "acme=ftp://mirror.example",
        "acme",
        "itaty=/srv",
    ] {
        let (code, _, err) = una_in(
            &["una", "ita", "lock", "--mirror", refused, arg(&dir)],
            &itaoca,
        );
        assert_eq!(code, 1, "{refused}");
        assert!(err.contains("U7013"), "{refused}: {err}");
        assert!(!dir.join("ita.lock").exists());
    }
    // `itaty`, whose root is not built into this release, stays unresolved.
    std::fs::write(
        dir.join("ita.una"),
        "ita demo\n  version 0.1.0\n  edition 0.4\n\nneeds\n  json 2.0\n",
    )
    .unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7013") && err.contains("`itaty`"), "{err}");
}

#[test]
fn a_tampered_mirror_is_refused_and_nothing_is_stored() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("tampered", &registry);
    let flag = mirror_flag(&mirror);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
    // One byte of the archive in the mirror changed.
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let digest = lock
        .lines()
        .find_map(|line| line.strip_prefix("  digest \"sha256:"))
        .unwrap()
        .trim_end_matches('"')
        .to_owned();
    let blob = mirror.join("blobs/sha256").join(&digest);
    let mut bytes = std::fs::read(&blob).unwrap();
    let last = bytes.len() - 2;
    bytes[last] ^= 0x01;
    std::fs::write(&blob, &bytes).unwrap();
    let (code, _, err) = una_in(
        &["una", "ita", "fetch", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 1);
    assert!(err.contains("U7010"), "{err}");
    assert!(!itaoca.join("packages").join("sha256").join(&digest).exists());
    // A changed tile: the log's proofs fail before any archive is read.
    registry.write(&mirror);
    let size = registry.entries.len();
    let tile = mirror.join(format!("tile/0/000.p/{size}"));
    let mut bytes = std::fs::read(&tile).unwrap();
    bytes[0] ^= 0x01;
    std::fs::write(&tile, &bytes).unwrap();
    let (code, _, err) = una_in(
        &["una", "ita", "fetch", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 1);
    assert!(err.contains("U7010"), "{err}");
}

#[test]
fn a_change_of_the_owners_keys_needs_accept_key() {
    let (mut registry, key) = acme();
    let (dir, mirror, itaoca) = project("accept-key", &registry);
    let flag = mirror_flag(&mirror);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
    // The owner adds a key after the lock's resolution point.
    let second = keypair(61, "acme.desktop");
    registry.owner_event("acme", "add", &format!("key {}", key_text(&second.1)), &[&key]);
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 1);
    assert!(
        err.contains("U7014") && err.contains("--accept-key acme/ledger"),
        "{err}"
    );
    let (code, _, err) = una_in(
        &[
            "una",
            "ita",
            "lock",
            "--accept-key",
            "acme/ledger",
            "--mirror",
            &flag,
            arg(&dir),
        ],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
}

#[test]
fn update_raises_past_the_cooldown_only() {
    let (mut registry, key) = acme();
    // 0.3.3 is logged a minute ago: within the 72-hour cooldown.
    registry.release("acme/ledger", "0.3.3", "none", "none", &[&key]);
    registry.checkpoint(now() - 60);
    let (dir, mirror, itaoca) = project("update", &registry);
    let flag = mirror_flag(&mirror);
    let (code, out, err) = una_at(&dir, &["ita", "update", "--mirror", &flag], &itaoca);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("acme/ledger 0.3.2 (acme)"), "{out}");
    let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
    assert!(manifest.contains("  acme/ledger 0.3.2 from acme\n"), "{manifest}");
    // `--now` takes the release within the cooldown.
    let (code, _, err) = una_at(&dir, &["ita", "update", "--now", "--mirror", &flag], &itaoca);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(lock.contains("\npackage acme/ledger 0.3.3\n"), "{lock}");
}

#[test]
fn add_takes_the_newest_release_past_the_cooldown_from_its_owners_source() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("add", &registry);
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita demo\n  version 0.1.0\n  edition 0.4\n\nsources\n  acme \"https://ita.acme.example\"\n    root \"{}\"\n    owners acme\n",
            registry.pin()
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/demo.una"), "module demo\n\npub one() -> Int = 1\n").unwrap();
    let flag = mirror_flag(&mirror);
    let (code, _, err) = una_at(&dir, &["ita", "add", "acme/ledger", "--mirror", &flag], &itaoca);
    assert_eq!(code, 0, "{err}");
    let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
    assert!(
        manifest.contains("  acme/ledger 0.3.2 from acme\n"),
        "{manifest}"
    );
}

#[test]
fn a_registry_that_logged_a_breach_of_rule_26_is_refused() {
    let mut registry = TestRegistry::new("acme");
    let key = keypair(60, "acme.laptop");
    registry.register("acme", &key.1);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.release("acme/ledger", "0.3.2", "Http[post]", "none", &[&key]);
    registry.checkpoint(now() - 20 * DAY);
    registry.checkpoint(now() - 60);
    let (dir, mirror, itaoca) = project("rule-26", &registry);
    let flag = mirror_flag(&mirror);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--mirror", &flag, arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 1);
    assert!(err.contains("U7010") && err.contains("rule 26"), "{err}");
    assert!(!dir.join("ita.lock").exists());
}
