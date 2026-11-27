//! `una ita publish`, `yank`, `audit`, `watch`, `bundle` and `unbundle` (ADR 0122 rules 33, 36,
//! 36.2 and 41 to 43, `spec/30 §15`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "../../una-project/tests/support/registry.rs"]
mod support;

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use support::{TestRegistry, keypair};
use una_project::client::{Point, Registry};
use una_project::content::File;
use una_project::registry::keyfile::SigningKey;
use una_project::registry::submit::{self, NameState, Submission};
use una_project::transport::Directory;
use una_project::tree::VerifiedTree;
use una_project::version::Version;

const MANIFEST: &str = "ita demo\n  version 0.1.0\n  edition 0.4\n";
const MODULE: &str = "module demo\n\npub one() -> Int = 1\n";

/// A fresh directory below the test's temporary directory.
fn scratch(group: &str, name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(group)
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A fresh package directory named `name`, with `ita.una` and one module.
fn package(name: &str, manifest: &str) -> PathBuf {
    let dir = scratch("ita-registry", name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("ita.una"), manifest).unwrap();
    std::fs::write(dir.join("src/demo.una"), MODULE).unwrap();
    dir
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

fn path(dir: &Path) -> &str {
    dir.to_str().unwrap()
}

/// The codes of a `--json` document.
fn codes_of(out: &str) -> Vec<String> {
    let document: serde_json::Value = serde_json::from_str(out).unwrap();
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

const DAY: u64 = 24 * 3600;

/// The local clock, as `una ita` reads it.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The seeds and names of the owner `acme`'s keys: three, at the key floor with threshold 2.
const ACME_KEYS: [(u8, &str); 3] = [(60, "acme.laptop"), (66, "acme.desk"), (67, "acme.spare")];

/// A registry `acme`: the owner `acme` registered with its three keys ([`ACME_KEYS`]) at
/// threshold 2, its checkpoint cosigned `ago` seconds before now. Returns it and the signing
/// keys a publisher holds: the first two, which meet the threshold.
fn acme_with(ago: u64) -> (TestRegistry, Vec<SigningKey>) {
    let mut registry = TestRegistry::new("acme");
    let pairs: Vec<_> = ACME_KEYS
        .iter()
        .map(|(seed, name)| keypair(*seed, name))
        .collect();
    let keys: Vec<_> = pairs.iter().collect();
    registry.register_keys("acme", &keys, 2, &[], 0);
    registry.checkpoint(now() - ago);
    let signing = ACME_KEYS[..2]
        .iter()
        .map(|(seed, name)| SigningKey::from_secret(name, &[*seed; 32]).unwrap())
        .collect();
    (registry, signing)
}

/// The publisher's signing keys, as `submit` takes them.
fn signers(signing: &[SigningKey]) -> Vec<&SigningKey> {
    signing.iter().collect()
}

/// The registry's client at its latest checkpoint, read from its files as a mirror serves them.
fn client(registry: &TestRegistry) -> Registry<'static> {
    // The files live as long as the test: leaked on purpose, so the client may borrow them.
    let memory: &'static una_project::registry::bundle::Memory =
        Box::leak(Box::new(una_project::registry::bundle::Memory {
            source: "acme".to_owned(),
            files: registry.files.clone(),
        }));
    Registry::open(
        "acme",
        memory,
        &registry.pin(),
        &Point::Latest { now: now() },
        None,
    )
    .unwrap()
}

/// The registry logs a submission it found in `inbox`: the archive, as a blob, and the entry,
/// as the next entry about `name`.
fn log_submission(registry: &mut TestRegistry, inbox: &Path, submission: &Submission, name: &str) {
    let dir = inbox.join(submission.dir());
    if let Ok(archive) = std::fs::read(dir.join("archive")) {
        let digest = VerifiedTree::from_archive(&archive)
            .unwrap()
            .digest()
            .to_owned();
        let hex = digest.strip_prefix("sha256:").unwrap();
        registry
            .files
            .insert(format!("blobs/sha256/{hex}"), archive);
    }
    let entry = std::fs::read_to_string(dir.join("entry")).unwrap();
    registry.append(name, &entry);
}

/// The files of the release `acme/ledger` at `version`, as a publisher's package holds them.
fn ledger_files(version: &str) -> (String, Vec<File>) {
    let manifest = format!(
        "ita acme/ledger\n  version {version}\n  edition 0.4\n  about \"Ledgers\"\n  license \"Apache-2.0\"\n  publish to acme\n"
    );
    let files = vec![
        File {
            path: "ita.una".to_owned(),
            bytes: manifest.as_bytes().to_vec(),
        },
        File {
            path: "src/ledger.una".to_owned(),
            bytes: b"module ledger\n\npub total() -> Int = 3\n".to_vec(),
        },
    ];
    (manifest, files)
}

/// A package that needs `acme/ledger 0.3` from `acme`, the registry written as a mirror beside
/// it, and an Itaoca whose `mirrors.una` lists it. Returns the package, the mirror and Itaoca.
fn project(name: &str, registry: &TestRegistry) -> (PathBuf, PathBuf, PathBuf) {
    let base = scratch("ita-registry-projects", name);
    let dir = base.join("demo");
    let mirror = base.join("mirror");
    let itaoca = base.join("itaoca");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(&itaoca).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "{MANIFEST}\nsources\n  acme \"https://ita.acme.example\"\n    root \"{}\"\n    owners acme\n\nneeds\n  acme/ledger 0.3 from acme\n",
            registry.pin()
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/demo.una"), MODULE).unwrap();
    registry.write(&mirror);
    std::fs::write(
        itaoca.join("mirrors.una"),
        format!("mirrors\n  acme\n    \"file://{}\"\n", mirror.display()),
    )
    .unwrap();
    (dir, mirror, itaoca)
}

/// A registry `acme` that holds `acme/ledger 0.3.1`, published through the library into a
/// directory and logged a month ago, and a fresh checkpoint.
fn published_registry(name: &str) -> (TestRegistry, Vec<SigningKey>, PathBuf) {
    let (mut registry, signing) = acme_with(40 * DAY);
    let inbox = scratch("ita-registry-inbox", name);
    let size = registry.entries.len() as u64;
    let checkpoint =
        String::from_utf8(registry.files[&format!("checkpoint/{size}")].clone()).unwrap();
    let note = una_project::tlog::note::parse_note(&checkpoint).unwrap();
    let root = una_project::tlog::note::parse_checkpoint(&note.text)
        .unwrap()
        .root;
    let memory = una_project::registry::bundle::Memory {
        source: "acme".to_owned(),
        files: registry.files.clone(),
    };
    let mut reader = Registry::open(
        "acme",
        &memory,
        &registry.pin(),
        &Point::At {
            size,
            hash: support::hash_hex(&root),
        },
        None,
    )
    .unwrap();
    let state = NameState::read(&mut reader, "acme/ledger").unwrap();
    let (text, files) = ledger_files("0.3.1");
    let (manifest, errors) = una_syntax::ita::parse_manifest(&text);
    assert!(errors.is_empty(), "{errors:?}");
    let submission = submit::release(
        &manifest,
        files,
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeSet::new(),
        &state,
        &signers(&signing),
    )
    .unwrap();
    // Published into a directory: the registry's inbox.
    submit::send(&Directory::new(&inbox), &submission).unwrap();
    log_submission(&mut registry, &inbox, &submission, "acme/ledger");
    registry.checkpoint(now() - 30 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(now() - 60);
    (registry, signing, inbox)
}

// --- publish and yank ------------------------------------------------------------------------

fn published(target: &str) -> String {
    format!(
        "ita acme/demo\n  version 0.1.0\n  edition 0.4\n  about \"A demo\"\n  license \"Apache-2.0\"\n  publish to {target}\n"
    )
}

/// A fresh directory outside any git repository: below the system's temporary directory, which
/// the test checks holds no `.git` on its way to the root.
fn outside_git(name: &str) -> PathBuf {
    let base = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("una-ita-registry-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    assert_eq!(
        una_project::registry::submit::git_entry(&base),
        Ok(None),
        "the temporary directory is inside a git repository"
    );
    base
}

/// A package `pkg` in a fresh directory outside any git repository.
fn package_outside_git(name: &str, manifest: &str) -> PathBuf {
    let dir = outside_git(name).join("pkg");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("ita.una"), manifest).unwrap();
    std::fs::write(dir.join("src/demo.una"), MODULE).unwrap();
    dir
}

#[test]
fn publish_refuses_what_rule_43_names_and_sends_nothing() {
    let store = scratch("ita-registry-store", "refusals");
    for (name, manifest, part) in [
        ("no-target", MANIFEST.to_owned(), "publish to"),
        (
            "path-need",
            format!(
                "{}\nneeds\n  util from path \"deps/util\"\n    accept name\n",
                published("acme")
            ),
            "path dependency",
        ),
    ] {
        let dir = package(&format!("publish-{name}"), &manifest);
        let (status, out, _) = una_in(&["una", "--json", "ita", "publish", path(&dir)], &store);
        assert_eq!(status, 1, "{name}");
        assert!(
            codes_of(&out).contains(&"U7013".to_owned()),
            "{name}: {out}"
        );
        assert!(out.contains(part), "{name}: {out}");
    }
    // A package that does not check is refused too.
    let dir = package_outside_git("unchecked", &published("itaty"));
    std::fs::write(
        dir.join("src/demo.una"),
        "module demo\n\npub one() -> Int = \"one\"\n",
    )
    .unwrap();
    let (status, out, _) = una_in(&["una", "--json", "ita", "publish", path(&dir)], &store);
    assert_eq!(status, 1);
    assert!(out.contains("checks at tier 1 with none"), "{out}");
}

/// Until the check for uncommitted changes is available, a package with a `.git` entry in its
/// directory or an ancestor is not published; no program is started to look.
#[test]
fn publish_refuses_a_package_in_a_git_repository() {
    let store = scratch("ita-registry-store", "git");
    let refused = |dir: &Path| {
        let (status, out, _) = una_in(&["una", "--json", "ita", "publish", path(dir)], &store);
        status == 1 && out.contains("uncommitted changes is not available yet")
    };
    // A package several directories below a repository's root. The test's target directory
    // may be anywhere, inside a checkout or not, so the repository is made here.
    let root = outside_git("in-repository");
    std::fs::create_dir(root.join(".git")).unwrap();
    let deep = root.join("a").join("b").join("pkg");
    std::fs::create_dir_all(deep.join("src")).unwrap();
    std::fs::write(deep.join("ita.una"), published("acme")).unwrap();
    std::fs::write(deep.join("src/demo.una"), MODULE).unwrap();
    assert!(refused(&deep), "a `.git` several directories above");
    let dir = package_outside_git("git-dir", &published("acme"));
    assert!(!refused(&dir));
    std::fs::create_dir(dir.join(".git")).unwrap();
    assert!(refused(&dir), "a `.git` directory");
    let dir = package_outside_git("git-file", &published("acme"));
    std::fs::write(dir.join(".git"), "gitdir: ../elsewhere\n").unwrap();
    assert!(refused(&dir), "a `.git` file");
    let dir = package_outside_git("git-above", &published("acme"));
    std::fs::create_dir(dir.parent().unwrap().join(".git")).unwrap();
    assert!(refused(&dir), "a `.git` in an ancestor");
}

/// A submission goes to the source the head names, at the URL it is declared with; an https
/// source has no transport yet, and nothing is sent.
#[test]
fn publish_and_yank_send_only_where_the_source_is_declared() {
    let store = scratch("ita-registry-store", "sources");
    let codes_and = |args: &[&str], part: &str| {
        let (status, out, _) = una_in(args, &store);
        assert_eq!(status, 1, "{out}");
        assert_eq!(codes_of(&out), ["U7013"], "{out}");
        assert!(out.contains(part), "{part}: {out}");
    };
    // `una check` of the package comes before the source is looked up (`spec/30 §15` rule 3),
    // and it needs the lock: each package is locked first, as a publisher does.
    let locked = |dir: &Path| {
        let (status, out, err) = una_in(&["una", "ita", "lock", path(dir)], &store);
        assert_eq!(status, 0, "{out}{err}");
    };
    let itaty = package_outside_git("itaty", &published("itaty"));
    locked(&itaty);
    codes_and(
        &["una", "--json", "ita", "publish", path(&itaty)],
        "no https transport yet",
    );
    codes_and(
        &["una", "--json", "ita", "yank", "0.1.0", path(&itaty)],
        "no https transport yet",
    );
    let lone = package_outside_git("lone", &published("acme"));
    locked(&lone);
    codes_and(
        &["una", "--json", "ita", "publish", path(&lone)],
        "declared by no",
    );
    let member = package_outside_git("member", &published("acme"));
    std::fs::write(
        member.parent().unwrap().join("itaty.una"),
        format!(
            "itaty\n  members\n    \"pkg\"\n\nsources\n  acme \"https://ita.acme.example\"\n    root \"sha256:{}\"\n    owners acme\n",
            "5e".repeat(32)
        ),
    )
    .unwrap();
    locked(&member);
    codes_and(
        &["una", "--json", "ita", "publish", path(&member)],
        "no https transport yet",
    );
    // A source the package's workspace does not declare is refused, and nothing is sent, the
    // name being well formed: `publish to` is checked against `itaty.una` when it is published.
    std::fs::write(member.join("ita.una"), published("other")).unwrap();
    codes_and(
        &["una", "--json", "ita", "publish", path(&member)],
        "`other` is not a source of",
    );
    codes_and(
        &["una", "--json", "ita", "yank", "0.1.0", path(&member)],
        "`other` is not a source of",
    );
    std::fs::write(member.join("ita.una"), published("acme")).unwrap();
    let (status, _, err) = una_in(&["una", "ita", "yank", "0.1", path(&member)], &store);
    assert_eq!(status, 2);
    assert!(err.contains("full version"), "{err}");
}

/// `una ita yank` reads an `ita.una` with `\r\n` line ends as the one with `\n`, and decodes
/// it once: a lone `\r` just before a `\r\n` is reported (U7008), at the `\r`, and nothing is
/// sent (#2252).
#[test]
fn yank_reads_crlf_line_ends_and_reports_a_lone_carriage_return() {
    let store = scratch("ita-registry-store", "yank-crlf");
    let dir = package_outside_git("yank-crlf", &published("itaty").replace('\n', "\r\n"));
    let (status, out, err) = una_in(&["una", "ita", "lock", path(&dir)], &store);
    assert_eq!(status, 0, "{out}{err}");
    let (status, out, _) = una_in(
        &["una", "--json", "ita", "yank", "0.1.0", path(&dir)],
        &store,
    );
    assert_eq!(status, 1, "{out}");
    assert_eq!(codes_of(&out), ["U7013"], "{out}");
    assert!(out.contains("no https transport yet"), "{out}");
    let lone = published("itaty").replacen('\n', "\r\r\n", 1);
    std::fs::write(dir.join("ita.una"), &lone).unwrap();
    let (status, out, _) = una_in(
        &["una", "--json", "ita", "yank", "0.1.0", path(&dir)],
        &store,
    );
    assert_eq!(status, 1, "{out}");
    assert_eq!(codes_of(&out), ["U7008"], "{out}");
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(
        diagnostic["message"],
        "carriage return not followed by a line feed"
    );
    assert_eq!(
        diagnostic["primary"]["start"],
        serde_json::json!({"line": 1, "col": 14}),
        "{out}"
    );
    assert_eq!(
        diagnostic["primary"]["end"],
        serde_json::json!({"line": 1, "col": 15}),
        "{out}"
    );
}

/// `una ita add` on an `ita.una` with `\r\n` line ends writes the same file as on the one
/// with `\n` alone: the text decoded once, `\n` line ends, and the new need (#2252).
#[test]
fn add_edits_an_ita_una_with_crlf_line_ends() {
    let (registry, _, _) = published_registry("add-crlf");
    let mut written = Vec::new();
    for (name, line_end) in [("add-lf", "\n"), ("add-crlf", "\r\n")] {
        let (dir, _, itaoca) = project(name, &registry);
        let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
        let (head, _) = manifest.split_once("\nneeds\n").unwrap();
        std::fs::write(
            dir.join("ita.una"),
            format!("{head}\n").replace('\n', line_end),
        )
        .unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_una"))
            .current_dir(&dir)
            .args(["ita", "add", "acme/ledger", "0.3"])
            .env("UNA_ITAOCA", &itaoca)
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}: {output:?}");
        written.push(std::fs::read_to_string(dir.join("ita.una")).unwrap());
    }
    assert!(!written[1].contains('\r'), "{:?}", written[1]);
    assert!(
        written[1].contains("\nneeds\n  acme/ledger 0.3"),
        "{}",
        written[1]
    );
    assert_eq!(written[1], written[0]);
}

// --- end to end: publish, lock and fetch through mirrors.una, yank, audit ------------------

/// A release published into a directory registry is locked and fetched through `mirrors.una`;
/// then its owner yanks it, and `una ita audit` reports U7014.
#[test]
fn a_published_release_is_locked_fetched_yanked_and_audited() {
    let (mut registry, signing, inbox) = published_registry("end-to-end");
    let (dir, mirror, itaoca) = project("end-to-end", &registry);
    let (status, out, err) = una_in(&["una", "ita", "lock", path(&dir)], &itaoca);
    assert_eq!(status, 0, "{out}{err}");
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    assert!(lock.contains("package acme/ledger 0.3.1"), "{lock}");
    let (status, out, err) = una_in(&["una", "ita", "fetch", path(&dir)], &itaoca);
    assert_eq!(status, 0, "{out}{err}");
    let (status, out, err) = una_in(&["una", "ita", "audit", path(&dir)], &itaoca);
    assert_eq!(status, 0, "{out}{err}");
    assert!(out.contains("audited 1 registry packages"), "{out}");
    // The owner yanks it: the yank is published into the directory and logged.
    let mut reader = client(&registry);
    let state = NameState::read(&mut reader, "acme/ledger").unwrap();
    drop(reader);
    let yank = submit::yank(
        &state,
        &Version::parse_full("0.3.1").unwrap(),
        &signers(&signing),
    )
    .unwrap();
    submit::send(&Directory::new(&inbox), &yank).unwrap();
    log_submission(&mut registry, &inbox, &yank, "acme/ledger");
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (status, out, _) = una_in(&["una", "--json", "ita", "audit", path(&dir)], &itaoca);
    assert_eq!(status, 1, "{out}");
    assert_eq!(codes_of(&out), ["U7014"], "{out}");
    assert!(out.contains("0.3.1 is yanked"), "{out}");
    // A source no place can serve is not called clean: unavailable (U7013), since what is
    // missing never makes a verdict.
    std::fs::remove_file(itaoca.join("mirrors.una")).unwrap();
    let (status, out, _) = una_in(&["una", "--json", "ita", "audit", path(&dir)], &itaoca);
    assert_eq!(status, 1);
    assert_eq!(codes_of(&out), ["U7013"], "{out}");
    assert!(out.contains("audit reports nothing as clean"), "{out}");
}

/// The registry then logs an owner-chain entry it must not log, a revocation that leaves the
/// owner below the key floor (two keys at threshold 2), though the owner's threshold signs it:
/// the audit refuses the package (U7010) and reports, apart, the evidence against the registry
/// at the lock's `registry` section (U7010), naming that entry's log index, read from the
/// client's typed record.
#[test]
fn audit_reports_an_unacceptable_owner_chain_entry_as_evidence_against_the_registry() {
    let (mut registry, _, _) = published_registry("unacceptable");
    let (dir, mirror, itaoca) = project("unacceptable", &registry);
    let (status, out, err) = una_in(&["una", "ita", "lock", path(&dir)], &itaoca);
    assert_eq!(status, 0, "{out}{err}");
    let laptop = keypair(ACME_KEYS[0].0, ACME_KEYS[0].1);
    let desk = keypair(ACME_KEYS[1].0, ACME_KEYS[1].1);
    let below = registry.owner_event("acme", "revoke acme.spare", 2, &[&laptop, &desk]);
    registry.checkpoint(now() - 30);
    registry.write(&mirror);
    let (status, out, _) = una_in(&["una", "--json", "ita", "audit", path(&dir)], &itaoca);
    assert_eq!(status, 1, "{out}");
    assert_eq!(codes_of(&out), ["U7010", "U7010"], "{out}");
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    let evidence: Vec<&serde_json::Value> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| {
            d["message"]
                .as_str()
                .unwrap()
                .contains("evidence against the registry `acme`")
        })
        .collect();
    assert_eq!(evidence.len(), 1, "{out}");
    let message = evidence[0]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("it logged the entry {below} of the owner `acme`")),
        "{message}"
    );
}

