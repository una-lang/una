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
fn acme() -> (
    TestRegistry,
    (ed25519_dalek::SigningKey, una_project::tlog::note::Key),
) {
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
    let (code, out, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    // Minimal version selection: the least release at or above the need, never the newest.
    assert!(
        out.starts_with("add acme/ledger 0.3.1 (acme): digest sha256:"),
        "{out}"
    );
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(lock.contains("\nregistry acme\n  root \""), "{lock}");
    assert!(
        lock.contains("\npackage acme/ledger 0.3.1\n  from acme\n"),
        "{lock}"
    );
    assert!(lock.contains("  signed \"ed25519:acme:"), "{lock}");
    assert!(lock.contains("  logged 1\n"), "{lock}");
    // The checkpoint read is stored as the registry's floor.
    assert!(floor_dir(&itaoca, &registry).join("checkpoint").is_file());
    // Locked but not fetched: U7011, and nothing is mounted, so the import fails too.
    assert!(codes(&dir, &itaoca).contains(&"U7011".to_owned()));
    // Fetched at the lock's resolution point, verified, into Itaoca.
    let (code, out, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(
        (code, out.as_str()),
        (0, "fetched acme/ledger 0.3.1\n"),
        "{err}"
    );
    let (code, out, _) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(
        (code, out.as_str()),
        (0, "acme/ledger 0.3.1 is in Itaoca\n")
    );
    // Mounted: the program checks, and the report names the registry.
    assert_eq!(codes(&dir, &itaoca), Vec::<String>::new());
    let (_, out, _) = una_in(&["una", "--json", "report", arg(&dir)], &itaoca);
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["dependencies"][0]["source"], "acme");
    assert_eq!(report["tcb"]["registries"][0]["source"], "acme");
    assert_eq!(report["tcb"]["registries"][0]["root"], registry.pin());
    // The lock is the one `una ita lock --check` writes at its own resolution point.
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
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
        err.contains("U7013") && err.contains("no https transport") && err.contains("mirrors.una"),
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
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
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
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010"), "{err}");
    assert!(
        !itaoca
            .join("packages")
            .join("sha256")
            .join(&digest)
            .exists()
    );
    // A changed tile: the log's proofs fail before any archive is read.
    registry.write(&mirror);
    let size = registry.entries.len();
    let tile = mirror.join(format!("tile/0/000.p/{size}"));
    let mut bytes = std::fs::read(&tile).unwrap();
    bytes[0] ^= 0x01;
    std::fs::write(&tile, &bytes).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010"), "{err}");
}

#[test]
fn a_change_of_the_owners_keys_needs_accept_key() {
    let (mut registry, key) = acme();
    let (dir, mirror, itaoca) = project("accept-key", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    // The owner adds a key after the lock's resolution point.
    let second = keypair(61, "acme.desktop");
    registry.owner_event(
        "acme",
        &format!("add {}", key_text(&second.1)),
        2,
        &[&key, &second],
    );
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
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
    assert!(
        manifest.contains("  acme/ledger 0.3.2 from acme\n"),
        "{manifest}"
    );
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
    std::fs::write(
        dir.join("src/demo.una"),
        "module demo\n\npub one() -> Int = 1\n",
    )
    .unwrap();
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
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(err.contains("U7010") && err.contains("rule 26"), "{err}");
    assert!(!dir.join("ita.lock").exists());
}

/// An owner chain that holds `kind` at `index`, an entry this release does not replay, leaves
/// the owner's names unavailable (U7013), never refused: no lock is written, with or without
/// `--accept-key`.
fn unavailable_with(name: &str, registry: &TestRegistry, kind: &str, index: u64) {
    let (dir, _, itaoca) = project(name, registry);
    for accept in [false, true] {
        let mut args = vec!["una", "ita", "lock"];
        if accept {
            args.extend(["--accept-key", "acme/ledger"]);
        }
        args.push(arg(&dir));
        let (code, _, err) = una_in(&args, &itaoca);
        assert_eq!(code, 1, "{err}");
        assert!(
            err.contains("U7013")
                && !err.contains("U7010")
                && err.contains(&format!("holds {kind}, at log entry {index}"))
                && err.contains("does not replay"),
            "{err}"
        );
        assert!(!dir.join("ita.lock").exists());
    }
}

/// A recovery in the owner's chain: its names are unavailable.
#[test]
fn a_recovery_makes_the_owners_names_unavailable() {
    let (mut registry, _) = acme();
    let index = registry.recover("acme", "recover", &keypair(64, "acme.new").1, &[]);
    registry.checkpoint(now() - 30);
    unavailable_with("recovery", &registry, "a recovery", index);
}

/// A dispute in the owner's chain: its names are unavailable.
#[test]
fn a_dispute_makes_the_owners_names_unavailable() {
    let (mut registry, _) = acme();
    let index = registry.recover("acme", "dispute", &keypair(64, "acme.disputed").1, &[]);
    registry.checkpoint(now() - 30);
    unavailable_with("dispute", &registry, "a dispute", index);
}

/// A co-signer change in the owner's chain, signed by the owner's threshold: its names are
/// unavailable.
#[test]
fn a_cosigner_change_makes_the_owners_names_unavailable() {
    let (mut registry, key) = acme();
    let trustee = keypair(70, "trustee.a");
    let index = registry.owner_event(
        "acme",
        &format!("cosigners 1 {}", key_text(&trustee.1)),
        2,
        &[&key],
    );
    registry.checkpoint(now() - 30);
    unavailable_with("cosigners", &registry, "a co-signer change", index);
}

/// A locked release that counts as signed only with a key compromised before it was logged is
/// not fetched.
#[test]
fn a_release_signed_by_a_compromised_key_is_not_fetched() {
    use una_project::tlog::merkle::{leaf_hash, root as tree_root};
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let e = keypair(62, "acme.e");
    let f = keypair(63, "acme.f");
    let mut registry = TestRegistry::new("acme");
    // Four keys at threshold 2: after the claim against `a`, three clean keys keep the floor.
    registry.register_keys("acme", &[&a, &d, &e, &f], 2, &[], 0);
    let (index, digest) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&a, &d]);
    registry.owner_event("acme", "revoke acme.a since 0", 2, &[&d, &e]);
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
            "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n\nregistry acme\n  root \"{}\"\n  checkpoint {} \"sha256:{hex}\"\n  last root {} \"{}\"\n\npackage acme/ledger 0.3.1\n  from acme\n  digest \"{digest}\"\n  signed \"{signed}\"\n  logged {index}\n",
            registry.pin(),
            leaves.len(),
            registry.roots.len(),
            una_project::tlog::root::digest(registry.roots.last().unwrap())
        ),
    )
    .unwrap();
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 1);
    assert!(
        err.contains("U7010") && err.contains("compromised"),
        "{err}"
    );
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
    assert!(
        err.contains("U7010") && err.contains("does not check"),
        "{err}"
    );
}

