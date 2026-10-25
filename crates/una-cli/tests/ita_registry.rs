//! `una ita publish`, `yank`, `audit`, `watch`, `bundle` and `unbundle` (ADR 0122 rules 33, 36,
//! 36.2 and 40 to 43, `spec/30 §14`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;

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

/// A test registry `acme` with one release, `acme/ledger 0.3.1`, signed by its own root,
/// log, witnesses and publisher (ADR 0122 rules 35 to 38).
mod registry {
    use ed25519_dalek::{Signer, SigningKey};
    use una_project::content::{File, archive, digest_of};
    use una_project::provenance::{Provenance, sha256_digest, signed_name};
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
        let checkpoint_text = format!("{LOG}\n2\n{}\n", encode_base64(&top));
        let checkpoint = format!(
            "{checkpoint_text}\n{}{}",
            sign_line(&log_pair, &log, &checkpoint_text),
            cosign_line(&w1, &wa, &checkpoint_text, 1_900_000_000)
        );
        let root = una_project::tlog::root::digest(&root_document);
        let provenance = Provenance {
            roots: vec![root_document],
            checkpoint,
            statement,
            index: 1,
            proof: inclusion_proof(1, &leaves),
            key: publisher.clone(),
        };
        let lock = format!(
            "registry acme\n  root \"{root}\"\n  checkpoint 2 \"sha256:{}\"\n\npackage acme/ledger 0.3.1\n  from acme\n  digest \"{digest}\"\n  signed \"{}\"\n  logged 1\n",
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
    let itaoca = scratch("ita-registry-store", name);
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

// --- publish and yank ------------------------------------------------------------------------

fn published(extra: &str) -> String {
    format!(
        "ita acme/demo\n  version 0.1.0\n  edition 0.4\n  about \"A demo\"\n  license \"Apache-2.0\"\n  publish to acme\n{extra}"
    )
}

/// A key file of the owner, readable by its owner only.
fn key_file(dir: &Path) -> PathBuf {
    let key = una_project::registry::keyfile::SigningKey::from_secret("acme.publisher", &[50; 32])
        .unwrap();
    let file = dir.join("publisher.key");
    std::fs::write(&file, key.encode()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    file
}

#[test]
fn publish_takes_only_a_directory_transport_and_never_plain_http() {
    let dir = package("publish-urls", &published(""));
    let keys = scratch("ita-registry-keys", "urls");
    let key = key_file(&keys);
    let store = scratch("ita-registry-store", "urls");
    let run = |upload: Option<&str>| {
        let mut args = vec!["una", "ita", "publish", path(&dir), "--key", path(&key)];
        if let Some(upload) = upload {
            args.extend(["--upload", upload]);
        }
        una_in(&args, &store)
    };
    let (status, _, err) = run(None);
    assert_eq!(status, 2);
    assert!(err.contains("--upload file:///"), "{err}");
    let (status, _, err) = run(Some("http://ita.acme.example"));
    assert_eq!(status, 2);
    assert!(err.contains("plain HTTP"), "{err}");
    let (status, _, err) = run(Some("https://ita.acme.example"));
    assert_eq!(status, 2);
    assert!(err.contains("no network transport"), "{err}");
    let (status, _, err) = run(Some("ftp://ita.acme.example"));
    assert_eq!(status, 2);
    assert!(err.contains("scheme `ftp`"), "{err}");
}

#[test]
fn publish_refuses_what_rule_43_names_and_sends_nothing() {
    let upload_dir = scratch("ita-registry-upload", "refusals");
    let upload = format!("file://{}", path(&upload_dir));
    let keys = scratch("ita-registry-keys", "refusals");
    let key = key_file(&keys);
    let store = scratch("ita-registry-store", "refusals");
    for (name, manifest, part) in [
        ("no-target", MANIFEST.to_owned(), "publish to"),
        (
            "path-need",
            published("\nneeds\n  util from path \"deps/util\"\n    accept name\n"),
            "path dependency",
        ),
    ] {
        let dir = package(&format!("publish-{name}"), &manifest);
        let (status, out, _) = una_in(
            &["una", "--json", "ita", "publish", path(&dir), "--key", path(&key), "--upload", &upload],
            &store,
        );
        assert_eq!(status, 1, "{name}");
        assert!(codes_of(&out).contains(&"U7013".to_owned()), "{name}: {out}");
        assert!(out.contains(part), "{name}: {out}");
    }
    // A package that does not check is refused too.
    let dir = package("publish-unchecked", &published(""));
    std::fs::write(dir.join("src/demo.una"), "module demo\n\npub one() -> Int = \"one\"\n").unwrap();
    let (status, out, _) = una_in(
        &["una", "--json", "ita", "publish", path(&dir), "--key", path(&key), "--upload", &upload],
        &store,
    );
    assert_eq!(status, 1);
    assert!(out.contains("checks at tier 1 with none"), "{out}");
    // A package that checks still needs its name's verified chain, which this release cannot
    // read: nothing is sent.
    let dir = package("publish-no-client", &published(""));
    let (status, out, _) = una_in(
        &["una", "--json", "ita", "publish", path(&dir), "--key", path(&key), "--upload", &upload],
        &store,
    );
    assert_eq!(status, 1);
    assert_eq!(codes_of(&out), ["U7013"]);
    assert!(out.contains("registry client"), "{out}");
    assert_eq!(std::fs::read_dir(&upload_dir).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn a_key_file_readable_by_others_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = package("publish-open-key", &published(""));
    let keys = scratch("ita-registry-keys", "open");
    let key = key_file(&keys);
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    let upload = format!("file://{}", path(&scratch("ita-registry-upload", "open")));
    let (status, _, err) = una_in(
        &["una", "ita", "publish", path(&dir), "--key", path(&key), "--upload", &upload],
        &keys,
    );
    assert_eq!(status, 2);
    assert!(err.contains("readable by its owner only"), "{err}");
}

#[test]
fn yank_needs_the_names_verified_chain() {
    let dir = package("yank", &published(""));
    let keys = scratch("ita-registry-keys", "yank");
    let key = key_file(&keys);
    let upload = format!("file://{}", path(&scratch("ita-registry-upload", "yank")));
    let (status, out, _) = una_in(
        &["una", "--json", "ita", "yank", "0.1.0", path(&dir), "--key", path(&key), "--upload", &upload],
        &keys,
    );
    assert_eq!(status, 1);
    assert_eq!(codes_of(&out), ["U7013"]);
    let (status, _, err) = una_in(
        &["una", "ita", "yank", "0.1", path(&dir), "--key", path(&key), "--upload", &upload],
        &keys,
    );
    assert_eq!(status, 2);
    assert!(err.contains("full version"), "{err}");
}

// --- audit and watch -------------------------------------------------------------------------

/// A verified client's states: `acme/ledger 0.3.1` yanked and named by an advisory.
struct Yanked;

impl una_project::registry::view::Registry for Yanked {
    fn name(
        &self,
        source: &str,
        name: &str,
        _held: u64,
    ) -> Result<una_project::registry::view::NameState, una_project::registry::view::Failure> {
        let mut state = una_project::registry::view::NameState {
            source: source.to_owned(),
            name: name.to_owned(),
            ..Default::default()
        };
        state.releases.insert(
            una_project::version::Version::parse_full("0.3.1").unwrap(),
            una_project::tlog::state::ReleaseState {
                digest: String::new(),
                logged_at: 1,
                yanked: true,
                advised: vec!["ITA-1".to_owned()],
                threshold_fix: false,
            },
        );
        Ok(state)
    }

    fn owner(
        &self,
        source: &str,
        owner: &str,
        _held: u64,
    ) -> Result<una_project::registry::view::OwnerState, una_project::registry::view::Failure>
    {
        Ok(una_project::registry::view::OwnerState {
            source: source.to_owned(),
            owner: owner.to_owned(),
            ..Default::default()
        })
    }

    fn log_key(
        &self,
        _source: &str,
    ) -> Result<una_project::tlog::note::Key, una_project::registry::view::Failure> {
        Err(una_project::registry::view::Failure::Unavailable(String::new()))
    }
}

#[test]
fn audit_reports_u7014_once_per_package_and_nothing_unverified_as_clean() {
    let (dir, store) = with_registry("audit", false);
    assert_eq!(una_cli::audit_codes(&dir, &Yanked), [una_diag::Code::U7014]);
    // With no verified client, every registry package is reported as not verified (U7010).
    let (status, out, _) = una_in(&["una", "--json", "ita", "audit", path(&dir)], &store);
    assert_eq!(status, 1);
    assert_eq!(codes_of(&out), ["U7010"]);
    // A lock with no registry package audits clean.
    let plain = package("audit-plain", MANIFEST);
    una_in(&["una", "ita", "lock", path(&plain)], &store);
    let (status, out, _) = una_in(&["una", "ita", "audit", path(&plain)], &store);
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("no registry package is locked"), "{out}");
}

#[test]
fn watch_needs_the_verified_client() {
    let store = scratch("ita-registry-store", "watch");
    let (status, _, err) = una_in(&["una", "ita", "watch", "acme", "--from", "acme"], &store);
    assert_eq!(status, 1);
    assert!(err.contains("registry client"), "{err}");
    let (status, _, _) = una_in(&["una", "ita", "watch", "Acme"], &store);
    assert_eq!(status, 2);
}

// --- bundle and unbundle (rule 33) -----------------------------------------------------------

#[test]
fn a_bundle_carries_the_lock_to_an_air_gapped_itaoca() {
    let (dir, store) = with_registry("bundle", true);
    let file = scratch("ita-registry-bundles", "one").join("demo.bundle");
    let (status, out, err) = una_in(&["una", "ita", "bundle", path(&file), path(&dir)], &store);
    assert_eq!(status, 0, "{out}{err}");
    assert!(out.contains("bundled 1 registry packages"), "{out}");
    // On another machine: a fresh Itaoca, filled from the bundle alone.
    let isolated = scratch("ita-registry-store", "bundle-isolated");
    let (status, out, err) = una_in(&["una", "ita", "unbundle", path(&file), path(&dir)], &isolated);
    assert_eq!(status, 0, "{out}{err}");
    assert!(out.contains("stored acme/ledger 0.3.1 in Itaoca"), "{out}");
    let (status, out, _) = una_in(&["una", "ita", "verify", path(&dir)], &isolated);
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("verified acme/ledger 0.3.1 in Itaoca"), "{out}");
    // Again: Itaoca holds it already.
    let (status, out, _) = una_in(&["una", "ita", "unbundle", path(&file), path(&dir)], &isolated);
    assert_eq!(status, 0, "{out}");
}

/// A tampered bundle is refused (U7010) and leaves Itaoca as it was.
#[test]
fn a_tampered_bundle_is_refused() {
    let (dir, store) = with_registry("bundle-tampered", true);
    let file = scratch("ita-registry-bundles", "tampered").join("demo.bundle");
    let (status, _, _) = una_in(&["una", "ita", "bundle", path(&file), path(&dir)], &store);
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
}

#[test]
fn bundle_reports_a_package_it_does_not_hold() {
    let (dir, store) = with_registry("bundle-absent", false);
    let file = scratch("ita-registry-bundles", "absent").join("demo.bundle");
    let (status, out, _) = una_in(
        &["una", "--json", "ita", "bundle", path(&file), path(&dir)],
        &store,
    );
    assert_eq!(status, 1);
    assert_eq!(codes_of(&out), ["U7011"]);
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
    assert!(out.contains("the per-name state of `acme`"), "{out}");
    let (code, _, err) = una_at(
        &dir,
        &["ita", "add", "acme/ledger", "0.3", "--from", "itaty"],
    );
    assert_eq!(code, 2);
    assert!(err.contains("served by the source `acme`"), "{err}");
    assert_eq!(std::fs::read_to_string(dir.join("ita.una")).unwrap(), manifest);
}