/// A document that no place serves leaves the audit's state unavailable (U7013), told apart
/// from a copy that does not verify (U7010) by the client's record of omissions, never by a
/// message: here every map tile is gone from the only place that has a transport.
#[test]
fn audit_reports_an_omission_as_unavailable() {
    let (registry, _, _) = published_registry("omission");
    let (dir, mirror, itaoca) = project("omission", &registry);
    let (status, out, err) = una_in(&["una", "ita", "lock", path(&dir)], &itaoca);
    assert_eq!(status, 0, "{out}{err}");
    std::fs::remove_dir_all(mirror.join("map")).unwrap();
    let (status, out, _) = una_in(&["una", "--json", "ita", "audit", path(&dir)], &itaoca);
    assert_eq!(status, 1, "{out}");
    assert_eq!(codes_of(&out), ["U7013"], "{out}");
}

#[test]
fn audit_of_a_lock_with_no_registry_package_is_clean() {
    let store = scratch("ita-registry-store", "audit-plain");
    let plain = package("audit-plain", MANIFEST);
    una_in(&["una", "ita", "lock", path(&plain)], &store);
    let (status, out, _) = una_in(&["una", "ita", "audit", path(&plain)], &store);
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("no registry package is locked"), "{out}");
}

#[test]
fn watch_reads_the_source_it_is_given() {
    let store = scratch("ita-registry-store", "watch");
    let (status, _, _) = una_in(&["una", "ita", "watch", "Acme"], &store);
    assert_eq!(status, 2);
    // The source must be declared where the command runs: here, nowhere.
    let (status, _, err) = una_in(&["una", "ita", "watch", "acme", "--from", "acme"], &store);
    assert_eq!(status, 2, "{err}");
}