/// ADR 0122, amendment of 2026-11-09 (L3): the root-version floor is stored as the version and
/// the digest of that root metadata. A floor whose version the chain holds with another digest
/// is a fork of the root chain: refused (U7010) by the offline load (`una check`), by
/// `una ita lock --check`, which walks the chain online, and by `una ita fetch`. A floor of the
/// earlier form, the version alone, is compared by version, and takes the digest of its version
/// from the next command that walks and verifies the chain at that version (#1970), after which
/// the offline load compares by digest; a floor line of neither form is unreadable (U7013)
/// offline.
#[test]
fn a_root_version_floor_with_another_digest_is_a_fork() {
    let (registry, _) = acme();
    let (dir, _, itaoca) = project("floor-fork", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let floor = floor_dir(&itaoca, &registry).join("root-version");
    let first = una_project::tlog::root::digest(&registry.roots[0]);
    assert_eq!(
        std::fs::read_to_string(&floor).unwrap(),
        format!("1 {first}\n")
    );
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    assert_eq!(codes(&dir, &itaoca), Vec::<String>::new());

    let write = |text: &str| {
        std::fs::remove_file(&floor).unwrap();
        std::fs::write(&floor, text).unwrap();
    };
    write(&format!("1 sha256:{}\n", "0".repeat(64)));
    let found = codes(&dir, &itaoca);
    assert!(
        found.contains(&"U7010".to_owned()) && !found.contains(&"U7013".to_owned()),
        "{found:?}"
    );
    let (_, _, err) = una_in(&["una", "check", arg(&dir)], &itaoca);
    assert!(
        err.contains("another digest") && err.contains("forked") && !err.contains("older than"),
        "{err}"
    );
    for command in ["--check", "fetch"] {
        let args: Vec<&str> = if command == "fetch" {
            vec!["una", "ita", "fetch", arg(&dir)]
        } else {
            vec!["una", "ita", "lock", "--check", arg(&dir)]
        };
        let (code, _, err) = una_in(&args, &itaoca);
        assert_eq!(code, 1, "{command}: {err}");
        assert!(
            err.contains("U7010") && err.contains("forked"),
            "{command}: {err}"
        );
    }

    // The earlier form, as builds before v0.4.0 write it: by version, accepted offline, and
    // given the digest of version 1 by a command that walks and verifies the chain at version 1.
    write("1\n");
    assert_eq!(codes(&dir, &itaoca), Vec::<String>::new());
    assert_eq!(std::fs::read_to_string(&floor).unwrap(), "1\n");
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("not stored"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&floor).unwrap(),
        format!("1 {first}\n")
    );
    assert_eq!(codes(&dir, &itaoca), Vec::<String>::new());

    // Neither form: unreadable offline, never no floor.
    write(&format!("1 {first}x\n"));
    let found = codes(&dir, &itaoca);
    assert!(
        found.contains(&"U7013".to_owned()) && !found.contains(&"U7010".to_owned()),
        "{found:?}"
    );
}

