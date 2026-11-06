//! Tests that do not depend on the directories above the checkout or above their temporary
//! directories (#1560).
//!
//! A single file named to `una check`, `una run`, `una test` or `una fmt` takes the edition of
//! the nearest `ita.una` or `una.toml` in a directory above it, however far up (`spec/30`,
//! `spec/13 §7`, #1175). A stray manifest above the checkout, above `CARGO_TARGET_DIR` or above
//! the system's temporary directory therefore changes the edition of every single-file fixture
//! below it: an `ita.una` of edition 0.4 above a private build made 167 conformance cases,
//! `explain_examples` and the verifier's corpus cross-check fail. The tests cannot rely on what
//! lies above them, so every single-file fixture lies below a seal: a `una.toml` of edition 0.1,
//! the edition a single file has with no manifest above it (ADR 0021), which the upward search
//! finds before anything further up. The product's search is unchanged.
//!
//! [`seal`] marks a temporary directory that a test writes its single files under. [`checkout`]
//! is a sealed copy of the repository's trees that tests read in place, such as
//! `tests/conformance`: the checkout itself cannot hold a seal, since a `una.toml` there would
//! also be the package root that `una lsp` finds for every single file of the repository.

// Each test file uses its own subset of these helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The seal's manifest: edition 0.1, the edition of a single file with no manifest above it.
pub const SEAL: &str = "# A seal for tests (#1560): the single files below it are of edition 0.1, whatever\n# lies above this directory.\n[package]\nname = \"seal\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

/// The repository root.
pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Creates `dir` and seals it, so that no manifest above it reaches the single files below it.
/// Idempotent and safe for tests running in parallel: the seal is written whole under another
/// name and renamed into place. A test never names the sealed directory itself as a package.
pub fn seal(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let manifest = dir.join("una.toml");
    if std::fs::read_to_string(&manifest).is_ok_and(|text| text == SEAL) {
        return;
    }
    let staged = dir.join(format!(
        ".una.toml.{}.{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&staged, SEAL).unwrap();
    std::fs::rename(&staged, &manifest).unwrap();
}

/// A sealed copy of the repository's `trees` (paths relative to the repository root, each a
/// directory or a file), at the same relative paths under the returned root: the files that a
/// test would read in place in the checkout, read where no manifest above the checkout reaches
/// them. The root itself holds no manifest; its parent is sealed. Made once per process of a
/// test binary, which passes the same `trees` on every call, under
/// `CARGO_TARGET_TMPDIR/hermetic/<test crate>-<process id>`, so two runs of one binary at once
/// never share or remove each other's copy; the copies of earlier runs, a day old or more, are
/// removed. O(bytes of the trees); `std::fs::copy` clones files on APFS.
pub fn checkout(trees: &[&str]) -> PathBuf {
    static ROOT: OnceLock<(Vec<String>, PathBuf)> = OnceLock::new();
    let (made, root) = ROOT.get_or_init(|| {
        let parent = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hermetic");
        remove_stale(&parent);
        let base = parent.join(format!(
            "{}-{}",
            env!("CARGO_CRATE_NAME"),
            std::process::id()
        ));
        let root = checkout_under(&base, trees);
        (trees.iter().map(|tree| (*tree).to_owned()).collect(), root)
    });
    assert_eq!(made, trees, "one set of trees per test binary");
    root.clone()
}

/// A sealed copy of `trees` for one test of a test that runs one test per case (#1652), at the
/// same relative paths under its [`CaseCheckout::root`]: [`checkout`] for a single case, where a
/// copy of the whole corpus per test would cost more than the test. Under
/// `CARGO_TARGET_TMPDIR/hermetic/<test crate>-<process id>-<n>`, `n` counting the calls of the
/// process, so tests running at once, in one process or in many, never share a copy; removed
/// when the value is dropped, and by a later call once a day old if its test was killed.
/// O(bytes of the trees).
pub fn checkout_case(trees: &[&str]) -> CaseCheckout {
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let parent = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hermetic");
    // The copies of a killed test, a day old or more.
    remove_stale(&parent);
    let base = parent.join(format!(
        "{}-{}-{n}",
        env!("CARGO_CRATE_NAME"),
        std::process::id()
    ));
    let root = checkout_under(&base, trees);
    CaseCheckout { base, root }
}

/// A copy made by [`checkout_case`], removed when dropped.
pub struct CaseCheckout {
    base: PathBuf,
    /// The root of the copy, which holds the trees at their paths relative to the repository.
    pub root: PathBuf,
}

impl Drop for CaseCheckout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// [`checkout`] made afresh under `base`, which it seals: the root is `base/repository`.
pub fn checkout_under(base: &Path, trees: &[&str]) -> PathBuf {
    let root = base.join("repository");
    let _ = std::fs::remove_dir_all(&root);
    seal(base);
    let repository = repository();
    for tree in trees {
        copy(&repository.join(tree), &root.join(tree));
    }
    root
}

/// Removes the copies of [`checkout`] under `parent` that are a day old or more: no run of a
/// test binary lasts that long, so none of them is in use.
fn remove_stale(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= day);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Copies `from`, a file, a directory or a link, to `to`, links as links.
fn copy(from: &Path, to: &Path) {
    let kind = std::fs::symlink_metadata(from).unwrap().file_type();
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    if kind.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from).unwrap(), to).unwrap();
    } else if kind.is_dir() {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            copy(&entry.path(), &to.join(entry.file_name()));
        }
    } else {
        std::fs::copy(from, to).unwrap();
    }
}

/// The nearest `ita.una` or `una.toml` strictly above `path`, as the product's upward search
/// finds it, if any.
pub fn manifest_above(path: &Path) -> Option<PathBuf> {
    let absolute = path.canonicalize().ok()?;
    absolute.ancestors().skip(1).find_map(|dir| {
        ["ita.una", "una.toml"]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|manifest| manifest.is_file())
    })
}