// --- bundle and unbundle (rule 33) -----------------------------------------------------------

/// A locked, fetched package: its registry, the package, the mirror and Itaoca.
fn fetched(name: &str) -> (TestRegistry, PathBuf, PathBuf, PathBuf) {
    let (registry, _, _) = published_registry(name);
    let (dir, mirror, itaoca) = project(name, &registry);
    let (status, out, err) = una_in(&["una", "ita", "lock", path(&dir)], &itaoca);
    assert_eq!(status, 0, "{out}{err}");
    (registry, dir, mirror, itaoca)
}

#[test]
fn a_bundle_carries_the_lock_to_an_air_gapped_itaoca() {
    let (registry, dir, _, itaoca) = fetched("bundle");
    let file = scratch("ita-registry-bundles", "one").join("demo.bundle");
    let (status, out, err) = una_in(&["una", "ita", "bundle", path(&file), path(&dir)], &itaoca);
    assert_eq!(status, 0, "{out}{err}");
    assert!(out.contains("bundled 1 registry packages"), "{out}");
    // On another machine: a fresh Itaoca, with no mirror, filled from the bundle alone.
    let isolated = scratch("ita-registry-store", "bundle-isolated");
    let (status, out, err) = una_in(
        &["una", "ita", "unbundle", path(&file), path(&dir)],
        &isolated,
    );
    assert_eq!(status, 0, "{out}{err}");
    assert!(out.contains("stored acme/ledger 0.3.1 in Itaoca"), "{out}");
    // The floor is kept under the registry's identity, the hex SHA-256 of its pin
    // (`spec/30 §14` rule 3), never under the source's name.
    let id = una_project::itaoca::registry_id(&registry.pin());
    assert!(
        isolated
            .join("registry")
            .join(&id)
            .join("checkpoint")
            .exists(),
        "the floor is stored"
    );
    // Every copy matches the lock, and the package is still not verified until the registry
    // client supplies its key chain and name map proof: unavailable (`spec/30 §10` rule 11).
    let (status, out, _) = una_in(&["una", "--json", "ita", "verify", path(&dir)], &isolated);
    assert_eq!(status, 1, "{out}");
    assert_eq!(codes_of(&out), ["U7013"], "{out}");
    assert!(out.contains("match the lock"), "{out}");
    // Again: Itaoca holds it already.
    let (status, out, _) = una_in(
        &["una", "ita", "unbundle", path(&file), path(&dir)],
        &isolated,
    );
    assert_eq!(status, 0, "{out}");
}