/// A command that loses the root-floor race fails (U7010), it does not end with a note (#2099,
/// `spec/30 §14`, Floors). Another run is played by this test: it holds the floor lock while
/// `una ita lock --check` and `una ita lock` each read the floor of the earlier form, walk the
/// chain and wait for the lock, stores another digest at the floor's version, and lets go. The
/// command then compares under the lock and finds another digest held for version 1: exit 1,
/// U7010 naming the floor file, the other run's floor kept, the checkpoint floor as it was. With the same digest
/// from the other run, the command passes, with no note.
///
/// The other run cannot see when the command has read the floor, so it waits before it stores.
/// On a machine so loaded that the command reads the floor only after that, the command meets
/// the digest in its walk instead, which is the same refusal by another sentence: the test
/// accepts both, and never fails for timing. The store's own comparison is tested without a
/// clock in `una-project` (`a_store_that_finds_another_digest_under_the_lock_is_a_fork`,
/// `unbundling_fails_when_a_run_at_the_same_time_stored_a_forked_root`).
#[test]
fn a_command_that_loses_the_root_floor_race_fails() {
    let (registry, _) = acme();
    let (dir, _, itaoca) = project("floor-race", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let floors = floor_dir(&itaoca, &registry);
    let floor = floors.join("root-version");
    let checkpoint = std::fs::read(floors.join("checkpoint")).unwrap();
    let first = una_project::tlog::root::digest(&registry.roots[0]);
    let other = format!("sha256:{}", "0".repeat(64));
    let write = |text: &str| {
        std::fs::remove_file(&floor).unwrap();
        std::fs::write(&floor, text).unwrap();
    };
    // Runs `args` while another run holds the floor lock and stores `meanwhile` as the floor.
    let raced = |args: &[&str], meanwhile: &str| {
        write("1\n");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(floors.join("floor.lock"))
            .unwrap();
        lock.lock().unwrap();
        std::thread::scope(|scope| {
            let command = scope.spawn(|| una_in(args, &itaoca));
            std::thread::sleep(std::time::Duration::from_millis(1500));
            write(meanwhile);
            lock.unlock().unwrap();
            command.join().unwrap()
        })
    };
    for args in [
        &["una", "ita", "lock", "--check", arg(&dir)][..],
        &["una", "ita", "lock", arg(&dir)][..],
    ] {
        let forked = format!("1 {other}\n");
        let (code, out, err) = raced(args, &forked);
        assert_eq!(code, 1, "{args:?}: {out}{err}");
        assert!(
            err.contains("U7010")
                && err.contains(&other)
                && (err.contains("root-version") || err.contains("forked")),
            "{args:?}: {err}"
        );
        assert!(!err.contains("not stored as its floor"), "{args:?}: {err}");
        assert_eq!(std::fs::read_to_string(&floor).unwrap(), forked);
        assert_eq!(
            std::fs::read(floors.join("checkpoint")).unwrap(),
            checkpoint
        );
        // Legitimate: the other run stored the digest this command walked.
        let same = format!("1 {first}\n");
        let (code, out, err) = raced(args, &same);
        assert_eq!(code, 0, "{args:?}: {out}{err}");
        assert!(
            !err.contains("not stored") && !err.contains("U7010"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&floor).unwrap(), same);
    }
}

/// #2132, the present behavior, reproduced end to end: a mirror that holds no key poisons the
/// root-version floor for good. The lock is made under root 1; the registry then rotates to
/// root 2 with the same witnesses. A mirror directory serves the honest root 2 with one junk
/// signature line more and no latest checkpoint. `una ita fetch` succeeds from it and stores
/// root version 2 with the digest of the altered note. From then on every command that reads
/// the honest registry is refused (U7010, "forked"). Removing the floor file does not end it:
/// the provenance fetched from the mirror, which Itaoca never replaces, holds the altered note,
/// and `una ita lock --check` refuses it against the honest registry's root 2.
///
/// This test documents the defect and flips when #2132 is fixed.
#[test]
fn a_mirror_with_no_key_poisons_the_root_floor() {
    let (mut registry, _) = acme();
    let (dir, mirror, itaoca) = project("floor-poison", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let floor = floor_dir(&itaoca, &registry).join("root-version");
    let first = una_project::tlog::root::digest(&registry.roots[0]);
    assert_eq!(
        std::fs::read_to_string(&floor).unwrap(),
        format!("1 {first}\n")
    );
    // The rotation: root 2, the same witnesses, and a later checkpoint signed under it.
    registry.rotate_witnesses(0, false);
    registry.append("other", "after the rotation");
    registry.checkpoint(now() - 30);
    let honest = mirror.parent().unwrap().join("honest");
    registry.write(&honest);
    // The mirror: the honest files, but root 2 with a junk signature line, and no latest
    // checkpoint. It needs no key.
    let altered = format!(
        "{}\u{2014} nobody {}\n",
        registry.roots[1],
        una_project::tlog::note::encode_base64(&[7u8; 68])
    );
    let second = una_project::tlog::root::digest(&registry.roots[1]);
    let poisoned = una_project::tlog::root::digest(&altered);
    assert_ne!(poisoned, second);
    registry.write(&mirror);
    std::fs::remove_file(mirror.join("checkpoint/latest")).unwrap();
    std::fs::write(mirror.join("root/2"), &altered).unwrap();

    let (code, out, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(
        (code, out.as_str()),
        (0, "fetched acme/ledger 0.3.1\n"),
        "{err}"
    );
    assert!(!err.contains("not stored"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&floor).unwrap(),
        format!("2 {poisoned}\n")
    );
    assert_eq!(codes(&dir, &itaoca), Vec::<String>::new());

    // The honest registry, from then on: refused.
    list_mirrors(&itaoca, &[&honest]);
    for args in [
        &["una", "ita", "lock", "--check", arg(&dir)][..],
        &["una", "ita", "lock", arg(&dir)][..],
    ] {
        let (code, out, err) = una_in(args, &itaoca);
        assert_eq!(
            code, 1,
            "#2132 is fixed, flip this test: {args:?}: {out}{err}"
        );
        assert!(
            err.contains("U7010") && err.contains("forked") && err.contains(&poisoned),
            "{args:?}: {err}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&floor).unwrap(),
        format!("2 {poisoned}\n")
    );
    // The user removes the floor file: the provenance stored from the mirror is still refused.
    std::fs::remove_file(&floor).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("U7010") && err.contains("forked"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&floor).unwrap(),
        format!("2 {second}\n")
    );
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
        let text =
            std::fs::read_to_string(floor_dir(&itaoca, registry).join("checkpoint")).unwrap();
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
            "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n\nregistry acme\n  root \"{}\"\n  checkpoint {} \"sha256:{hex}\"\n  last root {} \"{}\"\n\npackage acme/ledger 0.3.1\n  from acme\n  digest \"{digest}\"\n  signed \"{signed}\"\n  logged {index}\n",
            registry.pin(),
            leaves.len(),
            registry.roots.len(),
            una_project::tlog::root::digest(registry.roots.last().unwrap())
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
    // Consent lines follow the registry's `last root` line.
    let checkpoint_line = lock
        .lines()
        .find(|line| line.starts_with("  last root "))
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
        assert!(
            err.contains("U7009") && err.contains("accepted index"),
            "{forged}: {err}"
        );
    }
}

/// An `accepted` or `effect` index at or above the registry's checkpoint size is U7009 when
/// the lock is read.
#[test]
fn a_consented_index_past_the_checkpoint_is_refused_when_read() {
    let (registry, _) = acme();
    let (dir, _, itaoca) = project("index-range", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let checkpoint_line = lock
        .lines()
        .find(|line| line.starts_with("  checkpoint "))
        .unwrap()
        .to_owned();
    let size: u64 = checkpoint_line.split(' ').nth(3).unwrap().parse().unwrap();
    // Consent lines follow the registry's `last root` line.
    let last_line = lock
        .lines()
        .find(|line| line.starts_with("  last root "))
        .unwrap()
        .to_owned();
    for line in [
        format!("accepted acme {size}"),
        format!("effect acme {}", size + 5),
    ] {
        std::fs::write(
            dir.join("ita.lock"),
            lock.replacen(&last_line, &format!("{last_line}\n  {line}"), 1),
        )
        .unwrap();
        let (_, out, _) = una_in(&["una", "--json", "check", arg(&dir)], &itaoca);
        let document: serde_json::Value = serde_json::from_str(&out).unwrap();
        let codes: Vec<&str> = document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["code"].as_str().unwrap())
            .collect();
        assert!(codes.contains(&"U7009"), "{line}: {codes:?}");
    }
}

/// Plain `--check` needs no fresh checkpoint: with a place that serves none newer than the
/// lock's, it passes and says that a later freeze was not checked; `--check --fresh` fails
/// there (U7013), passes at a fresh checkpoint, and fails (U7013) once the owner's chain holds
/// an entry this release does not replay.
#[test]
fn check_fresh_needs_a_fresh_checkpoint() {
    let (mut registry, _) = acme();
    let (dir, mirror, itaoca) = project("check-fresh", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    // A place that serves the lock's checkpoint and none newer.
    std::fs::remove_file(mirror.join("checkpoint/latest")).unwrap();
    let (code, out, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    assert!(
        format!("{out}{err}").contains("a later freeze was not checked"),
        "{out}{err}"
    );
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--check", "--fresh", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("U7013") && err.contains("--fresh"), "{err}");
    // A fresh checkpoint and no freeze: it passes, and plain --check gives the time it read.
    registry.append("other", "later filler");
    registry.checkpoint(now() - 60);
    registry.write(&mirror);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--check", "--fresh", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    assert!(
        format!("{out}{err}").contains("checked at a checkpoint of witness time")
            && format!("{out}{err}").contains('Z'),
        "{out}{err}"
    );
    // A dispute after the lock's point: the owner's names are unavailable, and
    // --check --fresh fails with U7013.
    registry.recover("acme", "dispute", &keypair(72, "acme.m").1, &[]);
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (code, _, err) = una_in(
        &["una", "ita", "lock", "--check", "--fresh", arg(&dir)],
        &itaoca,
    );
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("U7013") && !err.contains("U7010") && err.contains("does not replay"),
        "{err}"
    );
}

/// Plain `--check` reads the lock's resolution point from a place: with no place reachable it
/// fails closed (U7013), never passing unchecked.
#[test]
fn check_with_no_place_fails_closed() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("check-no-place", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    std::fs::remove_dir_all(&mirror).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("U7013"), "{err}");
}

/// A newer checkpoint a place serves that does not extend the lock's resolution point, a
/// fork, fails plain `--check` too (U7010): the newest checkpoint must prove `extends`.
#[test]
fn check_fails_on_a_served_checkpoint_that_forks_from_the_lock() {
    let (registry, _) = acme();
    let (dir, mirror, itaoca) = project("check-fork", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    // Another log under the same keys: other entries from the first one on.
    let mut fork = TestRegistry::new("acme");
    let other = keypair(61, "acme.other");
    fork.register("acme", &other);
    for patch in 1..=5 {
        let version = format!("0.3.{patch}");
        fork.release("acme/ledger", &version, "none", "none", &[&other]);
    }
    let size = fork.entries.len();
    fork.checkpoint(now() - 30);
    // The place serves the lock's point as it was, and the fork's newest checkpoint with the
    // tiles and map it needs.
    for (path, bytes) in &fork.files {
        let newest = path == "checkpoint"
            || path.starts_with("map/")
            || path.ends_with(&format!(".p/{size}"));
        if newest {
            let file = if path == "checkpoint" {
                mirror.join("checkpoint").join("latest")
            } else {
                mirror.join(path)
            };
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
    }
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("U7010") && err.contains("forked"), "{err}");
}

/// Two packages fetched from one registry, the first archive served by no place and the second
/// served but another package's, then the reverse: each failure has its own code, U7013 for
/// the archive unserved and U7010 for the one that does not verify, never one carried over
/// from the other.
#[test]
fn each_package_of_a_fetch_has_the_code_of_its_own_failure() {
    let key = keypair(60, "acme.laptop");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &key);
    let (_, alpha) = registry.release("acme/alpha", "0.1.0", "none", "none", &[&key]);
    let (_, beta) = registry.release("acme/beta", "0.1.0", "none", "none", &[&key]);
    registry.checkpoint(now() - 20 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(now() - 60);
    let orders = [
        ("alpha-first", &alpha, &beta),
        ("beta-first", &beta, &alpha),
    ];
    for (name, unserved, mismatched) in orders {
        let base = scratch(&format!("two-packages-{name}"));
        let dir = base.join("demo");
        let mirror = base.join("mirror");
        let itaoca = base.join("itaoca");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(&itaoca).unwrap();
        std::fs::write(
            dir.join("ita.una"),
            format!(
                "ita demo\n  version 0.1.0\n  edition 0.4\n\nsources\n  acme \"https://ita.acme.example\"\n    root \"{}\"\n    owners acme\n\nneeds\n  acme/alpha 0.1 from acme\n  acme/beta 0.1 from acme\n",
                registry.pin()
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("src/demo.una"),
            "module demo\n\npub forty_two() -> Int = 42\n",
        )
        .unwrap();
        registry.write(&mirror);
        list_mirrors(&itaoca, &[&mirror]);
        let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
        assert_eq!(code, 0, "{err}");
        let blob = |digest: &str| mirror.join(format!("blobs/sha256/{}", &digest[7..]));
        // One archive no place serves; the other replaced by the first's bytes.
        let other = std::fs::read(blob(unserved)).unwrap();
        std::fs::remove_file(blob(unserved)).unwrap();
        std::fs::write(blob(mismatched), other).unwrap();
        let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
        assert_eq!(code, 1);
        let at_unserved = err.find("U7013").unwrap_or_else(|| panic!("{err}"));
        let at_refused = err.find("U7010").unwrap_or_else(|| panic!("{err}"));
        // The lock lists acme/alpha before acme/beta.
        if name == "alpha-first" {
            assert!(at_unserved < at_refused, "{err}");
        } else {
            assert!(at_refused < at_unserved, "{err}");
        }
    }
}

/// A resolution whose source's registry fails while it is read takes the code of that failure:
/// the release statements' entry tiles served by no place make it unavailable (U7013), and
/// tiles served but altered make it a refusal (U7010).
#[test]
fn a_resolution_takes_the_code_of_its_registrys_failure() {
    let (registry, _) = acme();
    let entries: Vec<&String> = registry
        .files
        .keys()
        .filter(|path| path.starts_with("tile/entries/"))
        .collect();
    assert!(!entries.is_empty());
    for (name, expected) in [("unserved-entries", "U7013"), ("altered-entries", "U7010")] {
        let (dir, mirror, itaoca) = project(name, &registry);
        for path in &entries {
            let file = mirror.join(path.as_str());
            if expected == "U7013" {
                std::fs::remove_file(&file).unwrap();
            } else {
                let mut bytes = std::fs::read(&file).unwrap();
                let at = bytes.len() / 2;
                bytes[at] ^= 0x01;
                std::fs::write(&file, &bytes).unwrap();
            }
        }
        let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
        assert_eq!(code, 1, "{name}: {err}");
        assert!(err.contains(expected), "{name}: {err}");
        let other = if expected == "U7013" {
            "U7010"
        } else {
            "U7013"
        };
        assert!(!err.contains(other), "{name}: {err}");
        assert!(!dir.join("ita.lock").exists());
    }
}

/// `spec/30 §11` rule 9 end to end: after a rotation to root 2, a place whose latest checkpoint
/// is of the lock's point's own size and names root 2, while the point names root 1, serves two
/// checkpoints of that size, both signed by the log. `una ita lock --check` refuses it (U7010,
/// exit code 1) and reports it as evidence against the registry, with both versions.
#[test]
fn a_same_size_checkpoint_under_another_root_is_evidence() {
    let (mut registry, _) = acme();
    let (dir, mirror, itaoca) = project("same-size-evidence", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let size = lock
        .lines()
        .find_map(|line| line.strip_prefix("  checkpoint "))
        .and_then(|rest| rest.split(' ').next())
        .unwrap()
        .to_owned();
    let first_line = format!(
        "ita-root 1 {}",
        una_project::tlog::root::digest(&registry.roots[0])
    );
    registry.rotate_witnesses(2, false);
    let point = String::from_utf8(registry.files[&format!("checkpoint/{size}")].clone()).unwrap();
    let text = point.split_once("\n\n").unwrap().0.to_owned() + "\n";
    assert!(text.contains(&first_line), "{text}");
    let text = text.replacen(&first_line, &registry.root_line(), 1);
    let mut latest = format!(
        "{text}\n{}",
        support::sign_line(&registry.keys.log.0, &registry.keys.log.1, &text)
    );
    for (pair, key) in &registry.keys.witnesses {
        latest.push_str(&support::cosign_line(pair, key, &text, now() - 30));
    }
    registry
        .files
        .insert("checkpoint".to_owned(), latest.into_bytes());
    registry.write(&mirror);
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("U7010") && !err.contains("U7013"), "{err}");
    assert!(
        err.contains(&format!("two checkpoints of size {size}")),
        "{err}"
    );
    assert!(
        err.contains("evidence against the registry `acme`")
            && err.contains("naming the root versions 1 and 2"),
        "{err}"
    );
    // `una ita bundle` reads the same point: refused, said as evidence, nothing written.
    let file = dir.parent().unwrap().join("demo.bundle");
    let (code, _, err) = una_in(&["una", "ita", "bundle", arg(&file), arg(&dir)], &itaoca);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("U7010") && err.contains("evidence against the registry `acme`"),
        "{err}"
    );
    assert!(!file.exists());
}

/// ADR 0122, amendment of 2026-11-03, end to end: a lock's point signed under root 1, whose
/// witnesses root 2 then retires, is fetched with the consistency proof and loads offline; a
/// provenance of the earlier format, with no proof, is unavailable (U7013, exit code 1) until
/// `una ita fetch` stores the newer format beside it; `una ita lock --check` keeps the lock's
/// `last root`; and the root-version floor this machine stores refuses a chain below it
/// (U7010) and, unreadable, makes the load unavailable (U7013).
#[test]
fn a_retired_witness_counts_offline_only_with_the_proof() {
    let (mut registry, _) = acme();
    let (dir, mirror, itaoca) = project("retired-proof", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let first_root = una_project::tlog::root::digest(&registry.roots[0]);
    assert!(
        lock.contains(&format!("\n  last root 1 \"{first_root}\"\n")),
        "{lock}"
    );
    // Two of the three witnesses retired by root 2, and a later checkpoint signed under it.
    registry.rotate_witnesses(2, false);
    registry.append("other", "after the rotation");
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (code, out, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(
        (code, out.as_str()),
        (0, "fetched acme/ledger 0.3.1\n"),
        "{err}"
    );
    assert_eq!(codes(&dir, &itaoca), Vec::<String>::new());
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let release = itaoca.join("registry/acme/releases/acme+ledger-0.3.1");
    let size = lock
        .lines()
        .find_map(|line| line.strip_prefix("  checkpoint "))
        .and_then(|rest| rest.split(' ').next())
        .unwrap()
        .to_owned();
    let second = std::fs::read(release.join(format!("checkpoint-{size}-2.provenance"))).unwrap();
    let mut document = una_project::provenance::parse(&second).unwrap();
    assert!(document.latest.is_some());
    // The earlier format, alone in another machine's Itaoca: unavailable, exit code 1.
    document.latest = None;
    document.format = una_project::provenance::Format::V1;
    let other = scratch("retired-proof-format-1").join("itaoca");
    std::fs::create_dir_all(&other).unwrap();
    list_mirrors(&other, &[&mirror]);
    let store = una_project::itaoca::Itaoca {
        root: other.clone(),
    };
    let digest = lock
        .lines()
        .find_map(|line| line.strip_prefix("  digest \"sha256:"))
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap();
    let archive = registry.files[&format!("blobs/sha256/{digest}")].clone();
    store.store(&archive).unwrap();
    store
        .store_provenance(
            "acme",
            "acme/ledger",
            "0.3.1",
            size.parse().unwrap(),
            &document.encode(),
        )
        .unwrap();
    let found = codes(&dir, &other);
    assert!(
        found.contains(&"U7013".to_owned()) && !found.contains(&"U7010".to_owned()),
        "{found:?}"
    );
    let (code, _, err) = una_in(&["una", "check", arg(&dir)], &other);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("retired"), "{err}");
    // Fetched again: the newer format is stored beside it, and preferred.
    let (code, out, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &other);
    assert_eq!(
        (code, out.as_str()),
        (0, "fetched acme/ledger 0.3.1\n"),
        "{err}"
    );
    assert_eq!(codes(&dir, &other), Vec::<String>::new());
    // The root-version floor: below it, refused; unreadable, unavailable.
    let floor = floor_dir(&itaoca, &registry).join("root-version");
    std::fs::remove_file(&floor).unwrap();
    std::fs::write(&floor, "3\n").unwrap();
    assert!(codes(&dir, &itaoca).contains(&"U7010".to_owned()));
    let (_, _, err) = una_in(&["una", "check", arg(&dir)], &itaoca);
    assert!(
        err.contains("older than root version 3") && !err.contains("forked"),
        "{err}"
    );
    std::fs::remove_file(&floor).unwrap();
    std::fs::create_dir(&floor).unwrap();
    let found = codes(&dir, &itaoca);
    assert!(
        found.contains(&"U7013".to_owned()) && !found.contains(&"U7010".to_owned()),
        "{found:?}"
    );
}

/// An Itaoca of its own below the test's directory, holding only the `mirrors.una` that lists
/// `mirror`: a machine that never stored a floor, such as a CI run.
fn empty_itaoca(dir: &Path, name: &str, mirror: &Path) -> PathBuf {
    let itaoca = dir.parent().unwrap().join(name);
    let _ = std::fs::remove_dir_all(&itaoca);
    std::fs::create_dir_all(&itaoca).unwrap();
    list_mirrors(&itaoca, &[mirror]);
    itaoca
}

/// Sets the mode of `path`, a directory the test made.
#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// The vendored provenance of `acme/ledger 0.3.1` in the package at `dir`.
fn vendored_provenance(dir: &Path) -> PathBuf {
    let found: Vec<PathBuf> = std::fs::read_dir(dir.join("vendor/ita/acme"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "provenance"))
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
    found[0].clone()
}

/// A package locked at a point signed under root 1, after which root 2 retires two of the
/// registry's three witnesses and a later checkpoint is signed under it; fetched and vendored
/// then, so its provenance holds roots 1 and 2 and the latest checkpoint's proof, while the lock
/// records `last root 1`. Returns the package, the mirror and the registry.
fn locked_across_a_rotation(name: &str) -> (PathBuf, PathBuf, TestRegistry) {
    let (mut registry, _) = acme();
    let (dir, mirror, itaoca) = project(name, &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    registry.rotate_witnesses(2, false);
    registry.append("other", "after the rotation");
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = una_in(&["una", "ita", "vendor", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let first_root = una_project::tlog::root::digest(&registry.roots[0]);
    assert!(
        lock.contains(&format!("\n  last root 1 \"{first_root}\"\n")),
        "{lock}"
    );
    let document =
        una_project::provenance::parse(&std::fs::read(vendored_provenance(&dir)).unwrap()).unwrap();
    assert_eq!(document.roots.len(), 2);
    assert!(document.latest.is_some());
    assert_eq!(codes(&dir, &itaoca), Vec::<String>::new());
    (dir, mirror, registry)
}

/// Asserts that `una ita lock --check` (with `extra` flags) refuses the package at `dir` with
/// Itaoca at `itaoca` as a provenance whose root chain ends below root version 2.
fn refused_below_root_2(dir: &Path, itaoca: &Path, extra: &[&str]) {
    let mut args = vec!["una", "ita", "lock", "--check"];
    args.extend_from_slice(extra);
    args.push(arg(dir));
    let (code, _, err) = una_in(&args, itaoca);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("U7010") && !err.contains("U7009"), "{err}");
    assert!(
        err.contains("ends at root version 1, older than root version 2")
            && err.contains("lock again at a fresh resolution point")
            && !err.contains("forked"),
        "{err}"
    );
}

/// ADR 0122, amendment of 2026-11-09 (L1): an attacker who edits the vendored package, its
/// provenance and the lock drops root 2 and the `latest` block from the vendored provenance and
/// lowers the lock's `last root` to 1 (here the lock, written before root 2 was served, records
/// `last root 1` already, so lowering it changes nothing). `una ita lock --check`, with and
/// without `--fresh`, walks the chain served to root 2 at the lock's point and refuses the
/// provenance (U7010, "lock again"), whatever Itaoca holds: empty, read-only, or failing to
/// store the floor. The floor store stays a note.
#[cfg(unix)]
#[test]
fn lock_check_refuses_a_root_chain_cut_below_the_version_it_walked() {
    let (dir, mirror, registry) = locked_across_a_rotation("cut-chain");
    let path = vendored_provenance(&dir);
    let mut document = una_project::provenance::parse(&std::fs::read(&path).unwrap()).unwrap();
    document.roots.truncate(1);
    document.latest = None;
    document.format = una_project::provenance::Format::V1;
    std::fs::write(&path, document.encode()).unwrap();

    // An empty Itaoca, as a CI run starts with: the cut passes `una check` there, and
    // `--check` refuses it, plain and `--fresh`.
    let empty = empty_itaoca(&dir, "itaoca-empty", &mirror);
    assert_eq!(codes(&dir, &empty), Vec::<String>::new());
    refused_below_root_2(&dir, &empty, &[]);
    let empty = empty_itaoca(&dir, "itaoca-empty-fresh", &mirror);
    refused_below_root_2(&dir, &empty, &["--fresh"]);

    // A read-only Itaoca: no floor is stored, and `--check` still refuses.
    let read_only = empty_itaoca(&dir, "itaoca-read-only", &mirror);
    set_mode(&read_only, 0o555);
    refused_below_root_2(&dir, &read_only, &[]);
    refused_below_root_2(&dir, &read_only, &["--fresh"]);
    let stored = floor_dir(&read_only, &registry)
        .join("root-version")
        .exists();
    set_mode(&read_only, 0o755);
    assert!(!stored);

    // Storing the floor fails (its lock cannot be opened): a note, and `--check` still refuses.
    let failing = empty_itaoca(&dir, "itaoca-store-fails", &mirror);
    std::fs::create_dir_all(floor_dir(&failing, &registry).join("floor.lock")).unwrap();
    refused_below_root_2(&dir, &failing, &[]);
    assert!(!floor_dir(&failing, &registry).join("root-version").exists());
    // The cut still loads offline there, which is why `--check` cannot rely on the floor.
    assert_eq!(codes(&dir, &failing), Vec::<String>::new());
}

/// The honest lock of the same history passes `una ita lock --check`, with a writable, an
/// empty and a read-only Itaoca: its vendored chain ends exactly at root 2, the version the
/// command walked, which is not below it.
#[cfg(unix)]
#[test]
fn lock_check_passes_a_chain_ending_at_the_version_it_walked() {
    let (dir, mirror, _) = locked_across_a_rotation("walked-chain");
    let itaoca = dir.parent().unwrap().join("itaoca");
    for extra in [&[][..], &["--fresh"][..]] {
        let mut args = vec!["una", "ita", "lock", "--check"];
        args.extend_from_slice(extra);
        args.push(arg(&dir));
        let (code, _, err) = una_in(&args, &itaoca);
        assert_eq!(code, 0, "{err}");
        let empty = empty_itaoca(&dir, "itaoca-empty", &mirror);
        let (code, _, err) = una_in(&args, &empty);
        assert_eq!(code, 0, "{err}");
        set_mode(&empty, 0o555);
        let (code, _, err) = una_in(&args, &empty);
        set_mode(&empty, 0o755);
        assert_eq!(code, 0, "{err}");
    }
    // Itaoca's copy alone, with no vendor directory, is verified too, and passes.
    std::fs::remove_dir_all(dir.join("vendor")).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
}

/// After an honest rotation, a lock whose provenance was written before it fails
/// `una ita lock --check` with "lock again" (the fail-safe cost of the amendment of
/// 2026-11-09), vendored and in Itaoca alike; a package that neither place holds is not checked
/// by `--check` (that is `una check`'s U7011).
#[test]
fn lock_check_says_lock_again_after_a_rotation_and_skips_an_absent_package() {
    let (mut registry, _) = acme();
    let (dir, mirror, itaoca) = project("rotation-after-lock", &registry);
    let (code, _, err) = una_in(&["una", "ita", "lock", arg(&dir)], &itaoca);
    assert_eq!(code, 0, "{err}");
    // Fetched before the rotation is served: its provenance's chain ends at root 1.
    let stored = empty_itaoca(&dir, "itaoca-before", &mirror);
    let (code, _, err) = una_in(&["una", "ita", "fetch", arg(&dir)], &stored);
    assert_eq!(code, 0, "{err}");
    registry.rotate_witnesses(2, false);
    registry.append("other", "after the rotation");
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    // In neither place: `--check` passes, and `una check` says U7011.
    let empty = empty_itaoca(&dir, "itaoca-absent", &mirror);
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &empty);
    assert_eq!(code, 0, "{err}");
    assert!(codes(&dir, &empty).contains(&"U7011".to_owned()));
    // In Itaoca: refused once `--check` walks root 2.
    refused_below_root_2(&dir, &stored, &[]);
    // Vendored from it (before the floor of root 2 is stored there): refused the same way,
    // with an empty Itaoca.
    std::fs::remove_file(floor_dir(&stored, &registry).join("root-version")).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "vendor", arg(&dir)], &stored);
    assert_eq!(code, 0, "{err}");
    let empty = empty_itaoca(&dir, "itaoca-vendored", &mirror);
    refused_below_root_2(&dir, &empty, &[]);
}

/// `una ita lock --check` reports the offline load's other results: a vendored provenance whose
/// point carries a retired witness's cosignature without the proof (its `latest` block
/// dropped, its chain kept) is unavailable (U7013), and so is a stored root-version floor that
/// exists and cannot be read.
#[test]
fn lock_check_reports_an_unavailable_provenance_as_the_offline_load_does() {
    let (dir, _, registry) = locked_across_a_rotation("check-unavailable");
    let itaoca = dir.parent().unwrap().join("itaoca");
    let path = vendored_provenance(&dir);
    let honest = std::fs::read(&path).unwrap();
    let mut document = una_project::provenance::parse(&honest).unwrap();
    document.latest = None;
    document.format = una_project::provenance::Format::V1;
    std::fs::write(&path, document.encode()).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("U7013") && !err.contains("U7010") && err.contains("retired"),
        "{err}"
    );
    std::fs::write(&path, &honest).unwrap();
    let floor = floor_dir(&itaoca, &registry).join("root-version");
    std::fs::remove_file(&floor).unwrap();
    std::fs::create_dir(&floor).unwrap();
    let (code, _, err) = una_in(&["una", "ita", "lock", "--check", arg(&dir)], &itaoca);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("U7013") && !err.contains("U7010") && err.contains("cannot be read"),
        "{err}"
    );
}
