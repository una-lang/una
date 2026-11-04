//! Registry packages on disk (ADR 0122 rules 17, 30 and 32): the verified load of a locked
//! registry package, from the vendor directory of the root or else from Itaoca, and
//! `una ita vendor`, which copies it into the vendor directory with its provenance.
//!
//! A registry package `owner/name` at `version` of `source` is vendored in
//! `vendor/ita/<source>/<owner>+<name>-<version>/` (an unscoped name has no `<owner>+`), with its
//! provenance beside it in `<owner>+<name>-<version>.provenance`. Both places are verified the
//! same way on every load: the files are read once each into memory as [`crate::tree`] states,
//! the digest of those bytes is the lock's ([`crate::tree::VerifiedTree`]), the provenance
//! verifies against the lock ([`crate::provenance`]), and the statement's `manifest` is the
//! digest of those same bytes of the package's `ita.una`. A vendored package that does not
//! verify refuses the load: Itaoca is not tried in its place, and nothing is repaired.
//! Vendoring writes the verified bytes of Itaoca's copy, never a file re-read by its path.
//!
//! Costs: O(B) for the B bytes of the package, and the provenance's O(log L) hashes.

use std::path::Path;

use una_syntax::ita;

use crate::ita::ITATY;
use crate::itaoca::Itaoca;
use crate::provenance::{self, Expected, Floor};
use crate::tree::{self, VerifiedTree};

/// The directory of the vendored packages, below the root.
pub const VENDOR: [&str; 2] = ["vendor", "ita"];

/// Why a registry package does not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// It is in neither the vendor directory nor Itaoca (U7011).
    Absent(String),
    /// It is there, and does not verify (U7010).
    Refused(String),
    /// It is there, and cannot be verified offline (U7013): its provenance's checkpoint carries
    /// a retired witness's cosignature with no proof to count it by, or the root-version floor
    /// this machine stores cannot be read (`spec/30 §9` rule 9).
    Unavailable(String),
}

impl Problem {
    /// Its sentence.
    pub fn message(&self) -> &str {
        match self {
            Problem::Absent(why) | Problem::Refused(why) | Problem::Unavailable(why) => why,
        }
    }
}

impl From<provenance::Problem> for Problem {
    fn from(problem: provenance::Problem) -> Problem {
        match problem {
            provenance::Problem::Refused(why) => Problem::Refused(why),
            provenance::Problem::Unavailable(why) => Problem::Unavailable(why),
        }
    }
}

/// Where a registry package was loaded from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The vendor directory of the root.
    Vendor,
    /// Itaoca.
    Itaoca,
}

/// A registry package, loaded and verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// Where it was loaded from.
    pub place: Place,
    /// Its files, as read once and verified.
    pub tree: VerifiedTree,
    /// Its provenance document, as stored.
    pub provenance: Vec<u8>,
}

/// A source's name as a directory: a lowercase identifier of at most 64 bytes.
pub fn check_source(source: &str) -> Result<(), String> {
    let valid = source.len() <= 64
        && source
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase())
        && source
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if valid {
        Ok(())
    } else {
        Err(format!("`{source}` is not a source's name"))
    }
}