/// A tampered bundle is refused (U7010) and leaves Itaoca as it was.
#[test]
fn a_tampered_bundle_is_refused() {
    let (_, dir, _, itaoca) = fetched("bundle-tampered");
    let file = scratch("ita-registry-bundles", "tampered").join("demo.bundle");
    let (status, _, _) = una_in(&["una", "ita", "bundle", path(&file), path(&dir)], &itaoca);
    assert_eq!(status, 0);
    let mut bytes = std::fs::read(&file).unwrap();
    // A byte of the module's text, inside the archive inside the bundle.
    let at = bytes
        .windows(b"pub total".len())
        .position(|w| w == b"pub total")
        .unwrap();
    bytes[at] = b'P';
    std::fs::write(&file, &bytes).unwrap();
    let isolated = scratch("ita-registry-store", "bundle-tampered-isolated");
    let (status, out, _) = una_in(
        &["una", "--json", "ita", "unbundle", path(&file), path(&dir)],
        &isolated,
    );
    assert_eq!(status, 1);
    assert_eq!(codes_of(&out), ["U7010"]);
    assert!(!isolated.join("packages").exists());
    assert!(!isolated.join("registry").exists());
}

/// ADR 0122, amendment of 2026-11-09 (L3): unbundling into an Itaoca whose root-version floor
/// holds the bundle's root version with another digest refuses the bundle as a fork of the root
/// chain (U7010) and writes no package; with the digest of the bundle's root it unbundles.
#[test]
fn unbundle_refuses_a_fork_at_the_root_version_floor() {
    let (registry, dir, _, itaoca) = fetched("bundle-root-fork");
    let file = scratch("ita-registry-bundles", "root-fork").join("demo.bundle");
    let (status, _, _) = una_in(&["una", "ita", "bundle", path(&file), path(&dir)], &itaoca);
    assert_eq!(status, 0);
    let id = una_project::itaoca::registry_id(&registry.pin());
    let floor = |store: &Path, text: &str| {
        let at = store.join("registry").join(&id);
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(at.join("root-version"), text).unwrap();
    };
    let last = registry.roots.len();
    let digest = una_project::tlog::root::text_digest(registry.roots.last().unwrap()).unwrap();
    let isolated = scratch("ita-registry-store", "bundle-root-fork-isolated");
    floor(
        &isolated,
        &format!("{last} text sha256:{}\n", "0".repeat(64)),
    );
    let (status, out, err) = una_in(
        &["una", "--json", "ita", "unbundle", path(&file), path(&dir)],
        &isolated,
    );
    assert_eq!(status, 1, "{out}{err}");
    assert_eq!(codes_of(&out), ["U7010"], "{out}");
    assert!(out.contains("forked"), "{out}");
    assert!(!isolated.join("packages").exists());
    let same = scratch("ita-registry-store", "bundle-root-same-isolated");
    floor(&same, &format!("{last} text {digest}\n"));
    let (status, out, err) = una_in(&["una", "ita", "unbundle", path(&file), path(&dir)], &same);
    assert_eq!(status, 0, "{out}{err}");
}

