//! `una ita` with a registry (ADR 0122 rules 25, 26 and 34 to 40, `spec/30 §14`): a source of
//! `sources` read from a mirror directory that Itaoca's `mirrors.una` lists and verified,
//! resolved into the lock, fetched into Itaoca, and mounted in the program; a source with no
//! place this release can read, a plain `http://` mirror, a tampered mirror skipped for a good
//! one, a key change, and a release that breaks rule 26.

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
    registry.register("acme", &key);
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
/// the registry written as a mirror beside it, as a registry's store keeps its files, and an
/// Itaoca whose `mirrors.una` lists it. Returns the package, the mirror and Itaoca.
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
    list_mirrors(&itaoca, &[&mirror]);
    (dir, mirror, itaoca)
}

/// Writes Itaoca's `mirrors.una`, listing `mirrors` for the source `acme` as `file://` URLs.
fn list_mirrors(itaoca: &Path, mirrors: &[&Path]) {
    let mut text = "mirrors\n  acme\n".to_owned();
    for mirror in mirrors {
        text.push_str(&format!("    \"file://{}\"\n", mirror.display()));
    }
    std::fs::write(itaoca.join("mirrors.una"), text).unwrap();
}

/// Where Itaoca keeps the floors of `registry`: under its identity, from its pin.
fn floor_dir(itaoca: &Path, registry: &TestRegistry) -> PathBuf {
    itaoca
        .join("registry")
        .join(una_project::itaoca::registry_id(&registry.pin()))
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
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
    let (dir, _, itaoca) = project("end-to-end", &registry);
    let (code, out, err) = una_in(
        &["una", "ita", "lock", arg(&dir)],
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
    assert!(floor_dir(&itaoca, &registry).join("checkpoint").is_file());
    // Locked but not fetched: U7011, and nothing is mounted, so the import fails too.
    assert!(codes(&dir, &itaoca).contains(&"U7011".to_owned()));
    // Fetched at the lock's resolution point, verified, into Itaoca.
    let (code, out, err) = una_in(
        &["una", "ita", "fetch", arg(&dir)],
        &itaoca,
    );
    assert_eq!((code, out.as_str()), (0, "fetched acme/ledger 0.3.1\n"), "{err}");
    let (code, out, _) = una_in(
        &["una", "ita", "fetch", arg(&dir)],
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
        &["una", "ita", "lock", "--check", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
}

#[test]
fn a_source_is_read_only_from_its_places_and_never_over_plain_http() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("no-mirror", &registry);
    // No mirror: the source's own URL is https, and this release has no https transport.
    std::fs::remove_file(itaoca.join("mirrors.una")).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(
        err.contains("U7013")
            && err.contains("no https transport")
            && err.contains("mirrors.una"),
        "{err}"
    );
    assert!(!dir.join("ita.lock").exists());
    // A mirror at a plain `http://` URL, another scheme, or a bare path: mirrors.una is refused,
    // and no source is read from a list that is not the one written.
    for refused in [
        "http://mirror.example".to_owned(),
        "ftp://mirror.example".to_owned(),
        mirror.display().to_string(),
    ] {
        std::fs::write(
            itaoca.join("mirrors.una"),
            format!("mirrors\n  acme\n    \"{refused}\"\n"),
        )
        .unwrap();
        let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
        assert_eq!(code, 1, "{refused}");
        assert!(
            err.contains("U7008") && err.contains("mirrors.una, line 3"),
            "{refused}: {err}"
        );
        assert!(!dir.join("ita.lock").exists());
    }
    // `itaty`, whose root is not built into this release, stays unresolved.
    std::fs::write(
        dir.join("ita.una"),
        "ita demo\n  version 0.1.0\n  edition 0.4\n\nneeds\n  json 2.0\n",
    )
    .unwrap();
    std::fs::remove_file(itaoca.join("mirrors.una")).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7013") && err.contains("`itaty`"), "{err}");
}

/// A mirror that serves a copy that does not verify is skipped and named, and the next place
/// whose copy verifies is used.
#[test]
fn a_mirror_with_a_bad_copy_is_skipped_for_one_that_verifies() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("skip-bad", &registry);
    let bad = mirror.with_file_name("bad-mirror");
    registry.write(&bad);
    let latest = bad.join("checkpoint/latest");
    let mut bytes = std::fs::read(&latest).unwrap();
    let at = bytes.len() / 2;
    bytes[at] ^= 0x01;
    std::fs::write(&latest, &bytes).unwrap();
    list_mirrors(&itaoca, &[&bad, &mirror]);
    let (code, out, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("U7010") && out.contains("bad-mirror") && out.contains("skipped"),
        "{out}"
    );
    assert!(dir.join("ita.lock").is_file());
    // With only the bad mirror, nothing verifies: U7010 names it.
    std::fs::remove_file(dir.join("ita.lock")).unwrap();
    list_mirrors(&itaoca, &[&bad]);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010") && err.contains("bad-mirror"), "{err}");
}

#[test]
fn a_tampered_mirror_is_refused_and_nothing_is_stored() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("tampered", &registry);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", arg(&dir)],
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
        &["una", "ita", "fetch", arg(&dir)],
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
        &["una", "ita", "fetch", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 1);
    assert!(err.contains("U7010"), "{err}");
}