/// The directory name of a release: `<owner>+<name>-<version>`, or `<name>-<version>` for a
/// name with no owner. Each part of the name is a lowercase identifier and the version a full
/// version, so the name holds `[a-z0-9_+.-]` only and never starts with `.`.
pub fn release_dir(name: &str, version: &str) -> Result<String, String> {
    let part = |text: &str| {
        !text.is_empty()
            && text.len() <= 64
            && text.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    let joined = match name.split_once('/') {
        Some((owner, short)) if part(owner) && part(short) => format!("{owner}+{short}"),
        None if part(name) => name.to_owned(),
        _ => return Err(format!("`{name}` is not a package's name")),
    };
    crate::version::Version::parse_full(version)?;
    let version_ok = version.len() <= 64
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if !version_ok {
        return Err(format!("`{version}` is not a full version"));
    }
    Ok(format!("{joined}-{version}"))
}

/// What the lock says of its registry package `package`, which its provenance must show: its
/// source, name, version, digest, key and log index, and its source's root and resolution
/// point. Refused (U7010) when the lock lacks one of them; for a source of `ita.una`'s
/// `sources` whose root is not the lock's; for a source that `ita.una` does not declare; and for
/// `itaty`, whose root is built into `una` and is not in this release.
pub fn expected(
    lock: &ita::Lock,
    manifest: Option<&ita::Manifest>,
    package: &ita::LockPackage,
) -> Result<Expected, String> {
    let name = package.name.value.to_string();
    let version = package.version.value.clone();
    let what = format!("`{name} {version}`");
    let Some(ita::At {
        value: ita::LockOrigin::Source(source),
        ..
    }) = &package.from
    else {
        return Err(format!("{what} is not from a registry"));
    };
    check_source(source)?;
    if source == ITATY {
        return Err(format!(
            "{what} is from `itaty`, whose root this release does not hold yet, so none of its packages verifies"
        ));
    }
    let registry = lock
        .registries
        .iter()
        .find(|r| r.source.value == *source)
        .ok_or_else(|| format!("{what} is from `{source}`, which the lock records no root for"))?;
    let declared = manifest
        .and_then(|m| m.sources.iter().find(|s| s.name.value == *source))
        .ok_or_else(|| {
            format!("{what} is from `{source}`, which ita.una's `sources` does not declare")
        })?;
    if declared.root.as_ref().map(|r| r.value.as_str()) != Some(registry.root.value.as_str()) {
        return Err(format!(
            "the lock's root of `{source}` is not the one ita.una's `sources` pins"
        ));
    }
    let digest = package
        .digest
        .as_ref()
        .ok_or_else(|| format!("{what} has no digest in the lock"))?;
    let (Some(signed), Some(logged)) = (&package.signed, &package.logged) else {
        return Err(format!(
            "{what} has no `signed` and `logged` lines in the lock, so its provenance cannot be verified"
        ));
    };
    Ok(Expected {
        source: source.clone(),
        name,
        version,
        digest: digest.value.clone(),
        signed: signed.value.clone(),
        logged: logged.value,
        root: registry.root.value.clone(),
        checkpoint_size: registry.checkpoint_size.value,
        checkpoint_hash: registry.checkpoint_hash.value.clone(),
        last_root: registry.last_root.value,
        last_root_digest: registry.last_root_digest.value.clone(),
    })
}

/// The release's files, verified: read once from `dir` with no link followed, with the digest
/// `digest`.
fn read_verified(dir: &Path, digest: &str) -> Result<VerifiedTree, Problem> {
    tree::read_verified(dir, digest).map_err(Problem::Refused)
}

/// Checks the provenance `bytes` against the lock, the root-version floor `floor` and the
/// release's files.
fn check(
    files: &VerifiedTree,
    bytes: &[u8],
    expected: &Expected,
    floor: &Floor,
) -> Result<(), Problem> {
    let read = provenance::parse(bytes).map_err(Problem::Refused)?;
    let statement = provenance::verify(&read, expected, floor)?;
    let manifest = files
        .file("ita.una")
        .ok_or_else(|| Problem::Refused(format!("`{}` has no ita.una", expected.name)))?;
    if statement.manifest != provenance::sha256_digest(manifest) {
        return Err(Problem::Refused(format!(
            "the ita.una of `{} {}` is not the one its release statement names",
            expected.name, expected.version
        )));
    }
    Ok(())
}

/// The vendored copy of the release, verified against the lock and the root-version floor
/// `floor` this machine stores ([`Floor::of`]), or `None` when the root has none.
pub fn vendored(
    root: &Path,
    expected: &Expected,
    floor: &Floor,
) -> Result<Option<Loaded>, Problem> {
    let dir_name = release_dir(&expected.name, &expected.version).map_err(Problem::Refused)?;
    let Some(dir) = tree::locate(root, &[VENDOR[0], VENDOR[1], &expected.source, &dir_name])
        .map_err(Problem::Refused)?
    else {
        return Ok(None);
    };
    let files = read_verified(&dir, &expected.digest)?;
    let provenance_name = format!("{dir_name}.provenance");
    let path = tree::locate(
        root,
        &[VENDOR[0], VENDOR[1], &expected.source, &provenance_name],
    )
    .map_err(Problem::Refused)?
    .ok_or_else(|| {
        Problem::Refused(format!(
            "the vendored `{} {}` has no {provenance_name} beside it",
            expected.name, expected.version
        ))
    })?;
    let bytes = tree::read_file(&path, provenance::MAX_BYTES as u64).map_err(Problem::Refused)?;
    check(&files, &bytes, expected, floor)?;
    Ok(Some(Loaded {
        place: Place::Vendor,
        tree: files,
        provenance: bytes,
    }))
}

/// The release from Itaoca, verified, or `None` when Itaoca lacks its files. Files held with no
/// provenance at the lock's resolution point are refused: the store is inconsistent (U7010),
/// which is not the absence of rule 17 (U7011). The provenance is verified against the
/// root-version floor Itaoca stores for the registry ([`Floor::of`]).
pub fn stored(itaoca: &Itaoca, expected: &Expected) -> Result<Option<Loaded>, Problem> {
    if !itaoca
        .contains(&expected.digest)
        .map_err(Problem::Refused)?
    {
        return Ok(None);
    }
    let files = itaoca.load(&expected.digest).map_err(Problem::Refused)?;
    let Some(bytes) = itaoca
        .load_provenance(
            &expected.source,
            &expected.name,
            &expected.version,
            expected.checkpoint_size,
        )
        .map_err(Problem::Refused)?
    else {
        return Err(Problem::Refused(format!(
            "Itaoca holds the files of `{} {}` but not its provenance at the checkpoint of size {}: the store is inconsistent; fetch it again",
            expected.name, expected.version, expected.checkpoint_size
        )));
    };
    check(
        &files,
        &bytes,
        expected,
        &Floor::of(Some(itaoca), &expected.root),
    )?;
    Ok(Some(Loaded {
        place: Place::Itaoca,
        tree: files,
        provenance: bytes,
    }))
}

/// Loads the release, verified: from the vendor directory of `root` when it holds it, else
/// from Itaoca (rule 32). A vendored copy that does not verify is refused, and Itaoca is not
/// tried in its place. Both are verified against the root-version floor Itaoca stores for the
/// registry, read at this load; with no Itaoca there is none (trust on first use).
pub fn load(root: &Path, itaoca: Option<&Itaoca>, expected: &Expected) -> Result<Loaded, Problem> {
    let floor = Floor::of(itaoca, &expected.root);
    if let Some(loaded) = vendored(root, expected, &floor)? {
        return Ok(loaded);
    }
    if let Some(itaoca) = itaoca
        && let Some(loaded) = stored(itaoca, expected)?
    {
        return Ok(loaded);
    }
    Err(Problem::Absent(format!(
        "the locked package `{} {}` is in neither the vendor directory nor Itaoca",
        expected.name, expected.version
    )))
}

/// Whether the registry package `package` of a lock is present, in the vendor directory of
/// `root` or in Itaoca, before anything of it is verified (rule 17's presence).
pub fn present(
    root: &Path,
    itaoca: Option<&Itaoca>,
    package: &ita::LockPackage,
) -> Result<bool, Problem> {
    let name = package.name.value.to_string();
    let version = &package.version.value;
    let Some(ita::At {
        value: ita::LockOrigin::Source(source),
        ..
    }) = &package.from
    else {
        return Err(Problem::Refused(format!(
            "`{name} {version}` is not from a registry"
        )));
    };
    check_source(source).map_err(Problem::Refused)?;
    let dir_name = release_dir(&name, version).map_err(Problem::Refused)?;
    let digest = package
        .digest
        .as_ref()
        .ok_or_else(|| Problem::Refused(format!("`{name} {version}` has no digest in the lock")))?;
    if tree::locate(root, &[VENDOR[0], VENDOR[1], source, &dir_name])
        .map_err(Problem::Refused)?
        .is_some()
    {
        return Ok(true);
    }
    match itaoca {
        Some(itaoca) => itaoca.contains(&digest.value).map_err(Problem::Refused),
        None => Ok(false),
    }
}

/// The problem of a locked package in neither place (U7011).
fn absent(package: &ita::LockPackage) -> Problem {
    Problem::Absent(format!(
        "the locked package `{} {}` is in neither the vendor directory nor Itaoca",
        package.name.value, package.version.value
    ))
}

/// Loads the registry package `package` of `lock`, verified (rule 17): first its presence, in
/// the vendor directory of `root` or in Itaoca (U7011 when it is in neither), then what the
/// lock says of it ([`expected`]) and the load of [`load`] (U7010 for any failure).
pub fn load_locked(
    root: &Path,
    itaoca: Option<&Itaoca>,
    lock: &ita::Lock,
    manifest: Option<&ita::Manifest>,
    package: &ita::LockPackage,
) -> Result<Loaded, Problem> {
    if !present(root, itaoca, package)? {
        return Err(absent(package));
    }
    let expected = expected(lock, manifest, package).map_err(Problem::Refused)?;
    load(root, itaoca, &expected)
}

/// `una ita vendor` for the registry package `package` of `lock`: its presence first, as
/// [`load_locked`] checks it, then [`vendor`]. Returns the paths written, relative to the root.
pub fn vendor_locked(
    root: &Path,
    itaoca: Option<&Itaoca>,
    lock: &ita::Lock,
    manifest: Option<&ita::Manifest>,
    package: &ita::LockPackage,
) -> Result<Vec<String>, Problem> {
    if !present(root, itaoca, package)? {
        return Err(absent(package));
    }
    let expected = expected(lock, manifest, package).map_err(Problem::Refused)?;
    vendor(root, itaoca, &expected)
}

/// `una ita vendor` for one release: copies it from Itaoca into the vendor directory of `root`
/// with its provenance, and verifies the copy as a load does. A copy already there that
/// verifies is kept; one whose files verify and whose provenance is missing or is not the
/// lock's gets Itaoca's; one whose files do not verify is refused and left as it is. Returns
/// the paths written, relative to the root.
pub fn vendor(
    root: &Path,
    itaoca: Option<&Itaoca>,
    expected: &Expected,
) -> Result<Vec<String>, Problem> {
    let dir_name = release_dir(&expected.name, &expected.version).map_err(Problem::Refused)?;
    let provenance_name = format!("{dir_name}.provenance");
    let relative = format!("vendor/ita/{}", expected.source);
    let present = tree::locate(root, &[VENDOR[0], VENDOR[1], &expected.source, &dir_name])
        .map_err(Problem::Refused)?;
    let floor = Floor::of(itaoca, &expected.root);
    let mut written = Vec::new();
    if let Some(dir) = &present {
        let files = read_verified(dir, &expected.digest)?;
        let current = tree::locate(
            root,
            &[VENDOR[0], VENDOR[1], &expected.source, &provenance_name],
        )
        .map_err(Problem::Refused)?;
        if let Some(path) = current {
            let bytes =
                tree::read_file(&path, provenance::MAX_BYTES as u64).map_err(Problem::Refused)?;
            if check(&files, &bytes, expected, &floor).is_ok() {
                return Ok(written);
            }
        }
    }
    let from = itaoca
        .map(|itaoca| stored(itaoca, expected))
        .transpose()?
        .flatten()
        .ok_or_else(|| {
            Problem::Absent(format!(
                "the locked package `{} {}` is not in Itaoca, so it cannot be vendored",
                expected.name, expected.version
            ))
        })?;
    let parent = tree::make_dirs(root, &[VENDOR[0], VENDOR[1], &expected.source])
        .map_err(Problem::Refused)?;
    if present.is_none() {
        tree::store_tree(&parent, &dir_name, &from.tree, false).map_err(|refusal| {
            Problem::Refused(format!(
                "cannot write the vendored copy into `{relative}/{dir_name}`: {refusal}"
            ))
        })?;
        written.push(format!("{relative}/{dir_name}"));
    }
    tree::store_file(&parent, &provenance_name, &from.provenance, false).map_err(|refusal| {
        Problem::Refused(format!(
            "cannot write `{relative}/{provenance_name}`: {refusal}"
        ))
    })?;
    written.push(format!("{relative}/{provenance_name}"));
    // The copy loads verified, as every load does.
    match vendored(root, expected, &floor)? {
        Some(_) => Ok(written),
        None => Err(Problem::Refused(format!(
            "the vendored copy of `{} {}` is missing after it was written",
            expected.name, expected.version
        ))),
    }
}