/// Bundling reads the registry at the lock's resolution point: with no place to read it from,
/// nothing is written.
#[test]
fn bundle_needs_the_registry() {
    let (_, dir, _, itaoca) = fetched("bundle-no-place");
    std::fs::remove_file(itaoca.join("mirrors.una")).unwrap();
    let file = scratch("ita-registry-bundles", "no-place").join("demo.bundle");
    let (status, out, _) = una_in(
        &["una", "--json", "ita", "bundle", path(&file), path(&dir)],
        &itaoca,
    );
    assert_eq!(status, 1);
    assert_eq!(codes_of(&out), ["U7013"], "{out}");
    assert!(!file.exists());
}

// --- private sources (rule 42) ---------------------------------------------------------------

/// Runs the `una` binary in `dir`, as `una ita add` works on the package in the current
/// directory: the exit code, stdout and stderr.
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

/// `una ita add` of an owner that a source of the root serves names that source itself, and
/// refuses another: a private owner's package never resolves at another registry.
#[test]
fn add_binds_an_owner_to_the_source_that_serves_it() {
    let manifest = format!(
        "{MANIFEST}\nsources\n  acme \"https://ita.acme.example\"\n    root \"sha256:{}\"\n    owners acme\n",
        "5e".repeat(32)
    );
    let dir = package("add-owner", &manifest);
    let (code, out, _) = una_at(&dir, &["--json", "ita", "add", "acme/ledger", "0.3"]);
    assert_eq!(code, 1);
    assert!(out.contains("`acme`"), "{out}");
    let (code, _, err) = una_at(
        &dir,
        &["ita", "add", "acme/ledger", "0.3", "--from", "itaty"],
    );
    assert_eq!(code, 2);
    assert!(err.contains("served by the source `acme`"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.join("ita.una")).unwrap(),
        manifest
    );
}