#[test]
fn a_change_of_the_owners_keys_needs_accept_key() {
    let (mut registry, key) = acme();
    let (dir, mirror, itaoca) = project("accept-key", &registry);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
    // The owner adds a key after the lock's resolution point.
    let second = keypair(61, "acme.desktop");
    registry.owner_event(
        "acme",
        &format!("add {}", key_text(&second.1)),
        1,
        &[&key, &second],
    );
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", arg(&dir)],
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
    let (dir, _, itaoca) = project("update", &registry);
    let (code, out, err) = una_at(&dir, &["ita", "update"], &itaoca);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("acme/ledger 0.3.2 (acme)"), "{out}");
    let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
    assert!(manifest.contains("  acme/ledger 0.3.2 from acme\n"), "{manifest}");
    // `--now` takes the release within the cooldown.
    let (code, _, err) = una_at(&dir, &["ita", "update", "--now"], &itaoca);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(lock.contains("\npackage acme/ledger 0.3.3\n"), "{lock}");
}

#[test]
fn add_takes_the_newest_release_past_the_cooldown_from_its_owners_source() {
    let (registry, _) = acme();
    let (dir, _, itaoca) = project("add", &registry);
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita demo\n  version 0.1.0\n  edition 0.4\n\nsources\n  acme \"https://ita.acme.example\"\n    root \"{}\"\n    owners acme\n",
            registry.pin()
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/demo.una"), "module demo\n\npub one() -> Int = 1\n").unwrap();
    let (code, _, err) = una_at(&dir, &["ita", "add", "acme/ledger"], &itaoca);
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
    registry.register("acme", &key);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.release("acme/ledger", "0.3.2", "Http[post]", "none", &[&key]);
    registry.checkpoint(now() - 20 * DAY);
    registry.checkpoint(now() - 60);
    let (dir, _, itaoca) = project("rule-26", &registry);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 1);
    assert!(err.contains("U7010") && err.contains("rule 26"), "{err}");
    assert!(!dir.join("ita.lock").exists());
}