// --- review fixes: floors, symlinks and the environment --------------------------------------

/// Runs `una` with `variables` as the whole environment.
fn una_with(args: &[&str], variables: &[(&str, &str)]) -> (u8, String, String) {
    let variables: Vec<(String, String)> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = una_cli::run(
        args.iter().copied(),
        &mut stdout,
        &mut stderr,
        false,
        &|name| {
            Ok(variables
                .iter()
                .find(|(set, _)| set == name)
                .map(|(_, value)| value.clone()))
        },
        una_cli::Heap::unmeasured(),
    );
    (
        status.code(),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

/// The audit proves that the fresh checkpoint extends the lock's resolution point, with no floor
/// stored: an empty Itaoca that holds only `mirrors.una`.
#[test]
fn audit_proves_the_lock_point_with_an_empty_itaoca() {
    let (_, dir, _, itaoca) = fetched("audit-lock-point");
    let empty = scratch("ita-registry-store", "audit-lock-point-empty");
    std::fs::copy(itaoca.join("mirrors.una"), empty.join("mirrors.una")).unwrap();
    let (status, out, err) = una_in(&["una", "ita", "audit", path(&dir)], &empty);
    assert_eq!(status, 0, "{out}{err}");
    // A lock whose resolution point the registry's log does not extend: not audited as clean.
    let lock = std::fs::read_to_string(dir.join("ita.lock")).unwrap();
    let at = lock.find("  checkpoint ").unwrap();
    let start = at + lock[at..].find("\"sha256:").unwrap() + 1;
    let forked = format!(
        "{}sha256:{}{}",
        &lock[..start],
        "00".repeat(32),
        &lock[start + 71..]
    );
    std::fs::write(dir.join("ita.lock"), forked).unwrap();
    let empty = scratch("ita-registry-store", "audit-lock-point-empty-2");
    std::fs::copy(itaoca.join("mirrors.una"), empty.join("mirrors.una")).unwrap();
    let (status, out, _) = una_in(&["una", "--json", "ita", "audit", path(&dir)], &empty);
    assert_eq!(status, 1, "{out}");
    assert_eq!(codes_of(&out), ["U7010"], "{out}");
}

/// A floor that cannot be stored is a note, never a failure (rule 39): with the registry's
/// floor directory read-only, the audit still verifies and succeeds, and says so.
#[cfg(unix)]
#[test]
fn a_floor_that_cannot_be_stored_is_a_note() {
    use std::os::unix::fs::PermissionsExt;
    let (registry, dir, _, itaoca) = fetched("floor-unstored");
    let floors = itaoca
        .join("registry")
        .join(una_project::itaoca::registry_id(&registry.pin()));
    std::fs::create_dir_all(&floors).unwrap();
    // Neither the lock file nor the directory can be written: no floor can be stored.
    let lock = floors.join("floor.lock");
    if lock.exists() {
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o444)).unwrap();
    }
    std::fs::set_permissions(&floors, std::fs::Permissions::from_mode(0o555)).unwrap();
    // A process that ignores file modes, as root does, can write here anyway: the test cannot
    // block the store this way, and says so.
    let probe = floors.join("probe");
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok()
    {
        let _ = std::fs::remove_file(&probe);
        std::fs::set_permissions(&floors, std::fs::Permissions::from_mode(0o755)).unwrap();
        println!("skipped: running with privileges that ignore file modes");
        return;
    }
    let (status, out, err) = una_in(&["una", "ita", "audit", path(&dir)], &itaoca);
    std::fs::set_permissions(&floors, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(status, 0, "{out}{err}");
    assert!(out.contains("note: "), "{out}");
    assert!(out.contains("audited 1 registry packages"), "{out}");
}

/// A bundle reached through a symbolic link is refused, and nothing is written.
#[cfg(unix)]
#[test]
fn unbundle_follows_no_link() {
    let (_, dir, _, itaoca) = fetched("bundle-link");
    let base = scratch("ita-registry-bundles", "link");
    let file = base.join("demo.bundle");
    let (status, _, _) = una_in(&["una", "ita", "bundle", path(&file), path(&dir)], &itaoca);
    assert_eq!(status, 0);
    let link = base.join("link.bundle");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let isolated = scratch("ita-registry-store", "bundle-link-isolated");
    let (status, _, err) = una_in(
        &["una", "ita", "unbundle", path(&link), path(&dir)],
        &isolated,
    );
    assert_eq!(status, 2, "{err}");
    assert!(!isolated.join("packages").exists());
}

/// `GIT_DIR` or `GIT_WORK_TREE` set can put any directory in a repository: publish refuses.
#[test]
fn publish_refuses_with_git_variables_set() {
    let dir = package_outside_git("git-variables", &published("acme"));
    for variable in ["GIT_DIR", "GIT_WORK_TREE"] {
        let (status, out, _) = una_with(
            &["una", "--json", "ita", "publish", path(&dir)],
            &[(variable, "/elsewhere")],
        );
        assert_eq!(status, 1, "{variable}");
        assert!(out.contains(&format!("`{variable}` is set")), "{out}");
    }
}

/// A symbolic link to a large file is refused as a link, and the large file itself by its size
/// on the opened descriptor, before any of it is read.
#[cfg(unix)]
#[test]
fn unbundle_refuses_a_link_to_a_large_file() {
    let (_, dir, _, _) = fetched("bundle-large");
    let base = scratch("ita-registry-bundles", "large");
    let large = base.join("large.bundle");
    // Sparse: no disk is written for its length.
    std::fs::File::create(&large)
        .unwrap()
        .set_len(una_project::registry::bundle::MAX_BYTES + 1)
        .unwrap();
    let link = base.join("link.bundle");
    std::os::unix::fs::symlink(&large, &link).unwrap();
    let isolated = scratch("ita-registry-store", "bundle-large-isolated");
    for file in [&link, &large] {
        let (status, _, err) = una_in(
            &["una", "ita", "unbundle", path(file), path(&dir)],
            &isolated,
        );
        assert_eq!(status, 2, "{err}");
    }
    assert!(!isolated.join("packages").exists());
    let _ = std::fs::remove_file(&large);
}