/// A recovery of the owner pending at the resolution point is a key change to consent to.
#[test]
fn a_pending_recovery_needs_accept_key() {
    let (mut registry, _) = acme();
    registry.recover("acme", "recover", &keypair(64, "acme.new").1, &[]);
    registry.checkpoint(now() - 30);
    let (dir, _, itaoca) = project("pending", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(
        err.contains("U7014") && err.contains("pending") && err.contains("a recovery logged at"),
        "{err}"
    );
    assert!(!dir.join("ita.lock").exists());
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--accept-key", "acme/ledger", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
}

/// A locked release that counts as signed only with a key compromised before it was logged is
/// not fetched.
#[test]
fn a_release_signed_by_a_compromised_key_is_not_fetched() {
    use una_project::tlog::merkle::{leaf_hash, root as tree_root};
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    let (index, digest) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
    registry.checkpoint(now() - 60);
    let (dir, _, itaoca) = project("compromised", &registry);
    let leaves: Vec<_> = registry.entries.iter().map(|e| leaf_hash(e)).collect();
    let hex: String = tree_root(&leaves)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let signed = una_project::provenance::signed_name("acme/ledger", &a.1.public);
    std::fs::write(
        dir.join("ita.lock"),
        format!(
            "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n\nregistry acme\n  root \"{}\"\n  checkpoint {} \"sha256:{hex}\"\n\npackage acme/ledger 0.3.1\n  from acme\n  digest \"{digest}\"\n  signed \"{signed}\"\n  logged {index}\n",
            registry.pin(),
            leaves.len()
        ),
    )
    .unwrap();
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010") && err.contains("compromised"), "{err}");
    assert!(!itaoca.join("packages").exists());
}

/// An archive whose code does not check is not fetched: what it uses cannot be counted.
#[test]
fn an_archive_that_does_not_check_is_not_fetched() {
    let mut registry = TestRegistry::new("acme");
    let key = keypair(60, "acme.laptop");
    registry.register("acme", &key);
    registry.release_with(
        "acme/ledger",
        "0.3.1",
        "none",
        "none",
        "module ledger\n\npub answer() -> Int = true\n",
        &[&key],
    );
    registry.checkpoint(now() - 20 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(now() - 60);
    let (dir, _, itaoca) = project("does-not-check", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010") && err.contains("does not check"), "{err}");
}

/// Floors are stored as soon as a checkpoint is accepted, even when nothing is written; and a
/// registry whose log does not extend the lock's point is refused, with no floor stored.
#[test]
fn floors_are_stored_on_every_read_and_a_fork_of_the_lock_is_refused() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("floors", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", "--dry-run", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    assert!(!dir.join("ita.lock").exists());
    assert!(floor_dir(&itaoca, &registry).join("checkpoint").is_file());
    assert!(floor_dir(&itaoca, &registry).join("root-version").is_file());
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    // Another log, larger than the lock's point, from the same keys.
    let mut other = TestRegistry::new("acme");
    let key = keypair(60, "acme.laptop");
    other.register("acme", &key);
    other.release("acme/ledger", "0.3.9", "none", "none", &[&key]);
    for _ in 0..4 {
        other.append("other", "filler");
    }
    other.checkpoint(now() - 60);
    let _ = std::fs::remove_dir_all(&mirror);
    other.write(&mirror);
    std::fs::remove_dir_all(itaoca.join("registry")).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010") && err.contains("forked"), "{err}");
}

/// Consent to a pending dispute is recorded in the lock, which `--check` and later locks honor;
/// a new pending entry still needs consent. A dispute logged six days ago is pending.
#[test]
fn consent_to_a_pending_dispute_is_recorded_in_the_lock() {
    let mut registry = TestRegistry::new("acme");
    let key = keypair(60, "acme.laptop");
    registry.register("acme", &key);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(now() - 20 * DAY);
    let dispute = registry.recover("acme", "dispute", &keypair(64, "acme.disputed").1, &[]);
    registry.checkpoint(now() - 6 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(now() - 60);
    let (dir, mirror, itaoca) = project("accepted", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7014") && err.contains("a dispute logged at"), "{err}");
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--accept-key", "acme/ledger", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(
        lock.contains(&format!("\n  accepted acme {dispute}\n")),
        "{lock}"
    );
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!((code, out.as_str()), (0, "ita.lock is up to date\n"), "{err}");
    // Another dispute: not recorded, so it needs consent again.
    registry.recover("acme", "dispute", &keypair(65, "acme.second").1, &[]);
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7014"), "{err}");
}

/// One name of `sources` given to two registries in turn: each keeps its own floors, so the
/// second is neither blocked by the first's larger floor nor stores into it.
#[test]
fn floors_belong_to_a_registry_not_to_its_name() {
    let (first, _) = acme();
    let (dir, mirror, itaoca) = project("one-name", &first);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    // Another registry, other keys, under the same name, with a smaller log.
    let mut second = TestRegistry::with_keys("acme", support::Keys::offset("acme", 100));
    let key = keypair(60, "acme.laptop");
    second.register("acme", &key);
    second.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    second.checkpoint(now() - 60);
    assert_ne!(first.pin(), second.pin());
    let _ = std::fs::remove_dir_all(&mirror);
    second.write(&mirror);
    std::fs::remove_file(dir.join("ita.lock")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita demo\n  version 0.1.0\n  edition 0.4\n\nsources\n  acme \"https://ita.acme.example\"\n    root \"{}\"\n    owners acme\n\nneeds\n  acme/ledger 0.3 from acme\n",
            second.pin()
        ),
    )
    .unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let size = |registry: &TestRegistry| {
        let text = std::fs::read_to_string(floor_dir(&itaoca, registry).join("checkpoint")).unwrap();
        text.lines().nth(1).unwrap().parse::<u64>().unwrap()
    };
    assert_eq!(size(&first), first.entries.len() as u64);
    assert_eq!(size(&second), second.entries.len() as u64);
}

/// An archive whose public function declares `Http[get]` and calls a module of its need, while
/// its statement gives no authority: fetch refuses it. A public function's `uses` are declared
/// and checked, so code that uses a capability always says so in its header.
#[test]
fn an_archive_that_understates_its_authority_is_not_fetched() {
    use una_project::tlog::merkle::{leaf_hash, root as tree_root};
    let mut registry = TestRegistry::new("acme");
    let key = keypair(60, "acme.laptop");
    registry.register("acme", &key);
    let (index, digest) = registry.release_custom(
        "acme/ledger",
        "0.3.1",
        "ita acme/ledger\n  version 0.3.1\n  edition 0.4\n\nneeds\n  acme/http 1.4\n",
        &["acme acme/http 1.4.0"],
        "none",
        "module ledger\n\nimport http.client (get_text)\n\npub page(url: Text) -> Text\n  uses @net: Http[get]\n=\n  get_text(url) with @http = @net[get]\n",
        &[&key],
    );
    registry.checkpoint(now() - 60);
    let (dir, _, itaoca) = project("understates", &registry);
    let leaves: Vec<_> = registry.entries.iter().map(|e| leaf_hash(e)).collect();
    let hex: String = tree_root(&leaves)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let signed = una_project::provenance::signed_name("acme/ledger", &key.1.public);
    std::fs::write(
        dir.join("ita.lock"),
        format!(
            "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n\nregistry acme\n  root \"{}\"\n  checkpoint {} \"sha256:{hex}\"\n\npackage acme/ledger 0.3.1\n  from acme\n  digest \"{digest}\"\n  signed \"{signed}\"\n  logged {index}\n",
            registry.pin(),
            leaves.len()
        ),
    )
    .unwrap();
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010"), "{err}");
    assert!(!itaoca.join("packages").exists());
}

/// An `accepted` index the lock's resolution point cannot have reached, or one that is no
/// entry of the owner's chain there, is U7009 at its line, and consents to nothing.
#[test]
fn a_forged_accepted_index_is_refused() {
    let (registry, _) = acme();
    let (dir, _, itaoca) = project("forged-accepted", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let checkpoint_line = lock
        .lines()
        .find(|line| line.starts_with("  checkpoint "))
        .unwrap()
        .to_owned();
    // 999, a future entry; 1, a release, not an entry of the owner's chain.
    for forged in ["999", "1"] {
        std::fs::write(
            dir.join("ita.lock"),
            lock.replacen(
                &checkpoint_line,
                &format!("{checkpoint_line}\n  accepted acme {forged}"),
                1,
            ),
        )
        .unwrap();
        let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
        assert_eq!(code, 1, "{forged}");
        assert!(err.contains("U7009") && err.contains("accepted index"), "{forged}: {err}");
    }
}
